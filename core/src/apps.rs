//! Applications installées (registre Windows) avec leur taille réelle ou estimée.

use crate::winfs;
use rayon::prelude::*;
use serde::Serialize;
use winreg::enums::*;
use winreg::RegKey;

#[derive(Serialize, Clone, Debug)]
pub struct AppInfo {
    pub name: String,
    pub publisher: String,
    pub version: String,
    /// JJ/MM/AAAA, vide si inconnue.
    pub install_date: String,
    pub size: u64,
    pub size_measured: bool,
    pub location: String,
    pub uninstall: String,
    /// Dernière ouverture enregistrée par Windows (secondes Unix), 0 si inconnue.
    pub last_used: i64,
    /// Nature de l'application : "dev", "music", "creation", "system" ou "" (usage général).
    /// Les quatre premières sont protégées : jamais proposées comme « inutilisées » par défaut.
    pub kind: &'static str,
    /// Clé de désinstallation du registre (ex. `HKLM\SOFTWARE\…\Uninstall\{GUID}`).
    pub reg_key: String,
    /// Commande de désinstallation silencieuse (sans fenêtres), si elle existe.
    pub quiet_uninstall: String,
}

const MUSIC_WORDS: &[&str] = &[
    "fl studio", "image-line", "ableton", "native instruments", "kontakt", "antares", "auto-tune", "waves ", "izotope", "serum", "xfer", "sugar bytes",
    "effectrix", "tone2", "ample sound", "cableguys", "shaperbox", "soundtoys", "fabfilter", "arturia", "spitfire", "eastwest", "steinberg", "cubase",
    "reaper", "melodyne", "celemony", "slate digital", "universal audio", "bitwig", "presonus", "studio one", "voxengo", "valhalla", "softube",
    "plugin alliance", "vst", "asio", "kilohearts", "nexus", "sylenth", "massive", "omnisphere", "spectrasonics", "u-he", "output ", "loopmasters",
    "splice", "reason studio", "korg", "roland", "yamaha", "novation", "akai", "midi", "synth", "sampler", "drum", "mastering", "piano", "guitar",
    "bass ", "vocal", "beat", "sound", "audio editor", "audacity", "ozone", "neutron", "rx ",
];
const CREATION_WORDS: &[&str] = &[
    "photoshop", "illustrator", "indesign", "premiere", "after effects", "adobe", "affinity", "gimp", "inkscape", "blender", "krita", "coreldraw",
    "canva", "affiche", "lightroom", "davinci", "capcut", "filmora", "obs studio", "handbrake", "paint.net", "figma", "sketch", "xd", "unity hub",
];
const SYSTEM_WORDS: &[&str] = &[
    "driver", "pilote", "redistributable", "runtime", "framework", "vulkan", "directx", "visual c++", "hotfix", "intel", "nvidia", "amd ", "realtek",
    "microsoft edge", "webview", "update", "windows ", "onedrive", "teams", "office", "bluetooth", "wifi", "touchpad", "synaptics", "elan", "hp ", "dell",
    "lenovo", "asus", "acer", "msi ", "support assistant", "chipset",
];

fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            _ => c,
        })
        .collect()
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

fn expand_known_folder(p: &str) -> String {
    let map: [(&str, &str); 8] = [
        ("{6D809377-6AF0-444B-8957-A3773F02200E}", "ProgramW6432"),
        ("{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}", "ProgramFiles(x86)"),
        ("{F1B32785-6FBA-4FCF-9D55-7B8E7F157091}", "LOCALAPPDATA"),
        ("{3EB685DB-65F9-4CF6-A03A-E3EF65729F3D}", "APPDATA"),
        ("{62AB5D82-FDC1-4DC3-A9DD-070D1D495D97}", "ProgramData"),
        ("{5E6C858F-0E22-4760-9AFE-EA3317B67173}", "USERPROFILE"),
        ("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}", "SystemRoot"),
        ("{F38BF404-1D43-42F2-9305-67DE0B28FC23}", "SystemRoot"),
    ];
    for (guid, var) in map {
        if let Some(rest) = p.strip_prefix(guid) {
            let base = std::env::var(var).unwrap_or_default();
            let suffix = if guid.starts_with("{1AC14E77") { format!("\\System32{}", rest) } else { rest.to_string() };
            return format!("{}{}", base, suffix);
        }
    }
    p.to_string()
}

struct Usage {
    exe: Vec<(String, i64)>,
    lnk: Vec<(String, i64)>,
}

/// Historique de lancement tenu par Windows (UserAssist) : exécutables et raccourcis ouverts via l'interface.
fn user_assist() -> Usage {
    let mut u = Usage { exe: Vec::new(), lnk: Vec::new() };
    let Ok(base) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist", KEY_READ) else { return u };
    for guid in base.enum_keys().flatten() {
        let Ok(count) = base.open_subkey_with_flags(format!("{}\\Count", guid), KEY_READ) else { continue };
        for (name, val) in count.enum_values().flatten() {
            let b: &[u8] = &val.bytes;
            if b.len() < 68 {
                continue;
            }
            let ft = u64::from_le_bytes(b[60..68].try_into().unwrap());
            if ft < 116_444_736_000_000_000 {
                continue;
            }
            let t = (ft / 10_000_000) as i64 - 11_644_473_600;
            let path = expand_known_folder(&rot13(&name));
            let l = path.to_lowercase();
            if l.ends_with(".lnk") {
                let stem = l.rsplit('\\').next().unwrap_or("").trim_end_matches(".lnk").to_string();
                u.lnk.push((norm(&stem), t));
            } else {
                u.exe.push((l, t));
            }
        }
    }
    u
}

const DEV_WORDS: &[&str] = &[
    "visual studio", "sdk", ".net", "python", "node.js", "nodejs", "git", "docker", "android", "flutter", "jetbrains", "postgres", "mysql",
    "mongodb", "sql server", "management studio", "azure data studio", "code", "cursor", "antigravity", "laragon", "xampp", "java", "jdk", "gradle",
    "rust", "cmake", "powershell", "redistributable", "webview2", "runtime", "build tools", "heroku", "github", "npm", "yarn", "vscode", "intellij",
    "pycharm", "webstorm", "phpstorm", "postman", "insomnia", "figma", "wireshark", "putty", "winscp", "filezilla", "dbeaver", "heidisql", "unity",
    "unreal", "godot", "qt ", "emulator", "bluestacks", "vmware", "virtualbox", "wsl", "terminal",
];

fn fmt_date(raw: &str) -> String {
    if raw.len() == 8 && raw.chars().all(|c| c.is_ascii_digit()) {
        format!("{}/{}/{}", &raw[6..8], &raw[4..6], &raw[0..4])
    } else {
        String::new()
    }
}

struct Raw {
    name: String,
    publisher: String,
    version: String,
    date: String,
    est_kb: u64,
    location: String,
    uninstall: String,
    icon: String,
    reg_key: String,
    quiet: String,
}

/// Dossier d'installation : `InstallLocation`, sinon celui de l'icône ou du désinstalleur (hors Windows).
fn best_location(r: &Raw) -> String {
    let loc = r.location.trim_end_matches(['\\', '/']).to_string();
    if !loc.is_empty() {
        return loc;
    }
    let win = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()).to_lowercase();
    for c in [r.icon.trim_matches('"').split(',').next().unwrap_or("").to_string(), crate::sys::split_command(&r.uninstall).0] {
        let c = c.replace('/', "\\");
        if let Some((dir, _)) = c.rsplit_once('\\') {
            let l = dir.to_lowercase();
            if dir.len() > 6 && !l.starts_with(&win) && !l.ends_with("\\installer") && matches!(winfs::attrs(&winfs::extended(dir)), Ok(a) if a & 0x10 != 0) {
                return dir.to_string();
            }
        }
    }
    String::new()
}

/// Vrai si l'entrée de désinstallation existe encore dans le registre.
pub fn key_exists(reg_key: &str) -> bool {
    let (hive, path) = match reg_key.split_once('\\') { Some(x) => x, None => return false };
    let root = match hive { "HKLM" => HKEY_LOCAL_MACHINE, "HKCU" => HKEY_CURRENT_USER, _ => return false };
    RegKey::predef(root).open_subkey_with_flags(path, KEY_READ).is_ok()
}

fn read_hive(root: RegKey, hive: &str, path: &str, out: &mut Vec<Raw>) {
    let Ok(base) = root.open_subkey_with_flags(path, KEY_READ) else { return };
    for k in base.enum_keys().flatten() {
        let Ok(key) = base.open_subkey_with_flags(&k, KEY_READ) else { continue };
        let name: String = key.get_value("DisplayName").unwrap_or_default();
        if name.trim().is_empty() {
            continue;
        }
        let sys_comp: u32 = key.get_value("SystemComponent").unwrap_or(0);
        let parent: String = key.get_value("ParentKeyName").unwrap_or_default();
        let release: String = key.get_value("ReleaseType").unwrap_or_default();
        if sys_comp == 1 || !parent.is_empty() || matches!(release.as_str(), "Update" | "Hotfix" | "Security Update" | "Update Rollup") {
            continue;
        }
        // On privilégie le désinstalleur interactif : l'utilisateur voit ce qui est retiré.
        let uninstall: String = key.get_value::<String, _>("UninstallString").unwrap_or_default();
        out.push(Raw {
            name,
            publisher: key.get_value("Publisher").unwrap_or_default(),
            version: key.get_value("DisplayVersion").unwrap_or_default(),
            date: key.get_value("InstallDate").unwrap_or_default(),
            est_kb: key.get_value::<u32, _>("EstimatedSize").map(|v| v as u64).unwrap_or(0),
            location: key.get_value::<String, _>("InstallLocation").unwrap_or_default().trim_matches('"').to_string(),
            uninstall,
            icon: key.get_value::<String, _>("DisplayIcon").unwrap_or_default(),
            reg_key: format!("{}\\{}\\{}", hive, path, k),
            quiet: {
                let q: String = key.get_value::<String, _>("QuietUninstallString").unwrap_or_default();
                let u: String = key.get_value::<String, _>("UninstallString").unwrap_or_default();
                let ul = u.to_lowercase();
                // Les installateurs MSI se désinstallent en silence avec /qn.
                if q.is_empty() && ul.contains("msiexec") && ul.contains("/x") { format!("{} /qn /norestart", u) } else { q }
            },
        });
    }
}

/// `size_of` : fonction qui renvoie la taille d'un dossier si elle est connue (ex. recherche dans l'arbre déjà analysé).
/// `measure_missing` : mesure directement sur le disque quand la taille n'est pas connue.
pub fn list(size_of: &(dyn Fn(&str) -> Option<u64> + Sync), measure_missing: bool) -> Vec<AppInfo> {
    let mut raws = Vec::new();
    read_hive(RegKey::predef(HKEY_LOCAL_MACHINE), "HKLM", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall", &mut raws);
    read_hive(RegKey::predef(HKEY_LOCAL_MACHINE), "HKLM", r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall", &mut raws);
    read_hive(RegKey::predef(HKEY_CURRENT_USER), "HKCU", r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall", &mut raws);

    raws.sort_by(|a, b| (a.name.to_lowercase(), &a.version).cmp(&(b.name.to_lowercase(), &b.version)));
    raws.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name) && a.version == b.version);

    let usage = user_assist();
    let mut apps: Vec<AppInfo> = raws
        .into_par_iter()
        .map(|r| {
            let mut size = r.est_kb * 1024;
            let mut measured = false;
            let loc = best_location(&r);
            // Dossiers d'application connus : InstallLocation, ou dossier de l'icône d'affichage.
            let icon_path = r.icon.trim_matches('"').split(',').next().unwrap_or("").to_lowercase();
            let icon_dir = icon_path.rsplit_once('\\').map(|x| x.0.to_string()).unwrap_or_default();
            let loc_l = loc.to_lowercase();
            let name_n = norm(&r.name);
            let mut last_used = 0i64;
            for (p, t) in &usage.exe {
                let hit = (!loc_l.is_empty() && p.starts_with(&format!("{}\\", loc_l)))
                    || (icon_dir.len() > 12 && !icon_dir.ends_with("system32") && p.starts_with(&format!("{}\\", icon_dir)))
                    || (icon_path.ends_with(".exe") && *p == icon_path);
                if hit {
                    last_used = last_used.max(*t);
                }
            }
            if name_n.len() >= 5 {
                for (stem, t) in &usage.lnk {
                    if stem.len() >= 5 && (name_n.contains(stem.as_str()) || stem.contains(name_n.as_str())) {
                        last_used = last_used.max(*t);
                    }
                }
            }
            let nl = format!("{} {}", r.name.to_lowercase(), r.publisher.to_lowercase());
            // Ordre : le plus protecteur d'abord (musique avant création avant développement avant système).
            let kind: &'static str = if MUSIC_WORDS.iter().any(|w| nl.contains(w)) {
                "music"
            } else if CREATION_WORDS.iter().any(|w| nl.contains(w)) {
                "creation"
            } else if DEV_WORDS.iter().any(|w| nl.contains(w)) {
                "dev"
            } else if SYSTEM_WORDS.iter().any(|w| nl.contains(w)) {
                "system"
            } else {
                ""
            };
            if !loc.is_empty() && winfs::attrs(&winfs::extended(&loc)).is_ok() {
                if let Some(s) = size_of(&loc) {
                    size = s;
                    measured = true;
                } else if measure_missing {
                    size = crate::scan::measure(&loc).0;
                    measured = true;
                }
            }
            AppInfo {
                name: r.name,
                publisher: r.publisher,
                version: r.version,
                install_date: fmt_date(&r.date),
                size,
                size_measured: measured,
                location: loc,
                uninstall: r.uninstall,
                last_used,
                kind,
                reg_key: r.reg_key,
                quiet_uninstall: r.quiet,
            }
        })
        .collect();
    apps.sort_by(|a, b| b.size.cmp(&a.size));
    apps
}

// ───────────────────────── suppression forcée ─────────────────────────

#[derive(Serialize, Clone, Debug)]
pub struct ForcePlan {
    pub location: String,
    pub size: u64,
    pub files: u64,
    pub services: Vec<String>,
    pub processes: Vec<String>,
    pub verdict: crate::protect::Verdict,
}

#[derive(Serialize, Clone, Debug)]
pub struct ForceStep {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct Leftover {
    pub path: String,
    pub size: u64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ForceReport {
    pub steps: Vec<ForceStep>,
    pub freed_bytes: u64,
    pub leftovers: Vec<Leftover>,
    pub failures: Vec<crate::cleaner::Failure>,
}

fn norm_path(p: &str) -> String {
    p.trim().trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Services Windows dont l'exécutable se trouve dans le dossier donné : (nom du service, chemin).
fn services_under(loc_l: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(base) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(r"SYSTEM\CurrentControlSet\Services", KEY_READ) else { return out };
    let prefix = format!("{}\\", loc_l);
    for name in base.enum_keys().flatten() {
        if let Ok(k) = base.open_subkey_with_flags(&name, KEY_READ) {
            if let Ok(ip) = k.get_value::<String, _>("ImagePath") {
                let exe = norm_path(&crate::sys::split_command(&ip.replace('/', "\\")).0);
                if exe.starts_with(&prefix) {
                    out.push((name, ip));
                }
            }
        }
    }
    out
}

fn ps(script: &str, loc: &str) -> String {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("CLR_LOC", loc)
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

const PS_LIST: &str = "$l=$env:CLR_LOC; Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($l,[StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { $_.Name }";
const PS_KILL: &str = "$l=$env:CLR_LOC; Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($l,[StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { try { Stop-Process -Id $_.Id -Force -ErrorAction Stop; $_.Name } catch {} }";

/// Contrôle qu'un dossier peut faire l'objet d'une suppression forcée (jamais Windows ni un dossier vital).
fn check_force_location(loc: &str) -> Result<(), String> {
    let disp = winfs::display_path(loc);
    if !matches!(winfs::attrs(&winfs::extended(&disp)), Ok(a) if a & 0x10 != 0) {
        return Err("Le dossier d'installation est introuvable : la suppression forcée n'a rien à retirer.".into());
    }
    if disp.trim_end_matches('\\').split('\\').count() < 3 {
        return Err("Dossier trop général pour une suppression forcée.".into());
    }
    match crate::protect::check(&disp) {
        crate::protect::Verdict::Interdit(m) => Err(m),
        _ => Ok(()),
    }
}

pub fn force_plan(location: &str) -> ForcePlan {
    let disp = winfs::display_path(location).trim_end_matches('\\').to_string();
    let verdict = match check_force_location(&disp) {
        Ok(()) => crate::protect::check_deep(&disp),
        Err(m) => crate::protect::Verdict::Interdit(m),
    };
    let (size, files) = crate::scan::measure(&disp);
    let loc_l = norm_path(&disp);
    let services = services_under(&loc_l).into_iter().map(|s| s.0).collect();
    let mut processes: Vec<String> = ps(PS_LIST, &format!("{}\\", disp)).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    processes.sort();
    processes.dedup();
    ForcePlan { location: disp, size, files, services, processes, verdict }
}

/// Retire de force une application dont le désinstalleur échoue : processus, services, dossier, entrée du registre.
/// Les données personnelles (AppData…) ne sont PAS supprimées : elles sont seulement listées.
pub fn force_remove(name: &str, location: &str, reg_key: &str, progress: &crate::cleaner::DelProgress) -> Result<ForceReport, String> {
    let disp = winfs::display_path(location).trim_end_matches('\\').to_string();
    check_force_location(&disp)?;
    let elevated = crate::sys::is_elevated();
    let loc_l = norm_path(&disp);
    let mut rep = ForceReport::default();
    let mut steps: Vec<ForceStep> = Vec::new();
    let mut step = |label: &str, ok: bool, detail: String| steps.push(ForceStep { label: label.to_string(), ok, detail });

    // 1. Processus lancés depuis le dossier
    let killed: Vec<String> = ps(PS_KILL, &format!("{}\\", disp)).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
    step("Arrêt des programmes en cours", true, if killed.is_empty() { "Aucun programme en cours".into() } else { killed.join(", ") });

    // 2. Services
    let services = services_under(&loc_l);
    if services.is_empty() {
        step("Services Windows", true, "Aucun service lié".into());
    } else if !elevated {
        step("Services Windows", false, format!("{} service(s) à retirer : droits administrateur requis ({}).", services.len(), services.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(", ")));
    } else {
        let mut done = Vec::new();
        for (svc, _) in &services {
            crate::sys::run_tool("sc.exe", &["stop", svc]);
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let (c, out) = crate::sys::run_tool("sc.exe", &["delete", svc]);
            if c == 0 {
                done.push(svc.clone());
            } else {
                done.push(format!("{} (échec : {})", svc, out.trim().replace('\n', " ")));
            }
        }
        step("Services Windows", true, format!("Arrêtés et supprimés : {}", done.join(", ")));
    }

    // 3. Dossier d'installation (moteur de suppression robuste)
    let opts = crate::cleaner::DeleteOptions { fix_permissions: true, schedule_on_reboot: elevated, ..Default::default() };
    let r = crate::cleaner::delete_paths(std::slice::from_ref(&disp), opts, progress);
    rep.freed_bytes = r.freed_bytes;
    let left = r.failures.len();
    step(
        "Dossier du programme",
        left == 0,
        if left == 0 { format!("{} supprimés", crate::format_size(r.freed_bytes)) } else { format!("{} supprimés, {} élément(s) résistent", crate::format_size(r.freed_bytes), left) },
    );
    if !r.scheduled.is_empty() {
        step("Suppression planifiée", true, format!("{} élément(s) seront supprimés au prochain redémarrage", r.scheduled.len()));
    }
    rep.failures = r.failures;

    // 4. Entrée « Applications et fonctionnalités »
    let reg_ok = (|| {
        let (hive, path) = reg_key.split_once('\\')?;
        if !path.to_lowercase().contains("\\uninstall\\") {
            return None;
        }
        let root = match hive { "HKLM" => HKEY_LOCAL_MACHINE, "HKCU" => HKEY_CURRENT_USER, _ => return None };
        Some(RegKey::predef(root).delete_subkey_all(path).is_ok())
    })();
    step(
        "Entrée dans les Applications Windows",
        reg_ok == Some(true),
        match reg_ok { Some(true) => "Retirée de la liste".into(), Some(false) => "Impossible de la retirer (droits administrateur ?)".into(), None => "Entrée inconnue".into() },
    );

    // 5. Restes éventuels (listés seulement) : dossiers de données au nom de l'application
    let token: String = name.chars().take_while(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
    if token.chars().count() >= 4 {
        for var in ["APPDATA", "LOCALAPPDATA", "ProgramData"] {
            if let Ok(base) = std::env::var(var) {
                let _ = winfs::read_dir(&winfs::extended(&base), |e| {
                    if e.is_dir() && !e.is_reparse() && e.name.to_lowercase().contains(&token) && rep.leftovers.len() < 8 {
                        let p = winfs::join(&base, &e.name);
                        let size = crate::scan::measure(&p).0;
                        rep.leftovers.push(Leftover { path: p, size });
                    }
                });
            }
        }
    }
    rep.steps = steps;
    Ok(rep)
}
