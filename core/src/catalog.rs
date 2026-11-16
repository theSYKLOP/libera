//! Catalogue des zones nettoyables connues (caches, journaux, restes de mises à jour…).
//! Chaque entrée est sûre ou signalée comme « modérée » ; rien n'est supprimé sans action explicite.

use crate::cleaner::{self, DelProgress, DeleteOptions, DeleteReport};
use crate::scan;
use crate::sys;
use crate::winfs;
use rayon::prelude::*;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub description: String,
    pub group: String,
    /// "sur" ou "modere"
    pub risk: String,
    pub needs_admin: bool,
    pub targets: Vec<String>,
    /// Vider le contenu mais garder le dossier lui-même.
    pub keep_root: bool,
    /// Action système spéciale (pas un simple dossier).
    pub action: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct CategoryResult {
    #[serde(flatten)]
    pub cat: Category,
    pub size: u64,
    pub files: u64,
    pub size_known: bool,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn exists(p: &str) -> bool {
    winfs::attrs(&winfs::extended(p)).is_ok()
}

fn sub_dirs(parent: &str, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    let _ = winfs::read_dir(&winfs::extended(parent), |e| {
        if e.is_dir() && keep(&e.name) {
            out.push(winfs::join(parent, &e.name));
        }
    });
    out
}

fn files_with_prefix(parent: &str, prefixes: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let _ = winfs::read_dir(&winfs::extended(parent), |e| {
        let l = e.name.to_lowercase();
        if !e.is_dir() && prefixes.iter().any(|p| l.starts_with(p)) {
            out.push(winfs::join(parent, &e.name));
        }
    });
    out
}

#[allow(clippy::too_many_arguments)]
fn cat(id: &str, name: &str, desc: &str, group: &str, risk: &str, admin: bool, targets: Vec<String>, keep_root: bool) -> Category {
    Category {
        id: id.into(),
        name: name.into(),
        description: desc.into(),
        group: group.into(),
        risk: risk.into(),
        needs_admin: admin,
        targets,
        keep_root,
        action: None,
    }
}

fn browser_targets(user_data: &str) -> Vec<String> {
    let mut v = Vec::new();
    for prof in sub_dirs(user_data, |n| n == "Default" || n.starts_with("Profile ")) {
        for s in ["Cache", "Code Cache", "GPUCache", "Service Worker\\CacheStorage", "Service Worker\\ScriptCache", "DawnCache"] {
            let p = winfs::join(&prof, s);
            if exists(&p) {
                v.push(p);
            }
        }
    }
    v
}

pub fn categories() -> Vec<Category> {
    let local = env("LOCALAPPDATA").unwrap_or_default();
    let roaming = env("APPDATA").unwrap_or_default();
    let profile = env("USERPROFILE").unwrap_or_default();
    let temp = env("TEMP").unwrap_or_default();
    let sysroot = env("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let pdata = env("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
    let drive = env("SystemDrive").unwrap_or_else(|| "C:".into());
    let j = |a: &str, b: &str| winfs::join(a, b);

    let mut v: Vec<Category> = Vec::new();

    // ── Windows
    v.push(cat("temp_user", "Fichiers temporaires (utilisateur)", "Fichiers créés par les programmes pendant leur exécution. Ceux en cours d'utilisation sont ignorés.", "Windows", "sur", false, vec![temp], true));
    v.push(cat("temp_windows", "Fichiers temporaires (Windows)", "Dossier Temp du système.", "Windows", "sur", true, vec![j(&sysroot, "Temp")], true));
    v.push(cat("thumbs", "Cache des miniatures et icônes", "Windows les recrée automatiquement.", "Windows", "sur", false, files_with_prefix(&j(&local, "Microsoft\\Windows\\Explorer"), &["thumbcache_", "iconcache_"]), false));
    v.push(cat("inetcache", "Cache Internet de Windows", "Fichiers Internet temporaires utilisés par Windows et certaines applications.", "Windows", "sur", false, vec![j(&local, "Microsoft\\Windows\\INetCache")], true));
    v.push(cat("wu_download", "Mises à jour Windows téléchargées", "Fichiers déjà installés, conservés par Windows Update. Il les retélécharge si besoin.", "Windows", "sur", true, vec![j(&sysroot, "SoftwareDistribution\\Download")], true));
    v.push(cat("delivery_opt", "Optimisation de la distribution", "Cache de partage des mises à jour entre PC.", "Windows", "sur", true, vec![j(&sysroot, "ServiceProfiles\\NetworkService\\AppData\\Local\\Microsoft\\Windows\\DeliveryOptimization\\Cache")], true));
    v.push(cat("wer", "Rapports d'erreurs Windows", "Rapports de plantage envoyés ou en attente.", "Windows", "sur", false, vec![j(&local, "Microsoft\\Windows\\WER"), j(&pdata, "Microsoft\\Windows\\WER\\ReportQueue"), j(&pdata, "Microsoft\\Windows\\WER\\ReportArchive")], true));
    v.push(cat("dumps", "Vidages mémoire (plantages)", "Fichiers de diagnostic après un écran bleu ou un plantage.", "Windows", "sur", false, vec![j(&local, "CrashDumps"), j(&sysroot, "Minidump"), j(&sysroot, "MEMORY.DMP"), j(&sysroot, "LiveKernelReports")], true));
    v.push(cat("win_logs", "Journaux Windows", "Anciens journaux d'installation et de maintenance.", "Windows", "sur", true, vec![j(&sysroot, "Logs")], true));
    v.push(cat("windows_old", "Ancienne installation de Windows (Windows.old)", "Sauvegarde de la version précédente. Après suppression, le retour arrière vers l'ancienne version n'est plus possible.", "Windows", "modere", true, vec![j(&drive, "Windows.old")], false));

    // ── Navigateurs
    let chrome = browser_targets(&j(&local, "Google\\Chrome\\User Data"));
    let edge = browser_targets(&j(&local, "Microsoft\\Edge\\User Data"));
    let brave = browser_targets(&j(&local, "BraveSoftware\\Brave-Browser\\User Data"));
    let ff: Vec<String> = sub_dirs(&j(&local, "Mozilla\\Firefox\\Profiles"), |_| true).into_iter().map(|p| j(&p, "cache2")).filter(|p| exists(p)).collect();
    v.push(cat("chrome", "Cache de Google Chrome", "Pages et images mises en cache. Mots de passe, favoris et historique sont conservés.", "Navigateurs", "sur", false, chrome, true));
    v.push(cat("edge", "Cache de Microsoft Edge", "Pages et images mises en cache. Mots de passe, favoris et historique sont conservés.", "Navigateurs", "sur", false, edge, true));
    v.push(cat("brave", "Cache de Brave", "Pages et images mises en cache.", "Navigateurs", "sur", false, brave, true));
    v.push(cat("firefox", "Cache de Firefox", "Pages et images mises en cache.", "Navigateurs", "sur", false, ff, true));

    // ── Applications
    v.push(cat("vscode", "Cache de Visual Studio Code", "Caches de l'éditeur, recréés au besoin.", "Applications", "sur", false, ["Cache", "CachedData", "Code Cache", "GPUCache", "CachedExtensionVSIXs"].iter().map(|s| j(&j(&roaming, "Code"), s)).collect(), true));
    v.push(cat("discord", "Cache de Discord", "Images et médias mis en cache.", "Applications", "sur", false, ["Cache", "Code Cache", "GPUCache"].iter().map(|s| j(&j(&roaming, "discord"), s)).collect(), true));
    v.push(cat("teams", "Cache de Teams", "Cache de l'application de messagerie.", "Applications", "sur", false, ["Cache", "Code Cache", "GPUCache"].iter().map(|s| j(&j(&roaming, "Microsoft\\Teams"), s)).collect(), true));
    v.push(cat("spotify", "Cache de Spotify", "Musique mise en cache pour l'écoute hors-ligne temporaire.", "Applications", "sur", false, vec![j(&local, "Spotify\\Data"), j(&local, "Spotify\\Storage")], true));

    // ── Développement
    v.push(cat("npm", "Cache npm", "Paquets Node.js téléchargés, retéléchargés au besoin.", "Développement", "sur", false, vec![j(&local, "npm-cache")], true));
    v.push(cat("pip", "Cache pip", "Paquets Python téléchargés.", "Développement", "sur", false, vec![j(&local, "pip\\Cache")], true));
    v.push(cat("yarn", "Cache Yarn / pnpm", "Paquets JavaScript téléchargés.", "Développement", "sur", false, vec![j(&local, "Yarn\\Cache"), j(&local, "pnpm\\store"), j(&local, "pnpm-cache")], true));
    v.push(cat("cargo", "Cache Cargo (Rust)", "Sources et archives de crates. Retéléchargées à la prochaine compilation.", "Développement", "modere", false, vec![j(&profile, ".cargo\\registry\\cache"), j(&profile, ".cargo\\registry\\src")], true));
    v.push(cat("nuget", "Cache NuGet (.NET)", "Paquets .NET. Restaurés à la prochaine compilation (connexion requise).", "Développement", "modere", false, vec![j(&profile, ".nuget\\packages"), j(&local, "NuGet\\v3-cache")], true));
    v.push(cat("gradle", "Cache Gradle", "Dépendances Java/Android. Retéléchargées au besoin.", "Développement", "modere", false, vec![j(&profile, ".gradle\\caches")], true));

    // Actions système natives
    let mut rb = cat("recycle_bin", "Corbeille", "Éléments déjà supprimés, encore présents sur le disque.", "Système", "sur", false, vec![], false);
    rb.action = Some("recycle_bin".into());
    v.push(rb);
    let mut hib = cat("hibernation", "Fichier d'hibernation (hiberfil.sys)", "Désactive l'hibernation et libère le fichier (≈ 40 % de la RAM). Le démarrage rapide est aussi désactivé.", "Système", "modere", true, vec![], false);
    hib.action = Some("hibernation".into());
    v.push(hib);
    let mut cc = cat("component_cleanup", "Composants Windows obsolètes (WinSxS)", "Outil natif DISM : retire les anciennes versions de composants. Peut durer plusieurs minutes.", "Système", "sur", true, vec![], false);
    cc.action = Some("component_cleanup".into());
    v.push(cc);

    // Ne garde que ce qui existe
    v.into_iter()
        .map(|mut c| {
            c.targets.retain(|t| exists(t));
            c
        })
        .filter(|c| c.action.is_some() || !c.targets.is_empty())
        .collect()
}

pub fn measure_all(progress: Option<&scan::Progress>) -> Vec<CategoryResult> {
    let cats = categories();
    let mut res: Vec<CategoryResult> = cats
        .into_par_iter()
        .map(|c| {
            if progress.map(|p| p.cancelled()).unwrap_or(false) {
                return CategoryResult { cat: c, size: 0, files: 0, size_known: true };
            }
            match c.action.as_deref() {
                Some("recycle_bin") => {
                    let (s, n) = sys::recycle_bin_size();
                    CategoryResult { cat: c, size: s, files: n, size_known: true }
                }
                Some("hibernation") => {
                    let drive = env("SystemDrive").unwrap_or_else(|| "C:".into());
                    let s = std::fs::metadata(format!("{}\\hiberfil.sys", drive)).map(|m| m.len()).unwrap_or(0);
                    CategoryResult { cat: c, size: s, files: u64::from(s > 0), size_known: true }
                }
                Some(_) => CategoryResult { cat: c, size: 0, files: 0, size_known: false },
                None => {
                    let (mut s, mut f) = (0, 0);
                    let fallback = scan::Progress::default();
                    for t in &c.targets {
                        let (a, b) = scan::measure_with(t, progress.unwrap_or(&fallback));
                        s += a;
                        f += b;
                    }
                    CategoryResult { cat: c, size: s, files: f, size_known: true }
                }
            }
        })
        .collect();
    res.sort_by(|a, b| b.size.cmp(&a.size));
    res
}

/// Nettoie une catégorie par son identifiant.
pub fn clean(id: &str, opts: DeleteOptions, progress: &DelProgress) -> Result<DeleteReport, String> {
    let c = categories().into_iter().find(|c| c.id == id).ok_or("Catégorie inconnue")?;
    let mut rep = DeleteReport::default();
    if let Ok(mut l) = progress.label.lock() {
        *l = c.name.clone();
    }
    let t0 = std::time::Instant::now();
    let r = clean_inner(&c, opts, progress, &mut rep);
    // Les actions système (corbeille, hibernation) ne passent pas par le compteur de fichiers : on l'alimente ici,
    // pour que la barre de progression avance aussi pour elles.
    if c.action.is_some() {
        progress.bytes.fetch_add(rep.freed_bytes, std::sync::atomic::Ordering::Relaxed);
        rep.elapsed_ms = t0.elapsed().as_millis();
    }
    r.map(|_| rep)
}

fn clean_inner(c: &Category, opts: DeleteOptions, progress: &DelProgress, rep: &mut DeleteReport) -> Result<(), String> {
    match c.action.as_deref() {
        Some("recycle_bin") => {
            let (s, _) = sys::recycle_bin_size();
            sys::empty_recycle_bin()?;
            rep.freed_bytes = s;
            rep.requested = 1;
            rep.succeeded = 1;
        }
        Some("hibernation") => {
            let drive = env("SystemDrive").unwrap_or_else(|| "C:".into());
            let s = std::fs::metadata(format!("{}\\hiberfil.sys", drive)).map(|m| m.len()).unwrap_or(0);
            sys::disable_hibernation()?;
            rep.freed_bytes = s;
            rep.requested = 1;
            rep.succeeded = 1;
        }
        Some("component_cleanup") => {
            let out = sys::component_cleanup()?;
            rep.notes.push(out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("Terminé").to_string());
            rep.requested = 1;
            rep.succeeded = 1;
        }
        _ => {
            let o = DeleteOptions { keep_root: c.keep_root, recycle: false, ..opts };
            *rep = cleaner::delete_paths(&c.targets, o, progress);
        }
    }
    Ok(())
}
