//! Libéra — moteur d'analyse et de nettoyage de disque pour Windows.

pub mod apps;
pub mod archive;
pub mod catalog;
pub mod cleaner;
pub mod dupes;
pub mod plugins;
pub mod organize;
pub mod explain;
pub mod mft;
pub mod protect;
pub mod scan;
pub mod settings;
pub mod stale;
pub mod sys;
pub mod winfs;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Zones connues (caches, temporaires, corbeille…) + applications. Quelques secondes.
    Rapide,
    /// Rapide + parcours complet du disque (dossiers et gros fichiers).
    Standard,
    /// Standard + tous les fichiers ≥ 1 Mo + détection de doublons.
    Approfondie,
    /// Comme Approfondie, mais l'arborescence est lue directement dans la table MFT de NTFS
    /// (très rapide). Nécessite les droits administrateur ; sinon repli automatique sur Approfondie.
    Experimentale,
}

pub const BIG_FILE_MIN: u64 = 32 * 1024 * 1024;
pub const DEEP_FILE_MIN: u64 = 1024 * 1024;
/// Les doublons plus petits n'en valent pas la peine : on gagne du temps de hachage.
pub const DUPES_MIN: u64 = 8 * 1024 * 1024;

pub struct Analysis {
    pub disk: Option<sys::DiskInfo>,
    pub junk: Vec<catalog::CategoryResult>,
    pub apps: Vec<apps::AppInfo>,
    pub scan: Option<scan::ScanResult>,
    pub dupes: Vec<dupes::DupGroup>,
    pub elapsed_ms: u128,
    /// Message d'information pour l'utilisateur (ex. repli du mode MFT).
    pub notice: Option<String>,
}

/// Lance une analyse complète. `phase` est appelé à chaque changement d'étape (pour l'interface).
pub fn analyze(mode: Mode, root: &str, progress: &scan::Progress, phase: &dyn Fn(&str)) -> Analysis {
    let t0 = std::time::Instant::now();
    let disk = sys::disk_info(&format!("{}\\", root.trim_end_matches('\\').trim_end_matches(':').to_string() + ":"));

    phase("Zones nettoyables");
    let junk = catalog::measure_all(Some(progress));

    // Les compteurs ont servi à mesurer les catégories : on repart de zéro pour le parcours du disque.
    // Pour le parcours classique, on attend à peu près autant d'octets que le disque en contient : cela donne un pourcentage.
    if mode != Mode::Rapide {
        use std::sync::atomic::Ordering::Relaxed;
        for a in [&progress.files, &progress.dirs, &progress.bytes, &progress.denied] {
            a.store(0, Relaxed);
        }
        progress.expected.store(disk.as_ref().map(|d| d.used).unwrap_or(0), Relaxed);
    }
    let mut scan_res = None;
    let mut notice = None;
    let deep = matches!(mode, Mode::Approfondie | Mode::Experimentale);
    if mode != Mode::Rapide && !progress.cancelled() {
        let min = if deep { DEEP_FILE_MIN } else { BIG_FILE_MIN };
        if mode == Mode::Experimentale {
            phase("Lecture de la table MFT");
            match mft::scan_mft(root, min, progress) {
                Ok(s) => scan_res = Some(s),
                Err(e) => {
                    notice = Some(format!("Lecture MFT impossible ({}). Analyse Approfondie utilisée à la place.", e));
                    for a in [&progress.files, &progress.dirs, &progress.bytes, &progress.denied] {
                        a.store(0, std::sync::atomic::Ordering::Relaxed);
                    }
                    progress.expected.store(disk.as_ref().map(|d| d.used).unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
                }
            }
        }
        if scan_res.is_none() && !progress.cancelled() {
            phase("Parcours du disque");
            scan_res = Some(scan::scan(root, min, progress));
        }
    }

    progress.expected.store(0, std::sync::atomic::Ordering::Relaxed);
    phase("Applications installées");
    let apps = match &scan_res {
        Some(s) => apps::list(&|p| s.lookup(p).map(|id| s.nodes[id as usize].size), false),
        None => apps::list(&|_| None, false),
    };

    let mut dupes = Vec::new();
    if deep && !progress.cancelled() {
        if let Some(s) = &scan_res {
            phase("Recherche de doublons");
            dupes = dupes::find(&s.big_files, DUPES_MIN, progress);
        }
    }

    Analysis { disk, junk, apps, scan: scan_res, dupes, elapsed_ms: t0.elapsed().as_millis(), notice }
}

pub fn format_size(b: u64) -> String {
    const U: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{} o", b) } else { format!("{:.1} {}", v, U[i]).replace('.', ",") }
}
