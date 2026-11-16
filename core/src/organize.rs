//! Rangement du Bureau par domaines, sans rien casser :
//!  • les raccourcis (.lnk) du Bureau qui pointent vers un élément déplacé sont RÉPARÉS ;
//!  • les projets de code ne sont pas déplacés par défaut (leurs chemins peuvent être utilisés ailleurs) ;
//!  • chaque rangement écrit un JOURNAL qui permet de tout annuler.

use crate::{scan, stale, winfs};
use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

#[derive(Serialize, Clone, Debug)]
pub struct DesktopItem {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub files: u64,
    pub modified: i64,
    pub domain: String,
    /// "projet", "raccourci" ou "element"
    pub kind: &'static str,
    pub movable: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Move {
    pub name: String,
    pub from: String,
    pub to: String,
    pub domain: String,
    /// "faible" ou "projet" (déplacer un projet peut casser des chemins)
    pub risk: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShortcutFix {
    pub lnk: String,
    pub old_target: String,
    pub new_target: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Plan {
    pub root: String,
    pub moves: Vec<Move>,
    pub fixes: Vec<ShortcutFix>,
    pub kept: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ApplyReport {
    pub moved: Vec<Move>,
    pub failed: Vec<(String, String)>,
    pub fixed_shortcuts: usize,
    pub journal: String,
}

/// Dossiers « Bureau » de l'utilisateur (le Bureau peut être redirigé vers OneDrive).
pub fn desktop_roots() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(p) = std::env::var("USERPROFILE") {
        let d = winfs::join(&p, "Desktop");
        if winfs::attrs(&winfs::extended(&d)).is_ok() {
            v.push(d);
        }
        let _ = winfs::read_dir(&winfs::extended(&p), |e| {
            if e.is_dir() && e.name.to_lowercase().starts_with("onedrive") {
                for n in ["Desktop", "Bureau"] {
                    let d = winfs::join(&winfs::join(&p, &e.name), n);
                    if winfs::attrs(&winfs::extended(&d)).is_ok() {
                        v.push(d);
                    }
                }
            }
        });
    }
    v
}

fn ps_lines(script: &str, envs: &[(&str, &str)]) -> Vec<String> {
    let mut c = Command::new("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-Command", script]).creation_flags(0x0800_0000);
    for (k, v) in envs {
        c.env(k, v);
    }
    c.output().map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()).unwrap_or_default()
}

/// Cibles des raccourcis (.lnk) du dossier : (chemin du .lnk, cible).
fn shortcut_targets(root: &str) -> Vec<(String, String)> {
    let script = "$sh = New-Object -ComObject WScript.Shell; Get-ChildItem -LiteralPath $env:CLR_DIR -Filter *.lnk -Force -ErrorAction SilentlyContinue | ForEach-Object { $l = $sh.CreateShortcut($_.FullName); $_.FullName + '|' + $l.TargetPath }";
    ps_lines(script, &[("CLR_DIR", root)])
        .into_iter()
        .filter_map(|l| l.split_once('|').map(|(a, b)| (a.to_string(), b.to_string())))
        .collect()
}

pub fn scan_desktop(root: &str, progress: &scan::Progress) -> Vec<DesktopItem> {
    let mut out = Vec::new();
    let mut entries = Vec::new();
    let _ = winfs::read_dir(&winfs::extended(root), |e| entries.push(e));
    for e in entries {
        if progress.cancelled() {
            break;
        }
        let l = e.name.to_lowercase();
        if l == "desktop.ini" {
            continue;
        }
        let path = winfs::join(root, &e.name);
        if e.is_dir() && !e.is_reparse() {
            let ext = winfs::extended(&path);
            let (domain, project) = stale::domain_of_dir(&ext, &e.name);
            let (size, files) = scan::measure(&path);
            out.push(DesktopItem { name: e.name, path, is_dir: true, size, files, modified: e.mtime, domain: domain.into(), kind: if project { "projet" } else { "element" }, movable: true });
        } else if l.ends_with(".lnk") || l.ends_with(".url") {
            out.push(DesktopItem { name: e.name, path, is_dir: false, size: e.size, files: 1, modified: e.mtime, domain: "Raccourcis".into(), kind: "raccourci", movable: false });
        } else if !e.is_dir() {
            out.push(DesktopItem { domain: stale::domain_of_file(&e.name).into(), name: e.name, path, is_dir: false, size: e.size, files: 1, modified: e.mtime, kind: "element", movable: true });
        }
    }
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}

fn clean_domain(d: &str) -> Option<String> {
    let t: String = d.trim().chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect();
    let t = t.trim().trim_end_matches('.').to_string();
    if t.is_empty() || t.len() > 40 { None } else { Some(t) }
}

/// Construit un plan à partir de (nom d'élément, dossier-domaine). Rien n'est déplacé ici.
pub fn build_plan(root: &str, assignments: &[(String, String)]) -> Result<Plan, String> {
    let root = winfs::display_path(root).trim_end_matches('\\').to_string();
    let mut plan = Plan { root: root.clone(), ..Default::default() };
    let shortcuts = shortcut_targets(&root);
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (name, domain) in assignments {
        let Some(dom) = clean_domain(domain) else {
            plan.kept.push((name.clone(), "Nom de dossier cible invalide".into()));
            continue;
        };
        if name.contains(['\\', '/']) || name.is_empty() {
            plan.kept.push((name.clone(), "Nom d'élément invalide".into()));
            continue;
        }
        let low = name.to_lowercase();
        if low.ends_with(".lnk") || low.ends_with(".url") || low == "desktop.ini" {
            plan.kept.push((name.clone(), "Les raccourcis restent sur le Bureau".into()));
            continue;
        }
        let from = winfs::join(&root, name);
        let Ok(attrs) = winfs::attrs(&winfs::extended(&from)) else {
            plan.kept.push((name.clone(), "Introuvable sur le Bureau".into()));
            continue;
        };
        if name.eq_ignore_ascii_case(&dom) {
            plan.kept.push((name.clone(), "C'est déjà un dossier de rangement".into()));
            continue;
        }
        if let crate::protect::Verdict::Interdit(m) = crate::protect::check(&from) {
            plan.kept.push((name.clone(), m));
            continue;
        }
        let is_dir = attrs & 0x10 != 0;
        let project = is_dir && stale::domain_of_dir(&winfs::extended(&from), name).1;
        let mut to = winfs::join(&winfs::join(&root, &dom), name);
        let mut i = 2;
        while winfs::attrs(&winfs::extended(&to)).is_ok() || !used.insert(to.to_lowercase()) {
            let stem = name.to_string();
            to = winfs::join(&winfs::join(&root, &dom), &format!("{} ({})", stem, i));
            i += 1;
        }
        let size = if is_dir { scan::measure(&from).0 } else { winfs::file_size(&winfs::extended(&from)).unwrap_or(0) };
        plan.moves.push(Move { name: name.clone(), from, to, domain: dom, risk: if project { "projet".into() } else { "faible".into() }, size });
    }
    // Raccourcis du Bureau à réparer : ceux dont la cible est dans un élément déplacé.
    for (lnk, target) in &shortcuts {
        let t = target.replace('/', "\\");
        for m in &plan.moves {
            let f = m.from.to_lowercase();
            let tl = t.to_lowercase();
            if tl == f || tl.starts_with(&format!("{}\\", f)) {
                let new_target = format!("{}{}", m.to, &t[m.from.len()..]);
                plan.fixes.push(ShortcutFix { lnk: lnk.clone(), old_target: t.clone(), new_target });
                break;
            }
        }
    }
    if plan.moves.iter().any(|m| m.risk == "projet") {
        plan.warnings.push("Déplacer un projet de code peut casser des chemins utilisés ailleurs (éditeur, scripts, services). Ces éléments ne sont pas cochés par défaut.".into());
    }
    plan.warnings.push("Seuls les raccourcis du Bureau sont réparés : ceux du menu Démarrer, de la barre des tâches ou d'autres dossiers ne sont pas modifiés.".into());
    Ok(plan)
}

fn journal_dir() -> PathBuf {
    let base = std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
    base.join("Libera").join("journal")
}

/// Applique les déplacements retenus, répare les raccourcis concernés et écrit le journal.
pub fn apply(moves: &[Move], fixes: &[ShortcutFix]) -> ApplyReport {
    let mut rep = ApplyReport::default();
    for m in moves {
        if let crate::protect::Verdict::Interdit(r) = crate::protect::check(&m.from) {
            rep.failed.push((m.name.clone(), r));
            continue;
        }
        if let Some(parent) = std::path::Path::new(&winfs::extended(&m.to)).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::rename(winfs::extended(&m.from), winfs::extended(&m.to)) {
            Ok(()) => rep.moved.push(m.clone()),
            Err(e) => rep.failed.push((m.name.clone(), format!("Déplacement impossible (élément utilisé par un programme ?) : {}", e))),
        }
    }
    let applicable: Vec<&ShortcutFix> = fixes.iter().filter(|f| rep.moved.iter().any(|m| f.old_target.to_lowercase().starts_with(&m.from.to_lowercase()))).collect();
    rep.fixed_shortcuts = patch_shortcuts(&applicable.iter().map(|f| (f.lnk.clone(), f.new_target.clone())).collect::<Vec<_>>());
    // journal
    let dir = journal_dir();
    let _ = std::fs::create_dir_all(&dir);
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let file = dir.join(format!("rangement-{}.json", ts));
    let body = serde_json::json!({ "moves": rep.moved, "fixes": applicable });
    if std::fs::write(&file, body.to_string()).is_ok() {
        rep.journal = file.to_string_lossy().to_string();
    }
    rep
}

fn patch_shortcuts(pairs: &[(String, String)]) -> usize {
    if pairs.is_empty() {
        return 0;
    }
    let f = std::env::temp_dir().join(format!("libera-fix-{}.json", std::process::id()));
    let json: Vec<serde_json::Value> = pairs.iter().map(|(l, t)| serde_json::json!({"lnk": l, "target": t})).collect();
    if std::fs::write(&f, serde_json::Value::Array(json).to_string()).is_err() {
        return 0;
    }
    let script = "$sh = New-Object -ComObject WScript.Shell; $n=0; foreach ($i in (Get-Content -LiteralPath $env:CLR_FIX -Raw | ConvertFrom-Json)) { try { $l = $sh.CreateShortcut($i.lnk); $old = $l.TargetPath; $l.TargetPath = $i.target; if ($l.WorkingDirectory -and $l.WorkingDirectory.StartsWith($old, [StringComparison]::OrdinalIgnoreCase)) { $l.WorkingDirectory = $i.target }; $l.Save(); $n++ } catch {} }; $n";
    let r = ps_lines(script, &[("CLR_FIX", &f.to_string_lossy())]);
    let _ = std::fs::remove_file(&f);
    r.last().and_then(|s| s.parse().ok()).unwrap_or(0)
}

/// Dernier journal de rangement, s'il existe.
pub fn last_journal() -> Option<String> {
    let mut files: Vec<_> = std::fs::read_dir(journal_dir()).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect();
    files.sort();
    files.pop().map(|p| p.to_string_lossy().to_string())
}

/// Annule un rangement : remet chaque élément à sa place et restaure les raccourcis.
pub fn undo(journal: &str) -> Result<ApplyReport, String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(journal).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let moves: Vec<Move> = serde_json::from_value(v["moves"].clone()).map_err(|e| e.to_string())?;
    let fixes: Vec<ShortcutFix> = serde_json::from_value(v["fixes"].clone()).unwrap_or_default();
    let mut rep = ApplyReport::default();
    for m in moves.iter().rev() {
        if winfs::attrs(&winfs::extended(&m.from)).is_ok() {
            rep.failed.push((m.name.clone(), "Un élément du même nom existe déjà à l'emplacement d'origine".into()));
            continue;
        }
        match std::fs::rename(winfs::extended(&m.to), winfs::extended(&m.from)) {
            Ok(()) => rep.moved.push(m.clone()),
            Err(e) => rep.failed.push((m.name.clone(), format!("{}", e))),
        }
    }
    rep.fixed_shortcuts = patch_shortcuts(&fixes.iter().map(|f| (f.lnk.clone(), f.old_target.clone())).collect::<Vec<_>>());
    // retire les dossiers de domaine devenus vides
    for m in &moves {
        if let Some(p) = std::path::Path::new(&m.to).parent() {
            let _ = std::fs::remove_dir(winfs::extended(&p.to_string_lossy()));
        }
    }
    let _ = std::fs::remove_file(journal);
    Ok(rep)
}
