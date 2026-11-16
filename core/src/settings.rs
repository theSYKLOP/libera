//! Réglages de protection, enregistrés sur le disque et modifiables à tout moment.
//!
//! Principe : Libéra n'impose pas de profil. L'utilisateur choisit ce qu'il veut garder (projets de code, musique,
//! création graphique, dossiers précis). Seuls les dossiers vitaux de Windows restent toujours protégés (module `protect`),
//! car les supprimer rendrait l'ordinateur inutilisable.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// Faux tant que l'utilisateur n'a rien choisi : l'assistant lui pose alors la question avant toute proposition.
    pub configured: bool,
    pub protect_dev: bool,
    pub protect_music: bool,
    pub protect_design: bool,
    /// Dossiers ou fichiers que l'utilisateur ne veut jamais voir supprimés (préfixes de chemin).
    pub custom_protected: Vec<String>,
    /// Exceptions accordées par l'utilisateur dans le chat : ces chemins peuvent être supprimés malgré les protections
    /// (jamais pour les dossiers vitaux de Windows). Révocables dans l'onglet Protections.
    pub allowed_paths: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        // Par prudence tant que rien n'est choisi ; l'utilisateur peut tout lever en un clic.
        Settings { configured: false, protect_dev: true, protect_music: true, protect_design: true, custom_protected: Vec::new(), allowed_paths: Vec::new() }
    }
}

static CURRENT: RwLock<Option<Settings>> = RwLock::new(None);

fn file() -> PathBuf {
    let base = std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    base.join("Libera").join("settings.json")
}

pub fn get() -> Settings {
    CURRENT.read().ok().and_then(|g| g.clone()).unwrap_or_default()
}

/// Charge les réglages enregistrés (à appeler au démarrage).
pub fn load() {
    let s = std::fs::read_to_string(file()).ok().and_then(|t| serde_json::from_str::<Settings>(&t).ok()).unwrap_or_default();
    if let Ok(mut g) = CURRENT.write() {
        *g = Some(s);
    }
}

/// Enregistre de nouveaux réglages (mémoire + disque).
pub fn set(mut s: Settings) -> Result<(), String> {
    for list in [&mut s.custom_protected, &mut s.allowed_paths] {
        list.retain(|p| !p.trim().is_empty());
        list.sort_by_key(|p| p.to_lowercase());
        list.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    }
    if let Ok(mut g) = CURRENT.write() {
        *g = Some(s.clone());
    }
    let f = file();
    if let Some(d) = f.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(f, serde_json::to_string_pretty(&s).map_err(|e| e.to_string())?).map_err(|e| format!("Enregistrement impossible : {}", e))
}

/// Ce que `after` assouplit par rapport à `before` (protections levées, dossiers retirés).
pub fn loosened(before: &Settings, after: &Settings) -> Vec<String> {
    let mut v = Vec::new();
    if before.protect_dev && !after.protect_dev { v.push("Projets de code".to_string()); }
    if before.protect_music && !after.protect_music { v.push("Musique".to_string()); }
    if before.protect_design && !after.protect_design { v.push("Création graphique".to_string()); }
    for p in &before.custom_protected {
        if !after.custom_protected.iter().any(|q| q.eq_ignore_ascii_case(p)) {
            v.push(format!("Dossier {}", p));
        }
    }
    v
}

fn under_any(list: &[String], path: &str) -> bool {
    let p = crate::winfs::display_path(path).replace('/', "\\").trim_end_matches('\\').to_lowercase();
    list.iter().any(|c| {
        let c = c.replace('/', "\\").trim_end_matches('\\').to_lowercase();
        !c.is_empty() && (p == c || p.starts_with(&format!("{}\\", c)))
    })
}

/// Vrai si l'utilisateur a explicitement autorisé la suppression de ce chemin (dans le chat ou les réglages).
pub fn is_allowed(path: &str) -> bool {
    under_any(&get().allowed_paths, path)
}

/// Enregistre une autorisation : le chemin n'est plus signalé ni refusé, et sort des dossiers protégés personnalisés.
pub fn allow(paths: &[String]) -> Result<(), String> {
    let mut s = get();
    for p in paths {
        let disp = crate::winfs::display_path(p).trim_end_matches('\\').to_string();
        s.custom_protected.retain(|c| !c.eq_ignore_ascii_case(&disp));
        if !s.allowed_paths.iter().any(|a| a.eq_ignore_ascii_case(&disp)) {
            s.allowed_paths.push(disp);
        }
    }
    s.configured = true;
    set(s)
}

/// Vrai si le chemin est dans un dossier protégé par l'utilisateur.
pub fn is_custom_protected(path: &str) -> bool {
    under_any(&get().custom_protected, path)
}
