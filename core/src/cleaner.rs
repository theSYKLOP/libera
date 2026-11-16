//! Moteur de suppression robuste. Escalade par paliers :
//!  1. suppression native parallèle sur chemins étendus `\\?\` (noms réservés, points/espaces finaux, chemins longs) ;
//!  2. retrait des attributs lecture seule/système + suppression « POSIX » (contourne les fichiers ouverts avec partage) ;
//!  3. correction des droits (takeown / icacls) puis nouvel essai ;
//!  4. méthodes de secours natives (rd, del, robocopy /MIR) pour les entrées orphelines ou corrompues ;
//!  5. identification des programmes qui verrouillent (Restart Manager) et suppression planifiée au redémarrage.

use crate::protect::{self, Verdict};
use crate::scan;
use crate::sys;
use crate::winfs::{self, wide, Entry};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::RestartManager::*;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(default)]
pub struct DeleteOptions {
    /// Envoyer à la Corbeille au lieu de supprimer définitivement (ne libère pas d'espace tout de suite).
    pub recycle: bool,
    /// Tenter takeown/icacls quand l'accès est refusé.
    pub fix_permissions: bool,
    /// Planifier au prochain redémarrage ce qui reste verrouillé (nécessite les droits administrateur).
    pub schedule_on_reboot: bool,
    /// Ne supprimer que le contenu du dossier, pas le dossier lui-même.
    pub keep_root: bool,
}

#[derive(Default)]
pub struct DelProgress {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    pub cancel: AtomicBool,
    pub current: Mutex<String>,
    /// Élément en cours de traitement (catégorie ou dossier) et numéro d'étape, pour l'interface.
    pub label: Mutex<String>,
    pub step: AtomicU64,
    pub steps: AtomicU64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Failure {
    pub path: String,
    pub code: u32,
    pub reason: String,
    pub hint: String,
    pub locked_by: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct DeleteReport {
    pub requested: usize,
    pub succeeded: usize,
    pub freed_bytes: u64,
    pub files_deleted: u64,
    pub failures: Vec<Failure>,
    pub scheduled: Vec<String>,
    pub notes: Vec<String>,
    pub elapsed_ms: u128,
}

// ───────────────────────── primitives ─────────────────────────

#[repr(C)]
struct DispositionInfoEx {
    flags: u32,
}
const FILE_DISPOSITION_INFO_EX_CLASS: i32 = 21;
const DISPOSE_DELETE: u32 = 0x1;
const DISPOSE_POSIX: u32 = 0x2;
const DISPOSE_IGNORE_READONLY: u32 = 0x10;

/// Suppression « POSIX » : fonctionne même si un autre programme garde le fichier ouvert (avec partage),
/// et ignore l'attribut lecture seule. Le nom disparaît immédiatement.
fn posix_delete(path_ext: &str) -> Result<(), u32> {
    let w = wide(path_ext);
    unsafe {
        let h = CreateFileW(
            w.as_ptr(),
            DELETE | 0x80, // DELETE | FILE_READ_ATTRIBUTES
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return Err(GetLastError());
        }
        let info = DispositionInfoEx { flags: DISPOSE_DELETE | DISPOSE_POSIX | DISPOSE_IGNORE_READONLY };
        let ok = SetFileInformationByHandle(
            h,
            FILE_DISPOSITION_INFO_EX_CLASS,
            &info as *const _ as *const c_void,
            std::mem::size_of::<DispositionInfoEx>() as u32,
        );
        let e = GetLastError();
        CloseHandle(h);
        if ok != 0 { Ok(()) } else { Err(e) }
    }
}

fn delete_file_once(path_ext: &str) -> Result<(), u32> {
    let w = wide(path_ext);
    if unsafe { DeleteFileW(w.as_ptr()) } != 0 {
        Ok(())
    } else {
        Err(winfs::last_error())
    }
}

fn remove_dir_once(path_ext: &str) -> Result<(), u32> {
    let w = wide(path_ext);
    if unsafe { RemoveDirectoryW(w.as_ptr()) } != 0 {
        Ok(())
    } else {
        Err(winfs::last_error())
    }
}

fn remove_file(path_ext: &str) -> Result<(), u32> {
    let mut code = match delete_file_once(path_ext) {
        Ok(()) => return Ok(()),
        Err(c) => c,
    };
    if code == ERROR_FILE_NOT_FOUND || code == ERROR_PATH_NOT_FOUND {
        return Err(code);
    }
    if code == ERROR_ACCESS_DENIED {
        winfs::set_attrs(path_ext, FILE_ATTRIBUTE_NORMAL);
        match delete_file_once(path_ext) {
            Ok(()) => return Ok(()),
            Err(c) => code = c,
        }
    }
    match posix_delete(path_ext) {
        Ok(()) => Ok(()),
        // On garde l'erreur la plus parlante : un verrou prime sur un simple « accès refusé ».
        Err(c) => Err(if code == ERROR_SHARING_VIOLATION { code } else { c }),
    }
}

fn remove_empty_dir(path_ext: &str) -> Result<(), u32> {
    let mut code = match remove_dir_once(path_ext) {
        Ok(()) => return Ok(()),
        Err(c) => c,
    };
    if code == ERROR_DIR_NOT_EMPTY {
        return Err(code);
    }
    if code == ERROR_ACCESS_DENIED {
        winfs::set_attrs(path_ext, FILE_ATTRIBUTE_NORMAL);
        match remove_dir_once(path_ext) {
            Ok(()) => return Ok(()),
            Err(c) => code = c,
        }
    }
    match posix_delete(path_ext) {
        Ok(()) => Ok(()),
        Err(c) => Err(if c == ERROR_DIR_NOT_EMPTY { c } else { code }),
    }
}

// ───────────────────────── parcours de suppression ─────────────────────────

struct Run<'a> {
    p: &'a DelProgress,
    failures: Mutex<Vec<(String, u32)>>,
}

impl Run<'_> {
    fn fail(&self, path_ext: &str, code: u32) {
        self.failures.lock().unwrap().push((path_ext.to_string(), code));
    }
    fn note_file(&self, path_ext: &str, size: u64) {
        let n = self.p.files.fetch_add(1, Ordering::Relaxed);
        self.p.bytes.fetch_add(size, Ordering::Relaxed);
        if n % 200 == 0 {
            if let Ok(mut c) = self.p.current.try_lock() {
                *c = winfs::display_path(path_ext);
            }
        }
    }
}

fn remove_entry(run: &Run, p: &str, attrs: u32, size: u64) {
    if run.p.cancel.load(Ordering::Relaxed) {
        return;
    }
    let is_dir = attrs & FILE_ATTRIBUTE_DIRECTORY != 0;
    if attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        // Lien/jonction : on retire le lien lui-même, jamais sa cible.
        let r = if is_dir { remove_empty_dir(p) } else { remove_file(p) };
        match r {
            Ok(()) => run.note_file(p, 0),
            Err(c) => run.fail(p, c),
        }
    } else if is_dir {
        remove_dir_contents(run, p);
        match remove_empty_dir(p) {
            Ok(()) => {}
            Err(ERROR_DIR_NOT_EMPTY) => {} // déjà signalé par les éléments enfants
            Err(c) => run.fail(p, c),
        }
    } else {
        match remove_file(p) {
            Ok(()) => run.note_file(p, size),
            Err(c) => run.fail(p, c),
        }
    }
}

fn remove_dir_contents(run: &Run, p: &str) {
    let mut entries: Vec<Entry> = Vec::new();
    if let Err(c) = winfs::read_dir(p, |e| entries.push(e)) {
        run.fail(p, c);
        return;
    }
    entries.par_iter().for_each(|e| remove_entry(run, &winfs::join(p, &e.name), e.attrs, e.size));
}

fn run_pass(path_ext: &str, keep_root: bool, progress: &DelProgress) -> Vec<(String, u32)> {
    let run = Run { p: progress, failures: Mutex::new(Vec::new()) };
    let pool = scan::io_pool();
    pool.install(|| match winfs::attrs(path_ext) {
        Err(c) if c == ERROR_FILE_NOT_FOUND || c == ERROR_PATH_NOT_FOUND => {}
        Err(c) => run.fail(path_ext, c),
        Ok(a) => {
            if keep_root && a & FILE_ATTRIBUTE_DIRECTORY != 0 && a & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                remove_dir_contents(&run, path_ext);
            } else {
                remove_entry(&run, path_ext, a, winfs::file_size(path_ext).unwrap_or(0));
            }
        }
    });
    run.failures.into_inner().unwrap()
}

// ───────────────────────── paliers de secours ─────────────────────────

fn cmd_raw(line: &str) -> i32 {
    Command::new("cmd.exe")
        .raw_arg(format!("/D /C {}", line))
        .creation_flags(0x0800_0000)
        .output()
        .map(|o| o.status.code().unwrap_or(-1))
        .unwrap_or(-1)
}

/// Corrige propriétaire et droits (outils natifs takeown / icacls). Dernier recours avant les méthodes de secours.
fn fix_permissions(path_disp: &str) {
    let user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default()
    );
    let elevated = sys::is_elevated();
    let mut take = vec!["/F", path_disp, "/R", "/D", "Y"];
    if elevated {
        take.push("/A");
    }
    sys::run_tool("takeown.exe", &take);
    sys::run_tool("icacls.exe", &[path_disp, "/reset", "/T", "/C", "/Q"]);
    let grant_admin = "*S-1-5-32-544:(OI)(CI)F";
    sys::run_tool("icacls.exe", &[path_disp, "/grant", grant_admin, "/T", "/C", "/Q"]);
    if user.len() > 1 {
        let g = format!("{}:(OI)(CI)F", user);
        sys::run_tool("icacls.exe", &[path_disp, "/grant", &g, "/T", "/C", "/Q"]);
    }
}

/// Méthodes de secours pour les entrées orphelines / noms invalides / dossiers qui « n'existent pas ».
fn fallback_delete(path_disp: &str, is_dir: bool) {
    let ext = winfs::extended(path_disp);
    if is_dir {
        cmd_raw(&format!("rd /s /q \"{}\"", ext));
        if winfs::attrs(&ext).is_ok() {
            // Astuce classique : un miroir vers un dossier vide purge ce que rd n'atteint pas.
            let empty = std::env::temp_dir().join(format!("libera_vide_{}", std::process::id()));
            let _ = std::fs::create_dir_all(&empty);
            sys::run_tool(
                "robocopy.exe",
                &[&empty.to_string_lossy(), &ext, "/MIR", "/R:0", "/W:0", "/NFL", "/NDL", "/NJH", "/NJS", "/NP", "/XJ"],
            );
            let _ = std::fs::remove_dir(&empty);
            cmd_raw(&format!("rd /s /q \"{}\"", ext));
        }
    } else {
        cmd_raw(&format!("del /f /q /a \"{}\"", ext));
    }
}

/// Programmes qui verrouillent un fichier (Restart Manager).
pub fn lockers(path_disp: &str) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut session = 0u32;
        let mut key = [0u16; 33];
        if RmStartSession(&mut session, 0, key.as_mut_ptr()) != 0 {
            return out;
        }
        let w = wide(path_disp);
        let arr = [w.as_ptr()];
        if RmRegisterResources(session, 1, arr.as_ptr(), 0, std::ptr::null(), 0, std::ptr::null()) == 0 {
            let (mut needed, mut count, mut reasons) = (0u32, 0u32, 0u32);
            let r = RmGetList(session, &mut needed, &mut count, std::ptr::null_mut(), &mut reasons);
            if (r == ERROR_MORE_DATA || r == 0) && needed > 0 {
                let mut infos: Vec<RM_PROCESS_INFO> = vec![std::mem::zeroed(); needed as usize];
                count = needed;
                if RmGetList(session, &mut needed, &mut count, infos.as_mut_ptr(), &mut reasons) == 0 {
                    for i in infos.iter().take(count as usize) {
                        let len = i.strAppName.iter().position(|&c| c == 0).unwrap_or(0);
                        let name = winfs::from_wide(&i.strAppName[..len]);
                        out.push(format!("{} (PID {})", name, i.Process.dwProcessId));
                    }
                }
            }
        }
        RmEndSession(session);
    }
    out
}

fn schedule_on_reboot(path_ext: &str) -> bool {
    let w = wide(path_ext);
    unsafe { MoveFileExW(w.as_ptr(), std::ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) != 0 }
}

fn hint_for(code: u32) -> &'static str {
    match code {
        ERROR_ACCESS_DENIED => "Relance l'application en administrateur. Si cela persiste, un antivirus ou un service protège cet élément.",
        ERROR_SHARING_VIOLATION | ERROR_LOCK_VIOLATION => "Ferme le programme qui l'utilise (voir la liste) ou planifie la suppression au redémarrage.",
        ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_INVALID_NAME | ERROR_BAD_PATHNAME | ERROR_CANT_ACCESS_FILE => {
            "Entrée orpheline ou corrompue. Si elle persiste après les méthodes de secours : chkdsk C: /scan, puis chkdsk C: /f au redémarrage."
        }
        ERROR_FILE_CORRUPT | ERROR_DISK_CORRUPT => "Le système de fichiers signale une corruption : lance chkdsk C: /f au prochain redémarrage.",
        ERROR_DIR_NOT_EMPTY => "Un élément à l'intérieur résiste : voir les autres lignes du rapport.",
        ERROR_FILENAME_EXCED_RANGE => "Chemin trop long : déjà géré par les chemins étendus, mais un outil externe l'a bloqué.",
        _ => "Réessaie après un redémarrage, ou en administrateur.",
    }
}

fn to_failure(path_ext: &str, code: u32, with_lockers: bool) -> Failure {
    let disp = winfs::display_path(path_ext);
    Failure {
        locked_by: if with_lockers && (code == ERROR_SHARING_VIOLATION || code == ERROR_LOCK_VIOLATION) { lockers(&disp) } else { Vec::new() },
        reason: winfs::explain(code).to_string(),
        hint: hint_for(code).to_string(),
        code,
        path: disp,
    }
}

// ───────────────────────── API publique ─────────────────────────

/// Aperçu sans rien supprimer : verdict de protection + taille.
#[derive(Serialize)]
pub struct Preview {
    pub path: String,
    pub verdict: Verdict,
    pub size: u64,
    pub files: u64,
    pub is_dir: bool,
}

/// Élément à prévisualiser. Si la taille est déjà connue (arbre analysé), on ne remesure pas : l'aperçu est instantané.
#[derive(Deserialize, Clone, Debug)]
pub struct PreviewReq {
    pub path: String,
    pub size: Option<u64>,
    pub files: Option<u64>,
}

pub fn preview(reqs: &[PreviewReq]) -> Vec<Preview> {
    reqs.par_iter()
        .map(|r| {
            let disp = winfs::display_path(&r.path);
            let ext = winfs::extended(&disp);
            let is_dir = matches!(winfs::attrs(&ext), Ok(a) if a & FILE_ATTRIBUTE_DIRECTORY != 0);
            let (size, files) = match (r.size, r.files) {
                (Some(s), Some(f)) => (s, f),
                (Some(s), None) => (s, if is_dir { 0 } else { 1 }),
                _ => scan::measure(&disp),
            };
            Preview { verdict: protect::check_deep(&disp), path: disp, size, files, is_dir }
        })
        .collect()
}

pub fn delete_paths(paths: &[String], opts: DeleteOptions, progress: &DelProgress) -> DeleteReport {
    let t0 = std::time::Instant::now();
    // Les compteurs sont partagés par toute l'opération (plusieurs appels successifs) :
    // le rapport ne doit compter que CE qui s'est passé pendant cet appel, sinon les totaux se cumulent.
    let bytes0 = progress.bytes.load(Ordering::Relaxed);
    let files0 = progress.files.load(Ordering::Relaxed);
    let mut report = DeleteReport { requested: paths.len(), ..Default::default() };

    for raw in paths {
        if progress.cancel.load(Ordering::Relaxed) {
            report.notes.push("Opération annulée.".into());
            break;
        }
        let disp = winfs::display_path(raw).trim_end_matches('\\').to_string();
        let disp = if disp.len() == 2 { format!("{}\\", disp) } else { disp };

        if let Verdict::Interdit(m) = protect::check(&disp) {
            report.failures.push(Failure {
                path: disp,
                code: 0,
                reason: "Protégé par Libéra".into(),
                hint: m,
                locked_by: Vec::new(),
            });
            continue;
        }
        let ext = winfs::extended(&disp);
        let attr = winfs::attrs(&ext);
        let is_dir = matches!(attr, Ok(a) if a & FILE_ATTRIBUTE_DIRECTORY != 0);

        if opts.recycle {
            let (sz, _) = scan::measure(&disp);
            match sys::send_to_recycle_bin(&disp) {
                Ok(()) => {
                    report.succeeded += 1;
                    report.notes.push(format!("« {} » envoyé à la Corbeille (l'espace sera libéré en la vidant).", disp));
                    let _ = sz;
                }
                Err(c) => report.failures.push(Failure {
                    path: disp,
                    code: c,
                    reason: "Impossible d'envoyer à la Corbeille".into(),
                    hint: "Essaie la suppression définitive.".into(),
                    locked_by: Vec::new(),
                }),
            }
            continue;
        }

        let before_files = progress.files.load(Ordering::Relaxed);
        let mut fails = run_pass(&ext, opts.keep_root, progress);

        // Palier 3 : droits
        if opts.fix_permissions && fails.iter().any(|(_, c)| *c == ERROR_ACCESS_DENIED) {
            fix_permissions(&disp);
            fails = run_pass(&ext, opts.keep_root, progress);
        }

        // Palier 4 : méthodes de secours pour les entrées orphelines/corrompues
        let ghostly = |c: u32| {
            matches!(c, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND | ERROR_INVALID_NAME | ERROR_BAD_PATHNAME | ERROR_CANT_ACCESS_FILE | ERROR_DIR_NOT_EMPTY | ERROR_ACCESS_DENIED)
        };
        let target_missing = matches!(attr, Err(c) if c == ERROR_FILE_NOT_FOUND || c == ERROR_PATH_NOT_FOUND);
        if !opts.keep_root && (fails.iter().any(|(_, c)| ghostly(*c)) || (target_missing && report.failures.is_empty() && fails.is_empty() && winfs::attrs(&ext).is_ok())) {
            let (rest_size, _) = scan::measure(&disp);
            fallback_delete(&disp, is_dir);
            if winfs::attrs(&ext).is_err() {
                progress.bytes.fetch_add(rest_size, Ordering::Relaxed);
                fails.clear();
            } else {
                fails = run_pass(&ext, opts.keep_root, progress);
            }
        } else if opts.keep_root && fails.iter().any(|(_, c)| ghostly(*c)) {
            // On ne peut pas utiliser rd sur la racine (on la garde) : on traite chaque élément en échec.
            let items: Vec<String> = fails.iter().map(|(p, _)| p.clone()).collect();
            for it in items {
                let d = winfs::display_path(&it);
                let dir = matches!(winfs::attrs(&it), Ok(a) if a & FILE_ATTRIBUTE_DIRECTORY != 0);
                fallback_delete(&d, dir);
            }
            fails = run_pass(&ext, opts.keep_root, progress);
        }

        // Palier 5 : verrous
        if !fails.is_empty() && opts.schedule_on_reboot {
            let mut still = Vec::new();
            for (p, c) in fails {
                if (c == ERROR_SHARING_VIOLATION || c == ERROR_LOCK_VIOLATION || c == ERROR_ACCESS_DENIED) && schedule_on_reboot(&p) {
                    report.scheduled.push(winfs::display_path(&p));
                } else {
                    still.push((p, c));
                }
            }
            fails = still;
        }

        if fails.is_empty() {
            report.succeeded += 1;
        } else {
            // On limite le détail pour ne pas inonder le rapport.
            let total = fails.len();
            for (p, c) in fails.into_iter().take(50) {
                report.failures.push(to_failure(&p, c, true));
            }
            if total > 50 {
                report.notes.push(format!("{} autres éléments ont aussi résisté dans « {} ».", total - 50, disp));
            }
        }
        let _ = before_files;
    }

    report.freed_bytes = progress.bytes.load(Ordering::Relaxed).saturating_sub(bytes0);
    report.files_deleted = progress.files.load(Ordering::Relaxed).saturating_sub(files0);
    report.elapsed_ms = t0.elapsed().as_millis();
    report
}
