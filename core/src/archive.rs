//! Archivage : regroupe des dossiers/fichiers dans un .zip, puis VÉRIFIE l'archive (relecture complète, CRC contrôlé)
//! avant de dire qu'elle est fiable. Aucune suppression ici : c'est à l'utilisateur de confirmer la suppression des originaux.

use crate::winfs::{self, Entry};
use serde::Serialize;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

#[derive(Default)]
pub struct ArchiveProgress {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
    pub phase: Mutex<String>,
    pub current: Mutex<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ArchiveReport {
    pub zip_path: String,
    pub files: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub verified: bool,
    pub skipped: Vec<String>,
    pub sources: Vec<String>,
}

/// Formats déjà compressés : on les stocke tels quels (beaucoup plus rapide, taille identique).
const STORED_EXT: &[&str] = &[
    "zip", "rar", "7z", "gz", "tgz", "xz", "zst", "mp3", "m4a", "aac", "ogg", "opus", "flac", "mp4", "mkv", "avi", "mov", "webm", "jpg", "jpeg", "png", "webp",
    "heic", "avif", "gif", "docx", "xlsx", "pptx", "apk", "jar", "woff2",
];

fn list_files(dir_ext: &str, rel: &str, out: &mut Vec<(String, String, u64)>, skipped: &mut Vec<String>) {
    let mut subdirs = Vec::new();
    let r = winfs::read_dir(dir_ext, |e: Entry| {
        let rel_path = format!("{}/{}", rel, e.name);
        if e.is_reparse() {
            skipped.push(format!("{} (lien symbolique ignoré)", rel_path));
        } else if e.is_dir() {
            subdirs.push((winfs::join(dir_ext, &e.name), rel_path));
        } else if e.is_cloud_only() {
            skipped.push(format!("{} (fichier en ligne seulement, non téléchargé)", rel_path));
        } else {
            out.push((winfs::join(dir_ext, &e.name), rel_path, e.size));
        }
    });
    if r.is_err() {
        skipped.push(format!("{} (dossier illisible)", rel));
    }
    for (d, r) in subdirs {
        list_files(&d, &r, out, skipped);
    }
}

fn unique_name(used: &mut std::collections::HashSet<String>, base: &str) -> String {
    let mut n = base.to_string();
    let mut i = 2;
    while !used.insert(n.to_lowercase()) {
        n = format!("{} ({})", base, i);
        i += 1;
    }
    n
}

/// Crée l'archive. `dest_zip` : chemin complet du .zip à créer. Vérifie l'espace libre avant de commencer.
pub fn create(sources: &[String], dest_zip: &str, progress: &ArchiveProgress) -> Result<ArchiveReport, String> {
    let set = |s: &str| {
        if let Ok(mut p) = progress.phase.lock() {
            *p = s.to_string();
        }
    };
    set("Inventaire des fichiers");
    let dest_disp = winfs::display_path(dest_zip);
    let dest_l = dest_disp.to_lowercase();
    let mut plan: Vec<(String, String, u64)> = Vec::new(); // (chemin disque étendu, nom dans le zip, taille)
    let mut skipped = Vec::new();
    let mut used = std::collections::HashSet::new();
    let mut srcs = Vec::new();
    for s in sources {
        let disp = winfs::display_path(s).trim_end_matches('\\').to_string();
        let sl = disp.to_lowercase();
        if dest_l.starts_with(&format!("{}\\", sl)) {
            return Err(format!("L'archive ne peut pas être créée à l'intérieur de « {} » (elle s'archiverait elle-même).", disp));
        }
        let ext = winfs::extended(&disp);
        let attrs = winfs::attrs(&ext).map_err(|c| format!("« {} » est inaccessible ({})", disp, winfs::explain(c)))?;
        let base = disp.rsplit('\\').next().unwrap_or("element").to_string();
        let top = unique_name(&mut used, &base);
        if attrs & 0x10 != 0 {
            list_files(&ext, &top, &mut plan, &mut skipped);
        } else {
            plan.push((ext, top, winfs::file_size(&winfs::extended(&disp)).unwrap_or(0)));
        }
        srcs.push(disp);
    }
    if plan.is_empty() {
        return Err("Rien à archiver : aucun fichier lisible dans la sélection.".into());
    }
    let total: u64 = plan.iter().map(|p| p.2).sum();
    progress.total.store(total, Ordering::Relaxed);
    progress.bytes.store(0, Ordering::Relaxed);
    progress.files.store(0, Ordering::Relaxed);

    // Espace libre : au pire, l'archive pèse autant que les originaux.
    let drive = dest_disp.chars().take(2).collect::<String>() + "\\";
    if let Some(d) = crate::sys::disk_info(&drive) {
        if d.free < total + (512 << 20) {
            return Err(format!(
                "Espace insuffisant sur {} : {} libres, {} nécessaires au maximum. Choisis un autre disque (ex. un disque externe) pour l'archive.",
                drive, crate::format_size(d.free), crate::format_size(total)
            ));
        }
    }
    if let Some(parent) = std::path::Path::new(&dest_disp).parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Dossier de destination impossible à créer : {}", e))?;
    }
    if std::path::Path::new(&dest_disp).exists() {
        return Err(format!("« {} » existe déjà : choisis un autre nom.", dest_disp));
    }

    set("Compression");
    let file = File::create(&dest_disp).map_err(|e| format!("Création de l'archive impossible : {}", e))?;
    let mut zw = ZipWriter::new(BufWriter::with_capacity(1 << 20, file));
    let manifest = format!(
        "Archive créée par Libéra.\r\nSources d'origine (pour restaurer, extraire chaque dossier à son emplacement d'origine) :\r\n{}\r\n",
        srcs.iter().map(|s| format!(" - {}", s)).collect::<Vec<_>>().join("\r\n")
    );
    let base = SimpleFileOptions::default().large_file(true);
    zw.start_file("LISEZMOI-LIBERA.txt", base.compression_method(CompressionMethod::Deflated)).map_err(|e| e.to_string())?;
    zw.write_all(manifest.as_bytes()).map_err(|e| e.to_string())?;

    let mut buf = vec![0u8; 1 << 20];
    let mut written = 0u64;
    for (disk, name, _size) in &plan {
        if progress.cancel.load(Ordering::Relaxed) {
            drop(zw);
            let _ = std::fs::remove_file(&dest_disp);
            return Err("Archivage annulé : l'archive partielle a été supprimée.".into());
        }
        if let Ok(mut c) = progress.current.lock() {
            *c = winfs::display_path(disk);
        }
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        let method = if STORED_EXT.contains(&ext.as_str()) { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        let mut f = match File::open(disk) {
            Ok(f) => f,
            Err(e) => {
                skipped.push(format!("{} (lecture impossible : {})", name, e));
                continue;
            }
        };
        zw.start_file(name.as_str(), base.compression_method(method)).map_err(|e| e.to_string())?;
        loop {
            let n = f.read(&mut buf).map_err(|e| format!("Lecture de « {} » : {}", name, e))?;
            if n == 0 {
                break;
            }
            zw.write_all(&buf[..n]).map_err(|e| format!("Écriture de l'archive : {}", e))?;
            progress.bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
        progress.files.fetch_add(1, Ordering::Relaxed);
        written += 1;
    }
    zw.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())?;
    let bytes_out = std::fs::metadata(&dest_disp).map(|m| m.len()).unwrap_or(0);

    // Vérification : on relit chaque entrée jusqu'au bout (le CRC est contrôlé par la bibliothèque).
    set("Vérification de l'archive");
    progress.bytes.store(0, Ordering::Relaxed);
    let verified = verify(&dest_disp, written + 1, progress).is_ok();
    Ok(ArchiveReport { zip_path: dest_disp, files: written, bytes_in: total, bytes_out, verified, skipped, sources: srcs })
}

/// Relit toute l'archive et contrôle le nombre d'entrées et leur somme de contrôle.
pub fn verify(zip_path: &str, expected_entries: u64, progress: &ArchiveProgress) -> Result<(), String> {
    let f = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut za = ZipArchive::new(BufReader::new(f)).map_err(|e| format!("Archive illisible : {}", e))?;
    if (za.len() as u64) != expected_entries {
        return Err(format!("{} entrées trouvées au lieu de {}", za.len(), expected_entries));
    }
    let mut buf = vec![0u8; 1 << 20];
    for i in 0..za.len() {
        let mut e = za.by_index(i).map_err(|e| format!("Entrée {} illisible : {}", i, e))?;
        loop {
            let n = e.read(&mut buf).map_err(|er| format!("Entrée n°{} corrompue : {}", i, er))?;
            if n == 0 {
                break;
            }
            progress.bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
    }
    Ok(())
}
