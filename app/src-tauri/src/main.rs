#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;

use libera_core::{
    self as core,
    apps::AppInfo,
    catalog::{self, CategoryResult},
    cleaner::{self, DelProgress, DeleteOptions, DeleteReport, Preview},
    dupes::DupGroup,
    protect::{self, Verdict},
    scan::{self, ChildInfo, FileEntry, Progress, ScanResult},
    stale::{self, StaleOptions, StaleReport},
    sys::{self, DiskInfo},
    winfs, Mode,
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

#[derive(Default)]
struct Stored {
    disk: Option<DiskInfo>,
    junk: Vec<CategoryResult>,
    apps: Vec<AppInfo>,
    scan: Option<ScanResult>,
    dupes: Vec<DupGroup>,
    mode: Option<Mode>,
    root: String,
    elapsed_ms: u128,
    notice: Option<String>,
}

#[derive(Default)]
struct AppState {
    progress: Progress,
    del: DelProgress,
    phase: Mutex<String>,
    busy: AtomicBool,
    data: Mutex<Stored>,
    /// Assistant IA : copie du panier, processus Claude Code en cours, serveur MCP local, objectif en Go.
    basket: Mutex<Vec<agent::BItem>>,
    /// Demandes d'autorisation en attente (réponse par bouton dans le chat), archivage en cours,
    /// sources déjà archivées et vérifiées, plan de rangement du Bureau en attente.
    perms: Mutex<std::collections::HashMap<String, agent::Perm>>,
    archive: Mutex<Option<agent::ArchiveJob>>,
    archived: Mutex<Vec<String>>,
    plan: Mutex<Option<core::organize::Plan>>,
    agent: Mutex<Option<std::process::Child>>,
    mcp: Mutex<Option<agent::McpInfo>>,
    goal: std::sync::atomic::AtomicU64,
}

type St<'a> = State<'a, Arc<AppState>>;

// ───────────── ticker : émet la progression vers l'interface ─────────────

struct Ticker {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Ticker {
    fn spawn(f: impl Fn() + Send + 'static) -> Ticker {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::spawn(move || {
            while !s2.load(Ordering::Relaxed) {
                f();
                std::thread::sleep(Duration::from_millis(120));
            }
        });
        Ticker { stop, handle: Some(handle) }
    }
    fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

#[derive(Serialize, Clone)]
struct ProgressPayload {
    files: u64,
    dirs: u64,
    bytes: u64,
    /// Quantité attendue pour l'étape en cours (0 = inconnue). Sert au pourcentage.
    expected: u64,
    denied: u64,
    current: String,
    phase: String,
}

#[derive(Serialize, Clone)]
struct DelPayload {
    files: u64,
    bytes: u64,
    current: String,
    label: String,
    step: u64,
    steps: u64,
}

// ───────────── commandes ─────────────

#[derive(Serialize)]
struct Status {
    elevated: bool,
    drives: Vec<DiskInfo>,
    has_analysis: bool,
    busy: bool,
}

#[tauri::command]
fn get_status(st: St) -> Status {
    Status {
        elevated: sys::is_elevated(),
        drives: sys::fixed_drives(),
        has_analysis: st.data.lock().unwrap().mode.is_some(),
        busy: st.busy.load(Ordering::Relaxed),
    }
}

#[derive(Serialize, Clone)]
struct Summary {
    mode: Mode,
    root: String,
    disk: Option<DiskInfo>,
    scanned: bool,
    total_size: u64,
    total_files: u64,
    total_dirs: u64,
    denied: u64,
    cloud_bytes: u64,
    elapsed_ms: u128,
    junk_total: u64,
    junk_count: usize,
    apps_count: usize,
    dupes_groups: usize,
    dupes_wasted: u64,
    big_files: usize,
    rebuildable_total: u64,
    cancelled: bool,
    notice: Option<String>,
}

fn summarize(d: &Stored, cancelled: bool) -> Option<Summary> {
    let mode = d.mode?;
    let (mut ts, mut tf, mut td, mut den, mut cloud, mut big, mut reb) = (0, 0, 0, 0, 0, 0, 0);
    if let Some(s) = &d.scan {
        ts = s.nodes[0].size;
        tf = s.nodes[0].files;
        td = s.nodes.len() as u64;
        den = s.denied_dirs;
        cloud = s.cloud_bytes;
        big = s.big_files.iter().filter(|f| f.size >= core::BIG_FILE_MIN).count();
        reb = s.rebuildable(50 << 20).iter().map(|c| c.size).sum();
    }
    Some(Summary {
        mode,
        root: d.root.clone(),
        disk: d.disk.clone(),
        scanned: d.scan.is_some(),
        total_size: ts,
        total_files: tf,
        total_dirs: td,
        denied: den,
        cloud_bytes: cloud,
        elapsed_ms: d.elapsed_ms,
        junk_total: d.junk.iter().map(|j| j.size).sum(),
        junk_count: d.junk.len(),
        apps_count: d.apps.len(),
        dupes_groups: d.dupes.len(),
        dupes_wasted: d.dupes.iter().map(|g| g.wasted).sum(),
        big_files: big,
        rebuildable_total: reb,
        cancelled,
        notice: d.notice.clone(),
    })
}

#[tauri::command]
fn start_analysis(app: AppHandle, st: St, mode: Mode, root: String) -> Result<(), String> {
    if st.busy.swap(true, Ordering::SeqCst) {
        return Err("Une analyse est déjà en cours.".into());
    }
    let p = &st.progress;
    for a in [&p.files, &p.dirs, &p.bytes, &p.denied] {
        a.store(0, Ordering::Relaxed);
    }
    p.cancel.store(false, Ordering::Relaxed);
    p.expected.store(0, Ordering::Relaxed);
    *st.phase.lock().unwrap() = "Préparation".into();
    let _ = app.emit("analysis-started", mode);

    let state = st.inner().clone();
    std::thread::spawn(move || {
        let (s2, a2) = (state.clone(), app.clone());
        let ticker = Ticker::spawn(move || {
            let p = &s2.progress;
            let _ = a2.emit(
                "progress",
                ProgressPayload {
                    files: p.files.load(Ordering::Relaxed),
                    dirs: p.dirs.load(Ordering::Relaxed),
                    bytes: p.bytes.load(Ordering::Relaxed),
                    denied: p.denied.load(Ordering::Relaxed),
                    expected: p.expected.load(Ordering::Relaxed),
                    current: p.current.lock().map(|c| c.clone()).unwrap_or_default(),
                    phase: s2.phase.lock().map(|c| c.clone()).unwrap_or_default(),
                },
            );
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            core::analyze(mode, &root, &state.progress, &|ph| *state.phase.lock().unwrap() = ph.to_string())
        }));
        ticker.stop();
        match result {
            Ok(a) => {
                let cancelled = state.progress.cancelled();
                let summary = {
                    let mut d = state.data.lock().unwrap();
                    *d = Stored {
                        disk: a.disk,
                        junk: a.junk,
                        apps: a.apps,
                        scan: a.scan,
                        dupes: a.dupes,
                        mode: Some(mode),
                        root: root.clone(),
                        elapsed_ms: a.elapsed_ms,
                        notice: a.notice,
                    };
                    summarize(&d, cancelled)
                };
                let _ = app.emit("analysis-done", summary);
            }
            Err(_) => {
                let _ = app.emit("analysis-error", "L'analyse a rencontré une erreur inattendue.");
            }
        }
        state.busy.store(false, Ordering::SeqCst);
    });
    Ok(())
}

/// Recherche des fichiers probablement inutiles (voir `stale.rs`). Indépendante de l'analyse principale.
#[tauri::command]
async fn find_stale(app: AppHandle, st: St<'_>, opts: StaleOptions) -> Result<StaleReport, String> {
    if st.busy.swap(true, Ordering::SeqCst) {
        return Err("Une analyse est déjà en cours.".into());
    }
    let state = st.inner().clone();
    let p = &state.progress;
    for a in [&p.files, &p.dirs, &p.bytes, &p.denied] {
        a.store(0, Ordering::Relaxed);
    }
    p.cancel.store(false, Ordering::Relaxed);
    *state.phase.lock().unwrap() = "Recherche des fichiers inutilisés".into();
    let (s2, a2) = (state.clone(), app.clone());
    let ticker = Ticker::spawn(move || {
        let p = &s2.progress;
        let _ = a2.emit(
            "progress",
            ProgressPayload {
                files: p.files.load(Ordering::Relaxed),
                dirs: p.dirs.load(Ordering::Relaxed),
                bytes: p.bytes.load(Ordering::Relaxed),
                denied: p.denied.load(Ordering::Relaxed),
                    expected: p.expected.load(Ordering::Relaxed),
                current: p.current.lock().map(|c| c.clone()).unwrap_or_default(),
                phase: s2.phase.lock().map(|c| c.clone()).unwrap_or_default(),
            },
        );
    });
    let names: Vec<String> = {
        let d = state.data.lock().unwrap();
        d.apps.iter().map(|a| a.name.clone()).collect()
    };
    let s3 = state.clone();
    let rep = tauri::async_runtime::spawn_blocking(move || {
        let names = if names.is_empty() { core::apps::list(&|_| None, false).into_iter().map(|a| a.name).collect() } else { names };
        stale::analyze(opts, &names, &s3.progress)
    })
    .await;
    ticker.stop();
    state.busy.store(false, Ordering::SeqCst);
    rep.map_err(|e| e.to_string())
}

#[tauri::command]
fn cancel(st: St) {
    st.progress.cancel.store(true, Ordering::Relaxed);
    st.del.cancel.store(true, Ordering::Relaxed);
}

#[tauri::command]
fn get_summary(st: St) -> Option<Summary> {
    summarize(&st.data.lock().unwrap(), false)
}

#[derive(Serialize)]
struct Row {
    #[serde(flatten)]
    info: ChildInfo,
    /// Raison pour laquelle l'élément ne peut pas être supprimé (None = suppressible).
    locked: Option<String>,
    warning: Option<String>,
}

fn verdict_fields(path: &str) -> (Option<String>, Option<String>) {
    match protect::check(path) {
        Verdict::Ok => (None, None),
        Verdict::Attention(m) => (None, Some(m)),
        Verdict::Interdit(m) => (Some(m), None),
    }
}

#[derive(Serialize)]
struct Crumb {
    id: u32,
    name: String,
}

#[derive(Serialize)]
struct Folder {
    id: u32,
    name: String,
    path: String,
    size: u64,
    files: u64,
    own_size: u64,
    own_files: u32,
    breadcrumb: Vec<Crumb>,
    children: Vec<Row>,
}

#[tauri::command]
fn get_children(st: St, node_id: Option<u32>) -> Result<Folder, String> {
    let d = st.data.lock().unwrap();
    let s = d.scan.as_ref().ok_or("Aucune analyse disponible.")?;
    let id = node_id.unwrap_or(0).min(s.nodes.len() as u32 - 1);
    let n = &s.nodes[id as usize];
    let mut crumbs = Vec::new();
    let mut cur = id;
    while cur != scan::NO_PARENT {
        crumbs.push(Crumb { id: cur, name: s.nodes[cur as usize].name.clone() });
        cur = s.nodes[cur as usize].parent;
    }
    crumbs.reverse();
    let children = s
        .children(id)
        .into_iter()
        .map(|info| {
            let (locked, warning) = verdict_fields(&info.path);
            Row { info, locked, warning }
        })
        .collect();
    Ok(Folder {
        id,
        name: n.name.clone(),
        path: s.path_of(id),
        size: n.size,
        files: n.files,
        own_size: n.own_size,
        own_files: n.own_files,
        breadcrumb: crumbs,
        children,
    })
}

#[derive(Serialize)]
struct FileRow {
    path: String,
    name: String,
    dir: String,
    size: u64,
    mtime: i64,
    locked: Option<String>,
    warning: Option<String>,
}

fn file_row(f: &FileEntry) -> FileRow {
    let disp = winfs::display_path(&f.path);
    let (mut dir, name) = disp.rsplit_once('\\').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((String::new(), disp.clone()));
    if dir.ends_with(':') {
        dir.push('\\');
    }
    let (locked, warning) = verdict_fields(&disp);
    FileRow { path: disp, name, dir, size: f.size, mtime: f.mtime, locked, warning }
}

#[tauri::command]
fn get_folder_files(path: String) -> Vec<FileRow> {
    scan::list_files(&path).iter().take(300).map(file_row).collect()
}

#[tauri::command]
fn get_big_files(st: St, min_mb: u64, older_than_days: Option<i64>, limit: usize) -> Result<Vec<FileRow>, String> {
    let d = st.data.lock().unwrap();
    let s = d.scan.as_ref().ok_or("Aucune analyse disponible.")?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|x| x.as_secs() as i64).unwrap_or(0);
    let min = min_mb << 20;
    Ok(s.big_files
        .iter()
        .filter(|f| f.size >= min)
        .filter(|f| older_than_days.map(|days| f.mtime > 0 && f.mtime < now - days * 86_400).unwrap_or(true))
        .take(limit)
        .map(file_row)
        .collect())
}

#[tauri::command]
fn get_rebuildable(st: St) -> Result<Vec<Row>, String> {
    let d = st.data.lock().unwrap();
    let s = d.scan.as_ref().ok_or("Aucune analyse disponible.")?;
    Ok(s.rebuildable(50 << 20)
        .into_iter()
        .map(|info| {
            let (locked, warning) = verdict_fields(&info.path);
            Row { info, locked, warning }
        })
        .collect())
}

#[tauri::command]
fn get_junk(st: St) -> Vec<CategoryResult> {
    st.data.lock().unwrap().junk.clone()
}

#[tauri::command]
fn get_apps(st: St) -> Vec<AppInfo> {
    st.data.lock().unwrap().apps.clone()
}

#[tauri::command]
fn get_dupes(st: St) -> Vec<DupGroup> {
    st.data.lock().unwrap().dupes.clone()
}

#[tauri::command]
async fn preview_delete(items: Vec<cleaner::PreviewReq>) -> Result<Vec<Preview>, String> {
    tauri::async_runtime::spawn_blocking(move || cleaner::preview(&items)).await.map_err(|e| e.to_string())
}

fn refresh_disk(d: &mut Stored) {
    let root = if d.root.is_empty() { "C:\\".to_string() } else { format!("{}\\", d.root.trim_end_matches('\\')) };
    d.disk = sys::disk_info(&root);
}

/// Prépare les compteurs d'une opération de suppression (plusieurs étapes possibles).
fn reset_del(state: &AppState, steps: u64) {
    state.del.cancel.store(false, Ordering::Relaxed);
    state.del.files.store(0, Ordering::Relaxed);
    state.del.bytes.store(0, Ordering::Relaxed);
    state.del.step.store(0, Ordering::Relaxed);
    state.del.steps.store(steps, Ordering::Relaxed);
    if let Ok(mut l) = state.del.label.lock() {
        l.clear();
    }
    if let Ok(mut c) = state.del.current.lock() {
        c.clear();
    }
}

fn del_ticker(app: AppHandle, state: Arc<AppState>) -> Ticker {
    Ticker::spawn(move || {
        let _ = app.emit(
            "delete-progress",
            DelPayload {
                files: state.del.files.load(Ordering::Relaxed),
                bytes: state.del.bytes.load(Ordering::Relaxed),
                current: state.del.current.lock().map(|c| c.clone()).unwrap_or_default(),
                label: state.del.label.lock().map(|c| c.clone()).unwrap_or_default(),
                step: state.del.step.load(Ordering::Relaxed),
                steps: state.del.steps.load(Ordering::Relaxed),
            },
        );
    })
}

fn merge_report(total: &mut DeleteReport, r: DeleteReport) {
    total.requested += r.requested;
    total.succeeded += r.succeeded;
    total.freed_bytes += r.freed_bytes;
    total.files_deleted += r.files_deleted;
    total.failures.extend(r.failures);
    total.scheduled.extend(r.scheduled);
    total.notes.extend(r.notes);
}

#[tauri::command]
async fn delete_paths(app: AppHandle, st: St<'_>, paths: Vec<String>, opts: DeleteOptions) -> Result<DeleteReport, String> {
    let state = st.inner().clone();
    reset_del(&state, paths.len() as u64);
    let ticker = del_ticker(app.clone(), state.clone());
    let (s3, p3) = (state.clone(), paths.clone());
    let rep = tauri::async_runtime::spawn_blocking(move || {
        let t0 = std::time::Instant::now();
        let mut total = DeleteReport::default();
        for (i, p) in p3.iter().enumerate() {
            if s3.del.cancel.load(Ordering::Relaxed) {
                total.notes.push("Opération arrêtée avant la fin.".into());
                break;
            }
            s3.del.step.store(i as u64 + 1, Ordering::Relaxed);
            if let Ok(mut l) = s3.del.label.lock() {
                *l = p.rsplit('\\').next().unwrap_or(p).to_string();
            }
            merge_report(&mut total, cleaner::delete_paths(std::slice::from_ref(p), opts, &s3.del));
        }
        total.elapsed_ms = t0.elapsed().as_millis();
        total
    })
    .await;
    ticker.stop();
    let rep = rep.map_err(|e| e.to_string())?;

    let mut d = state.data.lock().unwrap();
    if let Some(s) = d.scan.as_mut() {
        for p in &paths {
            if winfs::attrs(&winfs::extended(p)).is_err() {
                s.forget(p, 0);
            }
        }
    }
    refresh_disk(&mut d);
    Ok(rep)
}

#[tauri::command]
async fn clean_categories(app: AppHandle, st: St<'_>, ids: Vec<String>, opts: DeleteOptions) -> Result<DeleteReport, String> {
    let state = st.inner().clone();
    reset_del(&state, ids.len() as u64);
    let ticker = del_ticker(app.clone(), state.clone());
    let s3 = state.clone();
    let rep = tauri::async_runtime::spawn_blocking(move || {
        let t0 = std::time::Instant::now();
        let mut total = DeleteReport::default();
        for (i, id) in ids.iter().enumerate() {
            if s3.del.cancel.load(Ordering::Relaxed) {
                total.notes.push("Opération arrêtée avant la fin.".into());
                break;
            }
            s3.del.step.store(i as u64 + 1, Ordering::Relaxed);
            match catalog::clean(id, opts, &s3.del) {
                Ok(r) => merge_report(&mut total, r),
                Err(e) => total.notes.push(format!("{} : {}", id, e)),
            }
        }
        total.elapsed_ms = t0.elapsed().as_millis();
        total
    })
    .await;
    ticker.stop();
    let rep = rep.map_err(|e| e.to_string())?;

    let junk = tauri::async_runtime::spawn_blocking(|| catalog::measure_all(None)).await.map_err(|e| e.to_string())?;
    let mut d = state.data.lock().unwrap();
    d.junk = junk;
    refresh_disk(&mut d);
    Ok(rep)
}

// ───────────── assistant (agent IA) ─────────────

#[tauri::command]
fn agent_check() -> agent::AgentCheck {
    agent::check()
}

#[tauri::command]
fn agent_send(app: AppHandle, st: St, prompt: String, model: String, session: Option<String>, goal: u64) -> Result<String, String> {
    agent::send(app, st.inner().clone(), prompt, model, session, goal)
}

#[tauri::command]
fn agent_stop(st: St) {
    agent::stop(st.inner());
}

/// Réponse de l'utilisateur à une carte « Autoriser / Refuser » du chat. Renvoie une phrase décrivant ce qui a été fait,
/// que l'interface renvoie à l'assistant pour qu'il reprenne.
#[tauri::command]
fn permission_answer(app: AppHandle, st: St, id: String, allow: bool) -> Result<String, String> {
    let perm = st.perms.lock().unwrap().remove(&id).ok_or("Cette demande n'est plus valable.")?;
    if !allow {
        return Ok("Refusé : ne touche pas à cela.".into());
    }
    let msg = match perm.scope.as_str() {
        "chemins" => {
            core::settings::allow(&perm.paths)?;
            format!("Autorisé pour : {}.", perm.paths.join(" ; "))
        }
        scope => {
            let mut s = core::settings::get();
            match scope {
                "code" => s.protect_dev = false,
                "musique" => s.protect_music = false,
                _ => s.protect_design = false,
            }
            s.configured = true;
            core::settings::set(s)?;
            format!("Autorisé : la protection « {} » est levée (modifiable dans l'onglet Protections).", scope)
        }
    };
    let _ = app.emit("settings-changed", serde_json::json!({}));
    Ok(msg)
}

#[tauri::command]
fn settings_get() -> core::settings::Settings {
    core::settings::get()
}

#[tauri::command]
fn settings_set(app: AppHandle, s: core::settings::Settings) -> Result<(), String> {
    core::settings::set(s)?;
    let _ = app.emit("settings-changed", serde_json::json!({}));
    Ok(())
}

/// Applique les déplacements cochés dans la carte de rangement du Bureau.
#[tauri::command]
async fn organize_apply(st: St<'_>, names: Vec<String>) -> Result<core::organize::ApplyReport, String> {
    let plan = st.plan.lock().unwrap().clone().ok_or("Aucun plan de rangement en attente.")?;
    let moves: Vec<core::organize::Move> = plan.moves.iter().filter(|m| names.iter().any(|n| n == &m.name)).cloned().collect();
    let fixes = plan.fixes.clone();
    let rep = tauri::async_runtime::spawn_blocking(move || core::organize::apply(&moves, &fixes)).await.map_err(|e| e.to_string())?;
    *st.plan.lock().unwrap() = None;
    Ok(rep)
}

#[tauri::command]
async fn organize_undo() -> Result<core::organize::ApplyReport, String> {
    let j = core::organize::last_journal().ok_or("Aucun rangement à annuler.")?;
    tauri::async_runtime::spawn_blocking(move || core::organize::undo(&j)).await.map_err(|e| e.to_string())?
}

/// L'interface garde le panier ; elle en envoie une copie pour que l'assistant le voie.
#[tauri::command]
fn basket_sync(st: St, items: Vec<agent::BItem>) {
    *st.basket.lock().unwrap() = items;
}

#[tauri::command]
fn uninstall_app(command: String) -> Result<(), String> {
    sys::run_uninstaller(&command)
}

/// Relit la liste des applications (après une désinstallation) et la met à jour dans l'état partagé.
#[tauri::command]
async fn reload_apps(st: St<'_>) -> Result<usize, String> {
    let state = st.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let list = {
            let d = state.data.lock().unwrap();
            core::apps::list(&|p| d.scan.as_ref().and_then(|s| s.lookup(p).map(|id| s.nodes[id as usize].size)), false)
        };
        let n = list.len();
        state.data.lock().unwrap().apps = list;
        n
    })
    .await
    .map_err(|e| e.to_string())
}

/// Vrai tant que l'entrée de désinstallation existe : l'interface s'en sert pour savoir quand c'est terminé.
#[tauri::command]
fn app_installed(reg_key: String) -> bool {
    core::apps::key_exists(&reg_key)
}

#[tauri::command]
async fn force_plan(location: String) -> Result<core::apps::ForcePlan, String> {
    tauri::async_runtime::spawn_blocking(move || core::apps::force_plan(&location)).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn force_remove_app(app: AppHandle, st: St<'_>, name: String, location: String, reg_key: String) -> Result<core::apps::ForceReport, String> {
    let state = st.inner().clone();
    reset_del(&state, 1);
    if let Ok(mut l) = state.del.label.lock() {
        *l = name.clone();
    }
    let ticker = del_ticker(app.clone(), state.clone());
    let s3 = state.clone();
    let res = tauri::async_runtime::spawn_blocking(move || core::apps::force_remove(&name, &location, &reg_key, &s3.del)).await;
    ticker.stop();
    let rep = res.map_err(|e| e.to_string())??;
    let mut d = state.data.lock().unwrap();
    refresh_disk(&mut d);
    Ok(rep)
}

#[tauri::command]
fn reveal(path: String) {
    sys::reveal_in_explorer(&path);
}

#[tauri::command]
fn disk_now(st: St) -> Option<DiskInfo> {
    let mut d = st.data.lock().unwrap();
    refresh_disk(&mut d);
    d.disk.clone()
}

#[tauri::command]
fn relaunch_admin(app: AppHandle) -> Result<(), String> {
    sys::relaunch_elevated()?;
    app.exit(0);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Arc::new(AppState::default()))
        .setup(|app| {
            core::settings::load();
            // Serveur MCP local de l'assistant (127.0.0.1 uniquement, jeton aléatoire par lancement).
            let state: Arc<AppState> = app.state::<Arc<AppState>>().inner().clone();
            match agent::start_mcp(app.handle().clone(), state.clone()) {
                Ok(info) => *state.mcp.lock().unwrap() = Some(info),
                Err(e) => eprintln!("Assistant indisponible : {}", e),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            permission_answer,
            settings_get,
            settings_set,
            organize_apply,
            organize_undo,
            app_installed,
            reload_apps,
            force_plan,
            force_remove_app,
            agent_check,
            agent_send,
            agent_stop,
            basket_sync,
            get_status,
            start_analysis,
            find_stale,
            cancel,
            get_summary,
            get_children,
            get_folder_files,
            get_big_files,
            get_rebuildable,
            get_junk,
            get_apps,
            get_dupes,
            preview_delete,
            delete_paths,
            clean_categories,
            uninstall_app,
            reveal,
            disk_now,
            relaunch_admin,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de Libéra");
}
