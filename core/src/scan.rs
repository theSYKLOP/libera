//! Scanner parallèle : découpe le disque en sous-dossiers traités en parallèle (vol de travail rayon).
//! En mémoire : uniquement l'arbre des dossiers + les gros fichiers (pas un nœud par fichier),
//! ce qui garde l'empreinte faible même avec des millions de fichiers.

use crate::winfs::{self, Entry};
use rayon::prelude::*;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

pub const NO_PARENT: u32 = u32::MAX;

/// Compteurs partagés, lus par l'interface pendant l'analyse.
#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub denied: AtomicU64,
    pub cancel: AtomicBool,
    pub current: Mutex<String>,
    /// Quantité totale attendue pour l'étape en cours (même unité que `bytes`). 0 = inconnue → barre indéterminée.
    pub expected: AtomicU64,
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    /// Secondes Unix de la dernière modification.
    pub mtime: i64,
}

#[derive(Debug)]
pub struct DirNode {
    pub name: String,
    pub parent: u32,
    pub children: Vec<u32>,
    pub own_size: u64,
    pub own_files: u32,
    pub size: u64,
    pub files: u64,
    pub denied: bool,
}

pub struct ScanResult {
    pub root: String,
    pub nodes: Vec<DirNode>,
    pub big_files: Vec<FileEntry>,
    pub cloud_bytes: u64,
    pub denied_dirs: u64,
    pub elapsed_ms: u128,
}

struct Ctx<'a> {
    progress: &'a Progress,
    big_min: u64,
    big: Mutex<Vec<FileEntry>>,
    cloud: AtomicU64,
}

struct Built {
    name: String,
    own_size: u64,
    own_files: u32,
    denied: bool,
    children: Vec<Built>,
}

fn scan_dir(ctx: &Ctx, path_ext: &str, name: String) -> Built {
    let mut b = Built { name, own_size: 0, own_files: 0, denied: false, children: Vec::new() };
    if ctx.progress.cancelled() {
        return b;
    }
    let mut subdirs: Vec<(String, String)> = Vec::new();
    let mut local_big: Vec<FileEntry> = Vec::new();
    let mut local_bytes = 0u64;
    let mut local_files = 0u64;

    let res = winfs::read_dir(path_ext, |e: Entry| {
        if e.is_dir() {
            // Les liens (jonctions, liens symboliques) ne sont jamais suivis : évite boucles et doublons.
            if !e.is_reparse() {
                subdirs.push((winfs::join(path_ext, &e.name), e.name));
            }
        } else {
            let size = if e.is_cloud_only() {
                ctx.cloud.fetch_add(e.size, Ordering::Relaxed);
                0
            } else {
                e.size
            };
            b.own_size += size;
            b.own_files += 1;
            local_bytes += size;
            local_files += 1;
            if size >= ctx.big_min && !e.is_reparse() {
                local_big.push(FileEntry { path: winfs::join(path_ext, &e.name), size, mtime: e.mtime });
            }
        }
    });
    if res.is_err() {
        b.denied = true;
        ctx.progress.denied.fetch_add(1, Ordering::Relaxed);
    }
    let d = ctx.progress.dirs.fetch_add(1, Ordering::Relaxed);
    ctx.progress.files.fetch_add(local_files, Ordering::Relaxed);
    ctx.progress.bytes.fetch_add(local_bytes, Ordering::Relaxed);
    if d % 256 == 0 {
        if let Ok(mut c) = ctx.progress.current.try_lock() {
            *c = winfs::display_path(path_ext);
        }
    }
    if !local_big.is_empty() {
        ctx.big.lock().unwrap().extend(local_big);
    }
    b.children = subdirs.into_par_iter().map(|(p, n)| scan_dir(ctx, &p, n)).collect();
    b
}

fn flatten(b: Built, parent: u32, nodes: &mut Vec<DirNode>) -> u32 {
    let id = nodes.len() as u32;
    nodes.push(DirNode {
        name: b.name,
        parent,
        children: Vec::new(),
        own_size: b.own_size,
        own_files: b.own_files,
        size: b.own_size,
        files: b.own_files as u64,
        denied: b.denied,
    });
    let mut kids = Vec::with_capacity(b.children.len());
    for c in b.children {
        kids.push(flatten(c, id, nodes));
    }
    let (mut size, mut files) = (nodes[id as usize].size, nodes[id as usize].files);
    for &k in &kids {
        size += nodes[k as usize].size;
        files += nodes[k as usize].files;
    }
    kids.sort_by(|&a, &b| nodes[b as usize].size.cmp(&nodes[a as usize].size));
    let n = &mut nodes[id as usize];
    n.children = kids;
    n.size = size;
    n.files = files;
    id
}

/// Pool de threads dimensionné pour l'E/S disque (plus de threads que de cœurs : les threads attendent le SSD).
/// Créé une seule fois pour toute l'application (le recréer à chaque appel coûterait cher sur des milliers de fichiers).
pub fn io_pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        rayon::ThreadPoolBuilder::new()
            .num_threads((cores * 3).clamp(6, 24))
            .stack_size(32 * 1024 * 1024)
            .build()
            .expect("pool de threads")
    })
}

/// Analyse complète d'une racine (ex. `C:\`). `big_min` : taille minimale pour mémoriser un fichier individuellement.
pub fn scan(root: &str, big_min: u64, progress: &Progress) -> ScanResult {
    let t0 = std::time::Instant::now();
    let root_clean = root.trim_end_matches('\\').to_string();
    let root_display = if root_clean.len() == 2 && root_clean.ends_with(':') {
        format!("{}\\", root_clean)
    } else {
        root_clean.clone()
    };
    let ctx = Ctx { progress, big_min, big: Mutex::new(Vec::new()), cloud: AtomicU64::new(0) };
    let ext = winfs::extended(&root_clean);
    let pool = io_pool();
    let built = pool.install(|| scan_dir(&ctx, &ext, root_display.clone()));
    let mut nodes = Vec::new();
    flatten(built, NO_PARENT, &mut nodes);
    let mut big = ctx.big.into_inner().unwrap();
    for f in &mut big {
        f.path = winfs::display_path(&f.path);
    }
    big.sort_by(|a, b| b.size.cmp(&a.size));
    ScanResult {
        root: root_display,
        denied_dirs: progress.denied.load(Ordering::Relaxed),
        cloud_bytes: ctx.cloud.load(Ordering::Relaxed),
        nodes,
        big_files: big,
        elapsed_ms: t0.elapsed().as_millis(),
    }
}

/// Mesure rapide d'un dossier (sans arbre) : (octets, fichiers).
pub fn measure(path: &str) -> (u64, u64) {
    measure_with(path, &Progress::default())
}

/// Comme `measure`, mais alimente les compteurs partagés (affichés par l'interface) et respecte l'annulation.
pub fn measure_with(path: &str, progress: &Progress) -> (u64, u64) {
    let ctx = Ctx { progress, big_min: u64::MAX, big: Mutex::new(Vec::new()), cloud: AtomicU64::new(0) };
    let ext = winfs::extended(path.trim_end_matches('\\'));
    if !matches!(winfs::attrs(&ext), Ok(a) if a & 0x10 != 0) {
        return match winfs::file_size(&ext) {
            Some(s) => (s, 1),
            None => (0, 0),
        };
    }
    let b = scan_dir(&ctx, &ext, String::new());
    fn sum(b: &Built) -> (u64, u64) {
        let mut s = (b.own_size, b.own_files as u64);
        for c in &b.children {
            let r = sum(c);
            s.0 += r.0;
            s.1 += r.1;
        }
        s
    }
    sum(&b)
}

#[derive(Serialize, Clone)]
pub struct ChildInfo {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub size: u64,
    pub files: u64,
    pub pct: f32,
    pub denied: bool,
    pub has_children: bool,
}

impl ScanResult {
    pub fn path_of(&self, id: u32) -> String {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != NO_PARENT {
            let n = &self.nodes[cur as usize];
            parts.push(n.name.as_str());
            cur = n.parent;
        }
        parts.reverse();
        let mut out = String::new();
        for (i, p) in parts.iter().enumerate() {
            if i == 0 {
                out.push_str(p.trim_end_matches('\\'));
            } else {
                out.push('\\');
                out.push_str(p);
            }
        }
        if parts.len() == 1 {
            out.push('\\');
        }
        out
    }

    pub fn info(&self, id: u32, parent_size: u64) -> ChildInfo {
        let n = &self.nodes[id as usize];
        ChildInfo {
            id,
            name: n.name.clone(),
            path: self.path_of(id),
            size: n.size,
            files: n.files,
            pct: if parent_size > 0 { (n.size as f64 / parent_size as f64 * 100.0) as f32 } else { 0.0 },
            denied: n.denied,
            has_children: !n.children.is_empty(),
        }
    }

    pub fn children(&self, id: u32) -> Vec<ChildInfo> {
        let n = &self.nodes[id as usize];
        n.children.iter().map(|&c| self.info(c, n.size)).collect()
    }

    /// Retrouve un nœud à partir d'un chemin (insensible à la casse).
    pub fn lookup(&self, path: &str) -> Option<u32> {
        let p = winfs::display_path(path);
        let mut parts = p.trim_end_matches('\\').split('\\');
        let first = parts.next()?;
        let root = self.root.trim_end_matches('\\');
        if !first.eq_ignore_ascii_case(root) {
            return None;
        }
        let mut cur = 0u32;
        for part in parts {
            if part.is_empty() {
                continue;
            }
            let found = self.nodes[cur as usize]
                .children
                .iter()
                .copied()
                .find(|&c| self.nodes[c as usize].name.eq_ignore_ascii_case(part))?;
            cur = found;
        }
        Some(cur)
    }

    /// Retire un élément supprimé de l'arbre et met à jour les totaux des dossiers parents,
    /// pour que l'interface reflète la réalité sans relancer toute l'analyse.
    pub fn forget(&mut self, path: &str, size_hint: u64) {
        fn is_under(candidate: &str, base: &str) -> bool {
            let (c, b) = (candidate.as_bytes(), base.as_bytes());
            c.len() >= b.len()
                && c[..b.len()].eq_ignore_ascii_case(b)
                && (c.len() == b.len() || c[b.len()] == b'\\' || b.last() == Some(&b'\\'))
        }
        let disp = winfs::display_path(path).trim_end_matches('\\').to_string();
        let (size, files, parent) = if let Some(id) = self.lookup(&disp) {
            if id == 0 {
                return;
            }
            let n = &self.nodes[id as usize];
            let (s, f, p) = (n.size, n.files, n.parent);
            self.nodes[p as usize].children.retain(|&c| c != id);
            (s, f, p)
        } else if let Some(pid) = disp.rsplit_once('\\').and_then(|(dir, _)| self.lookup(dir)) {
            let s = self.big_files.iter().find(|f| f.path.eq_ignore_ascii_case(&disp)).map(|f| f.size).unwrap_or(size_hint);
            self.nodes[pid as usize].own_size = self.nodes[pid as usize].own_size.saturating_sub(s);
            self.nodes[pid as usize].own_files = self.nodes[pid as usize].own_files.saturating_sub(1);
            (s, 1, pid)
        } else {
            return;
        };
        let mut cur = parent;
        while cur != NO_PARENT {
            let n = &mut self.nodes[cur as usize];
            n.size = n.size.saturating_sub(size);
            n.files = n.files.saturating_sub(files);
            cur = n.parent;
        }
        self.big_files.retain(|f| !is_under(&f.path, &disp));
    }

    /// Dossiers « reconstructibles » (node_modules, target, caches de build…), triés par taille.
    pub fn rebuildable(&self, min_size: u64) -> Vec<ChildInfo> {
        const NAMES: &[&str] = &[
            "node_modules", "target", ".gradle", "__pycache__", ".venv", "venv", ".next", ".nuxt",
            ".parcel-cache", ".cache", "bower_components", ".tox", ".pytest_cache", "Pods", "DerivedData",
        ];
        let mut out: Vec<(u32, u64)> = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if n.size >= min_size && NAMES.iter().any(|x| n.name.eq_ignore_ascii_case(x)) {
                // Évite de lister un node_modules imbriqué dans un node_modules déjà listé.
                let mut p = n.parent;
                let mut nested = false;
                while p != NO_PARENT {
                    let pn = &self.nodes[p as usize];
                    if NAMES.iter().any(|x| pn.name.eq_ignore_ascii_case(x)) {
                        nested = true;
                        break;
                    }
                    p = pn.parent;
                }
                if !nested {
                    out.push((i as u32, n.size));
                }
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1));
        out.into_iter().take(200).map(|(id, _)| self.info(id, self.nodes[0].size)).collect()
    }

    /// Plus gros dossiers « feuilles utiles » : dossiers dont la taille propre domine (hors sous-dossiers).
    pub fn heaviest_dirs(&self, limit: usize) -> Vec<ChildInfo> {
        let mut v: Vec<(u32, u64)> = self.nodes.iter().enumerate().map(|(i, n)| (i as u32, n.own_size)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v.into_iter()
            .take(limit)
            .filter(|x| x.1 > 0)
            .map(|(id, _)| {
                let mut ci = self.info(id, self.nodes[0].size);
                ci.size = self.nodes[id as usize].own_size;
                ci
            })
            .collect()
    }
}

/// Liste en direct les fichiers d'un dossier (triés par taille décroissante).
pub fn list_files(path: &str) -> Vec<FileEntry> {
    let ext = winfs::extended(path.trim_end_matches('\\'));
    let mut v = Vec::new();
    let _ = winfs::read_dir(&ext, |e| {
        if !e.is_dir() {
            v.push(FileEntry {
                path: winfs::display_path(&winfs::join(&ext, &e.name)),
                size: if e.is_cloud_only() { 0 } else { e.size },
                mtime: e.mtime,
            });
        }
    });
    v.sort_by(|a, b| b.size.cmp(&a.size));
    v
}
