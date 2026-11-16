//! Garde-fous : empêche toute suppression qui rendrait Windows inutilisable.

use crate::winfs;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "niveau", content = "message")]
pub enum Verdict {
    Ok,
    /// Autorisé, mais l'utilisateur doit être prévenu.
    Attention(String),
    /// Refusé définitivement.
    Interdit(String),
}

fn norm(path: &str) -> String {
    winfs::display_path(path).replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn env_lower(name: &str) -> Option<String> {
    std::env::var(name).ok().map(|v| norm(&v))
}

/// Sous-dossiers de C:\Windows que l'on peut vider sans risque.
const WINDOWS_ALLOWED: &[&str] = &[
    "\\temp",
    "\\softwaredistribution\\download",
    "\\logs",
    "\\minidump",
    "\\memory.dmp",
    "\\livekernelreports",
    "\\installer\\$patchcache$",
    "\\serviceprofiles\\networkservice\\appdata\\local\\microsoft\\windows\\deliveryoptimization\\cache",
];

const PROGRAM_FILES_VITAL: &[&str] = &[
    "windowsapps", "common files", "windows defender", "windows nt", "windows mail", "windows media player",
    "windows photo viewer", "windows security", "windowspowershell", "internet explorer", "reference assemblies",
    "modifiablewindowsapps", "microsoft.net", "msbuild", "dotnet",
];

/// Verdict complet (rapide) : règles système + prudence sur les projets de code, de musique et de création.
pub fn check(path: &str) -> Verdict {
    // Dossiers que l'utilisateur a lui-même déclarés intouchables : refus ferme, tant qu'il ne les retire pas des réglages.
    if crate::settings::is_custom_protected(path) {
        return Verdict::Interdit("Protégé par tes réglages (onglet Protections). Retire-le de la liste pour pouvoir le supprimer.".into());
    }
    match check_base(path) {
        // Autorisé par l'utilisateur dans le chat : plus d'avertissement (les dossiers vitaux restent refusés).
        Verdict::Attention(_) if crate::settings::is_allowed(path) => Verdict::Ok,
        Verdict::Ok => match crate::stale::hint_fast(path) {
            Some(m) => Verdict::Attention(m.to_string()),
            None => Verdict::Ok,
        },
        v => v,
    }
}

/// Idem, avec détection des marqueurs de projet dans les dossiers parents (accès disque : aperçu avant suppression).
pub fn check_deep(path: &str) -> Verdict {
    match check(path) {
        Verdict::Ok => match crate::stale::hint_markers(path) {
            Some(m) => Verdict::Attention(m.to_string()),
            None => Verdict::Ok,
        },
        v => v,
    }
}

fn check_base(path: &str) -> Verdict {
    let p = norm(path);
    let b = p.as_bytes();
    if b.len() < 3 || !b[0].is_ascii_alphabetic() || b[1] != b':' || b[2] != b'\\' {
        if b.len() == 2 && b[1] == b':' {
            return Verdict::Interdit("La racine d'un disque ne peut pas être supprimée.".into());
        }
        return Verdict::Interdit("Chemin non valide : un chemin absolu (ex. C:\\Dossier) est requis.".into());
    }
    let drive = &p[..2];
    let rest = &p[2..]; // commence par '\'
    let sysroot = env_lower("SystemRoot").unwrap_or_else(|| format!("{}\\windows", drive));

    // Fichiers système à la racine
    for f in ["\\pagefile.sys", "\\hiberfil.sys", "\\swapfile.sys", "\\bootmgr", "\\bootnxt"] {
        if rest == f {
            return Verdict::Interdit(format!("Fichier système géré par Windows ({}). Utilise les réglages Windows.", f.trim_start_matches('\\')));
        }
    }
    for d in [
        "\\windows", "\\program files", "\\program files (x86)", "\\programdata", "\\users", "\\recovery", "\\boot",
        "\\efi", "\\$recycle.bin", "\\system volume information", "\\documents and settings", "\\$winreagent",
        "\\config.msi", "\\msocache", "\\perflogs",
    ] {
        if rest == d {
            return Verdict::Interdit("Dossier système essentiel : suppression impossible.".into());
        }
    }

    // C:\Windows\…
    if p == sysroot || p.starts_with(&format!("{}\\", sysroot)) {
        let sub = &p[sysroot.len()..];
        if sub.is_empty() || !WINDOWS_ALLOWED.iter().any(|a| sub == *a || sub.starts_with(&format!("{}\\", a))) {
            return Verdict::Interdit("Dossier interne de Windows : seuls les caches connus (Temp, Logs, mises à jour téléchargées…) peuvent être nettoyés.".into());
        }
    }

    // Profils utilisateurs
    if let Some(after) = rest.strip_prefix("\\users\\") {
        let mut it = after.split('\\');
        let user = it.next().unwrap_or("");
        let sub: Vec<&str> = it.collect();
        if matches!(user, "default" | "default user" | "all users" | "public") && sub.is_empty() {
            return Verdict::Interdit("Profil système : suppression impossible.".into());
        }
        if sub.is_empty() {
            return Verdict::Interdit("Le dossier de profil utilisateur ne peut pas être supprimé (supprime plutôt son contenu).".into());
        }
        let first = sub[0];
        if sub.len() == 1 && matches!(first, "desktop" | "documents" | "downloads" | "pictures" | "videos" | "music" | "appdata" | "onedrive" | "favorites" | "contacts" | "links" | "saved games" | "searches") {
            return Verdict::Interdit("Dossier utilisateur de base : supprime son contenu, pas le dossier lui-même.".into());
        }
        if first.starts_with("ntuser.dat") {
            return Verdict::Interdit("Fichier de registre de l'utilisateur : suppression impossible.".into());
        }
        if first == "appdata" && sub.len() == 2 && matches!(sub[1], "local" | "roaming" | "locallow") {
            return Verdict::Interdit("Dossier AppData de base : suppression impossible.".into());
        }
        if first == "appdata" && sub.len() >= 3 && sub[1] == "local" && sub[2] == "microsoft" && sub.len() == 3 {
            return Verdict::Interdit("Dossier Microsoft d'AppData : suppression impossible.".into());
        }
        if first == "appdata" && sub.len() >= 3 && sub[1] == "local" && sub[2] == "packages" && sub.len() <= 4 {
            return Verdict::Attention("Données d'applications du Microsoft Store : l'application concernée peut cesser de fonctionner.".into());
        }
    }

    // Program Files
    for pf in ["\\program files\\", "\\program files (x86)\\"] {
        if let Some(after) = rest.strip_prefix(pf) {
            let first = after.split('\\').next().unwrap_or("");
            if PROGRAM_FILES_VITAL.contains(&first) || first.starts_with("microsoft") {
                return Verdict::Interdit("Composant Windows ou Microsoft : suppression impossible.".into());
            }
            if !after.contains('\\') {
                return Verdict::Attention("Application installée : il est préférable de la désinstaller proprement (onglet Applications) plutôt que d'effacer son dossier.".into());
            }
        }
    }
    if rest.starts_with("\\programdata\\") && rest.matches('\\').count() == 2 {
        return Verdict::Attention("Données partagées d'un logiciel : vérifie qu'il n'est plus utilisé.".into());
    }
    Verdict::Ok
}

/// Les doublons dans ces zones ne sont pas exploitables par l'utilisateur (binaires, composants).
pub fn is_system_zone(path: &str) -> bool {
    let p = norm(path);
    let sysroot = env_lower("SystemRoot").unwrap_or_else(|| "c:\\windows".into());
    p.starts_with(&sysroot)
        || p[2.min(p.len())..].starts_with("\\program files")
        || p[2.min(p.len())..].starts_with("\\programdata")
        || p.contains("\\appdata\\local\\packages\\")
        || p.contains("\\$recycle.bin")
        || p.contains("\\system volume information")
}
