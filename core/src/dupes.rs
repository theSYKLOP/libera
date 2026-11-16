//! Détection de doublons en trois étapes de plus en plus coûteuses :
//! même taille → même empreinte des 64 premiers Ko → même empreinte complète (BLAKE3).
//! Seuls les groupes déjà très probables sont lus en entier, ce qui garde l'analyse rapide.

use crate::protect;
use crate::scan::{FileEntry, Progress};
use crate::winfs;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::atomic::Ordering;

#[derive(Serialize, Clone, Debug)]
pub struct DupGroup {
    pub size: u64,
    pub wasted: u64,
    pub files: Vec<FileEntry>,
}

fn open(path: &str) -> std::io::Result<File> {
    File::open(winfs::extended(path))
}

fn hash_head(path: &str) -> Option<[u8; 32]> {
    let mut f = open(path).ok()?;
    let mut buf = vec![0u8; 64 * 1024];
    let n = f.read(&mut buf).ok()?;
    Some(*blake3::hash(&buf[..n]).as_bytes())
}

fn hash_full(path: &str) -> Option<[u8; 32]> {
    let mut f = open(path).ok()?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(*h.finalize().as_bytes())
}

pub fn find(files: &[FileEntry], min_size: u64, progress: &Progress) -> Vec<DupGroup> {
    let mut by_size: HashMap<u64, Vec<&FileEntry>> = HashMap::new();
    // Jamais dans les projets de code, les outils, les caches ni les zones système.
    for f in files.iter().filter(|f| f.size >= min_size && !protect::is_system_zone(&f.path) && !crate::stale::is_tool_or_project_path(&f.path)) {
        by_size.entry(f.size).or_default().push(f);
    }
    let candidates: Vec<(u64, Vec<&FileEntry>)> = by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();
    // Progression : on avance par groupe de fichiers de même taille, jusqu'au total des octets à comparer.
    progress.bytes.store(0, Ordering::Relaxed);
    progress.expected.store(candidates.iter().map(|(s, v)| s * v.len() as u64).sum(), Ordering::Relaxed);

    let mut groups: Vec<DupGroup> = candidates
        .par_iter()
        .flat_map(|(size, group)| {
            if progress.cancelled() {
                return Vec::new();
            }
            let mut by_head: HashMap<[u8; 32], Vec<&FileEntry>> = HashMap::new();
            for f in group {
                if let Some(h) = hash_head(&f.path) {
                    by_head.entry(h).or_default().push(f);
                }
            }
            let mut out = Vec::new();
            for (_, same_head) in by_head.into_iter().filter(|(_, v)| v.len() > 1) {
                let mut by_full: HashMap<[u8; 32], Vec<&FileEntry>> = HashMap::new();
                for f in same_head {
                    if let Some(h) = hash_full(&f.path) {
                        by_full.entry(h).or_default().push(f);
                    }
                }
                for (_, same) in by_full.into_iter().filter(|(_, v)| v.len() > 1) {
                    let mut files: Vec<FileEntry> = same.into_iter().cloned().collect();
                    files.sort_by(|a, b| a.mtime.cmp(&b.mtime));
                    out.push(DupGroup { size: *size, wasted: size * (files.len() as u64 - 1), files });
                }
            }
            progress.bytes.fetch_add(size * group.len() as u64, Ordering::Relaxed);
            out
        })
        .collect();
    groups.sort_by(|a, b| b.wasted.cmp(&a.wasted));
    groups.truncate(500);
    groups
}
