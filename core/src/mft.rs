//! Analyse « expérimentale » : lecture directe de la table des fichiers NTFS ($MFT).
//! Au lieu de demander à Windows de lister 1 million de dossiers un par un, on lit la table en
//! continu (lecture séquentielle, très rapide sur SSD) et on reconstruit l'arborescence en mémoire.
//! Exige les droits administrateur (ouverture du volume brut). Lecture seule : rien n'est écrit.

use crate::scan::{DirNode, FileEntry, Progress, ScanResult, NO_PARENT};
use crate::winfs::{self, wide};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::mpsc::sync_channel;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, NTFS_VOLUME_DATA_BUFFER};
use windows_sys::Win32::System::IO::DeviceIoControl;

const GENERIC_READ: u32 = 0x8000_0000;
const CHUNK: usize = 8 * 1024 * 1024;
const MASK48: u64 = 0x0000_FFFF_FFFF_FFFF;

struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Applique le tableau de contrôle (fixups) : rétablit les 2 derniers octets de chaque secteur de 512 o.
fn apply_fixups(rec: &mut [u8]) -> bool {
    if rec.len() < 48 || &rec[0..4] != b"FILE" {
        return false;
    }
    let off = u16_at(rec, 4) as usize;
    let cnt = u16_at(rec, 6) as usize;
    if cnt < 2 || off + cnt * 2 > rec.len() {
        return false;
    }
    for i in 1..cnt {
        let end = i * 512;
        if end > rec.len() {
            break;
        }
        let (a, b) = (rec[off + i * 2], rec[off + i * 2 + 1]);
        rec[end - 2] = a;
        rec[end - 1] = b;
    }
    true
}

#[derive(Default)]
struct Rec {
    /// Index du fichier auquel ces informations s'appliquent (la base, pour un enregistrement d'extension).
    idx: u32,
    extension: bool,
    is_dir: bool,
    parent: u32,
    has_name: bool,
    name: Option<Box<str>>,
    size: Option<u64>,
    attrs: u32,
    mtime: u32,
    has_attr_list: bool,
}

fn parse_record(index: u32, rec: &mut [u8], big_min: u64) -> Option<Rec> {
    if !apply_fixups(rec) {
        return None;
    }
    let flags = u16_at(rec, 0x16);
    if flags & 1 == 0 {
        return None;
    }
    let base = (u64_at(rec, 0x20) & MASK48) as u32;
    let mut r = Rec { idx: if base != 0 { base } else { index }, extension: base != 0, is_dir: flags & 2 != 0, ..Default::default() };
    let used = (u32_at(rec, 0x18) as usize).min(rec.len());
    let mut pos = u16_at(rec, 0x14) as usize;
    let mut name_w: Option<(u8, Vec<u16>, u32)> = None; // (espace de noms, nom, parent)
    while pos + 16 <= used {
        let ty = u32_at(rec, pos);
        if ty == 0xFFFF_FFFF {
            break;
        }
        let len = u32_at(rec, pos + 4) as usize;
        if len < 16 || pos + len > used {
            break;
        }
        let non_res = rec[pos + 8] != 0;
        let name_len = rec[pos + 9];
        match ty {
            0x10 if !non_res => {
                let c = pos + u16_at(rec, pos + 0x14) as usize;
                if c + 0x24 <= rec.len() {
                    let ft = u64_at(rec, c + 8);
                    r.mtime = if ft > 116_444_736_000_000_000 { ((ft / 10_000_000) - 11_644_473_600) as u32 } else { 0 };
                    r.attrs = u32_at(rec, c + 0x20);
                }
            }
            0x20 => r.has_attr_list = true,
            0x30 if !non_res => {
                let c = pos + u16_at(rec, pos + 0x14) as usize;
                if c + 0x42 <= rec.len() {
                    let ns = rec[c + 0x41];
                    let nl = rec[c + 0x40] as usize;
                    // On préfère les noms Win32/POSIX aux noms courts DOS (espace de noms 2).
                    let better = match &name_w { None => true, Some((old, _, _)) => *old == 2 && ns != 2 };
                    if better && c + 0x42 + nl * 2 <= rec.len() {
                        let parent = (u64_at(rec, c) & MASK48) as u32;
                        let w: Vec<u16> = (0..nl).map(|i| u16_at(rec, c + 0x42 + i * 2)).collect();
                        name_w = Some((ns, w, parent));
                    }
                }
            }
            0x80 if name_len == 0 => {
                // flux de données principal
                if !non_res {
                    r.size = Some(u32_at(rec, pos + 0x10) as u64);
                } else if pos + 0x38 <= rec.len() && u64_at(rec, pos + 0x10) == 0 {
                    r.size = Some(u64_at(rec, pos + 0x30));
                }
            }
            _ => {}
        }
        pos += len;
    }
    if let Some((_, w, parent)) = name_w {
        r.parent = parent;
        r.has_name = true;
        // les noms ne sont conservés que pour les dossiers et les gros fichiers (économie de mémoire)
        let keep = r.is_dir || r.size.map(|s| s >= big_min).unwrap_or(false) || r.has_attr_list;
        if keep {
            r.name = Some(winfs::from_wide(&w).into_boxed_str());
        }
    }
    Some(r)
}

/// Décode une liste d'extents NTFS : (LCN de départ ou None si zone creuse, longueur en clusters).
fn parse_runs(b: &[u8]) -> Vec<(Option<i64>, u64)> {
    let mut runs = Vec::new();
    let (mut pos, mut lcn) = (0usize, 0i64);
    while pos < b.len() && b[pos] != 0 {
        let h = b[pos];
        let (ls, os) = ((h & 0xF) as usize, (h >> 4) as usize);
        pos += 1;
        if pos + ls + os > b.len() || ls == 0 || ls > 8 || os > 8 {
            break;
        }
        let mut len = 0u64;
        for i in 0..ls {
            len |= (b[pos + i] as u64) << (8 * i);
        }
        pos += ls;
        if os == 0 {
            runs.push((None, len));
        } else {
            let mut off = 0i64;
            for i in 0..os {
                off |= (b[pos + i] as i64) << (8 * i);
            }
            if b[pos + os - 1] & 0x80 != 0 && os < 8 {
                off |= -1i64 << (8 * os); // extension du signe
            }
            lcn += off;
            runs.push((Some(lcn), len));
        }
        pos += os;
    }
    runs
}

fn read_at(h: HANDLE, offset: u64, buf: &mut [u8]) -> Result<(), String> {
    unsafe {
        if SetFilePointerEx(h, offset as i64, std::ptr::null_mut(), FILE_BEGIN) == 0 {
            return Err(format!("positionnement impossible (code {})", GetLastError()));
        }
        let mut got = 0u32;
        if ReadFile(h, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut got, std::ptr::null_mut()) == 0 {
            return Err(format!("lecture impossible (code {})", GetLastError()));
        }
        if (got as usize) < buf.len() {
            return Err("lecture incomplète".into());
        }
    }
    Ok(())
}

pub fn scan_mft(drive: &str, big_min: u64, progress: &Progress) -> Result<ScanResult, String> {
    let t0 = std::time::Instant::now();
    let letter = drive.chars().next().ok_or("Disque invalide")?.to_ascii_uppercase();
    let vol = wide(&format!("\\\\.\\{}:", letter));
    let h = unsafe {
        CreateFileW(vol.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut())
    };
    if h == INVALID_HANDLE_VALUE {
        let e = unsafe { GetLastError() };
        return Err(if e == ERROR_ACCESS_DENIED { "les droits administrateur sont requis pour lire la table MFT".into() } else { format!("ouverture du volume impossible (code {})", e) });
    }
    let result = scan_inner(h, letter, big_min, progress, t0);
    unsafe { CloseHandle(h) };
    result
}

fn scan_inner(h: HANDLE, letter: char, big_min: u64, progress: &Progress, t0: std::time::Instant) -> Result<ScanResult, String> {
    // 1. Géométrie du volume
    let mut vd: NTFS_VOLUME_DATA_BUFFER = unsafe { std::mem::zeroed() };
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(h, FSCTL_GET_NTFS_VOLUME_DATA, std::ptr::null(), 0, &mut vd as *mut _ as *mut _, std::mem::size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32, &mut ret, std::ptr::null_mut())
    };
    if ok == 0 {
        return Err("le volume n'est pas au format NTFS".into());
    }
    let rec_size = vd.BytesPerFileRecordSegment as usize;
    let bpc = vd.BytesPerCluster as u64;
    let sector = vd.BytesPerSector.max(512) as usize;
    if rec_size == 0 || rec_size % 512 != 0 {
        return Err("taille d'enregistrement MFT inattendue".into());
    }
    let mft_bytes = vd.MftValidDataLength as u64;
    let total_records = (mft_bytes / rec_size as u64) as usize;

    // 2. Enregistrement 0 ($MFT) : où se trouve la table sur le disque ?
    let first = ((rec_size.max(sector) + 4095) / 4096) * 4096;
    let mut buf0 = vec![0u8; first];
    read_at(h, vd.MftStartLcn as u64 * bpc, &mut buf0)?;
    let rec0 = &mut buf0[..rec_size];
    if !apply_fixups(rec0) {
        return Err("enregistrement $MFT illisible".into());
    }
    let mut runs = Vec::new();
    let (mut pos, used) = (u16_at(rec0, 0x14) as usize, (u32_at(rec0, 0x18) as usize).min(rec_size));
    while pos + 16 <= used {
        let ty = u32_at(rec0, pos);
        if ty == 0xFFFF_FFFF {
            break;
        }
        let len = u32_at(rec0, pos + 4) as usize;
        if len < 16 || pos + len > used {
            break;
        }
        if ty == 0x80 && rec0[pos + 8] != 0 && rec0[pos + 9] == 0 {
            let ro = u16_at(rec0, pos + 0x20) as usize;
            runs = parse_runs(&rec0[pos + ro..pos + len]);
        }
        pos += len;
    }
    if runs.is_empty() {
        return Err("emplacement de la table MFT introuvable".into());
    }

    // 3. Lecture séquentielle de toute la table (un thread lit, les autres analysent)
    let (tx, rx) = sync_channel::<(usize, Vec<u8>)>(3);
    let hs = SendHandle(h);
    let runs2 = runs.clone();
    let reader = std::thread::spawn(move || -> Result<(), String> {
        let hs = hs;
        let mut remaining = mft_bytes;
        let mut rec_index = 0usize;
        for (lcn, clusters) in runs2 {
            let mut run_bytes = clusters * bpc;
            let Some(lcn) = lcn else { // zone creuse : enregistrements vides
                let skip = run_bytes.min(remaining);
                remaining -= skip;
                rec_index += (skip as usize) / rec_size;
                continue;
            };
            let mut off = lcn as u64 * bpc;
            while run_bytes > 0 && remaining > 0 {
                let want = (CHUNK as u64).min(run_bytes);
                let mut v = vec![0u8; want as usize];
                read_at(hs.0, off, &mut v)?;
                let useful = (want.min(remaining)) as usize;
                v.truncate(useful - useful % rec_size);
                let n = v.len() / rec_size;
                if tx.send((rec_index, v)).is_err() {
                    return Ok(());
                }
                rec_index += n;
                off += want;
                run_bytes -= want;
                remaining = remaining.saturating_sub(want);
            }
        }
        Ok(())
    });

    // Progression : on a lu toute la table quand on a lu `mft_bytes` octets.
    progress.expected.store(mft_bytes, Ordering::Relaxed);
    progress.bytes.store(0, Ordering::Relaxed);
    let n = total_records.max(64);
    let mut parent = vec![u32::MAX; n];
    let mut size = vec![0u64; n];
    let mut flags = vec![0u8; n]; // 1 = en usage, 2 = dossier, 4 = nom connu, 8 = en ligne seulement
    let mut mtime = vec![0u32; n];
    let mut names: HashMap<u32, Box<str>> = HashMap::with_capacity(1 << 20);
    let mut cloud_bytes = 0u64;

    for (start, mut chunk) in rx {
        if progress.cancelled() {
            break;
        }
        let recs: Vec<Rec> = chunk
            .par_chunks_mut(rec_size)
            .enumerate()
            .filter_map(|(i, r)| parse_record((start + i) as u32, r, big_min))
            .collect();
        progress.files.fetch_add(recs.len() as u64, Ordering::Relaxed);
        progress.bytes.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        for r in recs {
            let i = r.idx as usize;
            if i >= n {
                continue;
            }
            if let Some(s) = r.size {
                if s > size[i] {
                    size[i] = s;
                }
            }
            if r.extension {
                continue;
            }
            flags[i] |= 1 | if r.is_dir { 2 } else { 0 } | if r.has_name { 4 } else { 0 };
            if r.attrs & (0x0040_0000 | 0x0004_0000 | 0x1000) != 0 {
                flags[i] |= 8;
            }
            parent[i] = r.parent;
            mtime[i] = r.mtime;
            if let Some(nm) = r.name {
                names.insert(i as u32, nm);
            }
        }
        if let Ok(mut c) = progress.current.try_lock() {
            *c = format!("Table MFT : {} enregistrements lus", fmt_n(progress.files.load(Ordering::Relaxed)));
        }
    }
    match reader.join() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("le thread de lecture a échoué".into()),
    }

    // 4. Reconstruction de l'arborescence (dossiers uniquement)
    let root_rec = 5usize;
    if root_rec >= n || flags[root_rec] & 2 == 0 {
        return Err("racine du volume introuvable dans la MFT".into());
    }
    let mut node_of = vec![u32::MAX; n];
    let mut nodes: Vec<DirNode> = Vec::new();
    let root_name = format!("{}:\\", letter);
    nodes.push(DirNode { name: root_name.clone(), parent: NO_PARENT, children: Vec::new(), own_size: 0, own_files: 0, size: 0, files: 0, denied: false });
    node_of[root_rec] = 0;
    let dirs: Vec<usize> = (0..n).filter(|&i| i != root_rec && flags[i] & 3 == 3 && flags[i] & 4 != 0).collect();
    for &d in &dirs {
        node_of[d] = nodes.len() as u32;
        nodes.push(DirNode {
            name: names.get(&(d as u32)).map(|s| s.to_string()).unwrap_or_default(),
            parent: NO_PARENT,
            children: Vec::new(),
            own_size: 0,
            own_files: 0,
            size: 0,
            files: 0,
            denied: false,
        });
    }
    let mut linked: Vec<u32> = Vec::with_capacity(dirs.len());
    for &d in &dirs {
        let p = parent[d] as usize;
        let pn = if p < n { node_of[p] } else { u32::MAX };
        if pn != u32::MAX && pn != node_of[d] {
            nodes[node_of[d] as usize].parent = pn;
            nodes[pn as usize].children.push(node_of[d]);
            linked.push(node_of[d]);
        }
    }
    // Les dossiers dont la chaîne de parents ne remonte pas à la racine (orphelins) sont écartés.
    let mut reachable = vec![false; nodes.len()];
    let mut order: Vec<u32> = Vec::with_capacity(nodes.len());
    let mut stack = vec![0u32];
    while let Some(x) = stack.pop() {
        reachable[x as usize] = true;
        order.push(x);
        stack.extend(nodes[x as usize].children.iter().copied());
    }

    // fichiers : attribués à leur dossier parent
    let mut cloud_total = 0u64;
    let mut big: Vec<(usize, u64)> = Vec::new();
    for i in 0..n {
        if flags[i] & 3 != 1 {
            continue; // pas en usage, ou dossier
        }
        let p = parent[i] as usize;
        let pn = if p < n { node_of[p] } else { u32::MAX };
        if pn == u32::MAX || !reachable[pn as usize] {
            continue;
        }
        let mut s = size[i];
        if flags[i] & 8 != 0 {
            cloud_total += s;
            s = 0;
        }
        let nd = &mut nodes[pn as usize];
        nd.own_size += s;
        nd.own_files += 1;
        if s >= big_min {
            big.push((i, s));
        }
    }
    cloud_bytes += cloud_total;

    // totaux cumulés (enfants avant parents)
    for &x in order.iter().rev() {
        let (s, f, p) = {
            let nd = &nodes[x as usize];
            (nd.size + nd.own_size, nd.files + nd.own_files as u64, nd.parent)
        };
        nodes[x as usize].size = s;
        nodes[x as usize].files = f;
        if p != NO_PARENT {
            nodes[p as usize].size += s;
            nodes[p as usize].files += f;
        }
    }
    // On renumérote en ne gardant que les dossiers atteignables, enfants triés par taille.
    let mut remap = vec![u32::MAX; nodes.len()];
    let mut keep: Vec<u32> = Vec::with_capacity(order.len());
    let mut q = std::collections::VecDeque::new();
    q.push_back(0u32);
    while let Some(x) = q.pop_front() {
        remap[x as usize] = keep.len() as u32;
        keep.push(x);
        let mut kids = nodes[x as usize].children.clone();
        kids.sort_by(|a, b| nodes[*b as usize].size.cmp(&nodes[*a as usize].size));
        nodes[x as usize].children = kids.clone();
        q.extend(kids);
    }
    let mut final_nodes: Vec<DirNode> = Vec::with_capacity(keep.len());
    for &old in &keep {
        let o = &nodes[old as usize];
        final_nodes.push(DirNode {
            name: o.name.clone(),
            parent: if o.parent == NO_PARENT { NO_PARENT } else { remap[o.parent as usize] },
            children: o.children.iter().map(|c| remap[*c as usize]).collect(),
            own_size: o.own_size,
            own_files: o.own_files,
            size: o.size,
            files: o.files,
            denied: false,
        });
    }
    let node_of_final = |rec: usize| -> Option<u32> {
        let nn = node_of[rec];
        if nn == u32::MAX { None } else { let r = remap[nn as usize]; if r == u32::MAX { None } else { Some(r) } }
    };

    let mut res = ScanResult {
        root: root_name,
        nodes: final_nodes,
        big_files: Vec::new(),
        cloud_bytes,
        denied_dirs: 0,
        elapsed_ms: 0,
    };
    let mut files_out: Vec<FileEntry> = Vec::with_capacity(big.len());
    for (i, s) in big {
        let Some(name) = names.get(&(i as u32)) else { continue };
        let Some(dn) = node_of_final(parent[i] as usize) else { continue };
        let dir = res.path_of(dn);
        files_out.push(FileEntry { path: winfs::join(&dir, name), size: s, mtime: mtime[i] as i64 });
    }
    files_out.sort_by(|a, b| b.size.cmp(&a.size));
    res.big_files = files_out;
    res.elapsed_ms = t0.elapsed().as_millis();
    progress.dirs.store(res.nodes.len() as u64, Ordering::Relaxed);
    Ok(res)
}

fn fmt_n(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('\u{202F}');
        }
        out.push(c);
    }
    out
}
