//! Assistant IA : donne la main à Claude Code (abonnement) sur l'application, de façon encadrée.
//!
//!  • Un serveur MCP local (127.0.0.1, jeton aléatoire, hôte contrôlé) expose des OUTILS : analyser, trier, archiver,
//!    ranger le Bureau, remplir le panier… Aucun outil ne supprime directement : `demander_suppression` ouvre la
//!    confirmation que seul l'utilisateur peut valider.
//!  • Quand une protection bloque, l'assistant DEMANDE L'AUTORISATION dans le chat (`demander_autorisation`) : l'utilisateur
//!    répond par un bouton, et l'assistant reprend. Seuls les dossiers vitaux de Windows ne sont jamais autorisables.
//!  • Claude Code est lancé en mode non interactif avec UNIQUEMENT ces outils (`--strict-mcp-config`, `--tools ""`).

use crate::{AppState, Stored};
use libera_core::{
    self as core,
    apps::AppInfo,
    archive,
    cleaner::{self, PreviewReq},
    explain, organize, plugins,
    protect::{self, Verdict},
    scan::Progress,
    settings,
    stale::{self, StaleOptions},
    winfs,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};

const GB: f64 = 1024.0 * 1024.0 * 1024.0;
const MB: f64 = 1024.0 * 1024.0;

#[derive(Clone)]
pub struct McpInfo {
    pub port: u16,
    pub token: String,
}

/// Élément du panier, miroir de celui de l'interface.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct BItem {
    pub kind: String,
    pub path: String,
    pub id: String,
    pub name: String,
    pub size: u64,
    pub admin: bool,
    pub file: bool,
    pub warning: String,
    pub reason: String,
}

/// Demande d'autorisation en attente de la réponse de l'utilisateur dans le chat.
#[derive(Clone, Debug)]
pub struct Perm {
    pub paths: Vec<String>,
    /// "chemins", "code", "musique" ou "creation"
    pub scope: String,
}

pub struct ArchiveJob {
    pub progress: Arc<archive::ArchiveProgress>,
    pub result: Arc<Mutex<Option<Result<archive::ArchiveReport, String>>>>,
}

fn go(b: u64) -> f64 {
    ((b as f64 / GB) * 100.0).round() / 100.0
}
fn mo(b: u64) -> f64 {
    ((b as f64 / MB) * 10.0).round() / 10.0
}

fn rand_hex(n: usize) -> String {
    use std::hash::{BuildHasher, Hasher, RandomState};
    let mut s = String::new();
    for i in 0..n as u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        s.push_str(&format!("{:016x}", h.finish()));
    }
    s
}

// ───────────────────────── serveur MCP (JSON-RPC sur HTTP) ─────────────────────────

pub fn start_mcp(app: AppHandle, state: Arc<AppState>) -> Result<McpInfo, String> {
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = server.server_addr().to_ip().map(|a| a.port()).ok_or("port introuvable")?;
    let info = McpInfo { port, token: rand_hex(4) };
    let tok = info.token.clone();
    std::thread::spawn(move || {
        for mut req in server.incoming_requests() {
            let host_ok = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Host"))
                .map(|h| {
                    let v = h.value.as_str();
                    v.starts_with("127.0.0.1:") || v.starts_with("localhost:")
                })
                .unwrap_or(false);
            let auth_ok = req.headers().iter().find(|h| h.field.equiv("Authorization")).map(|h| h.value.as_str() == format!("Bearer {}", tok)).unwrap_or(false);
            if !host_ok || !auth_ok {
                let _ = req.respond(tiny_http::Response::empty(401));
                continue;
            }
            if req.method() != &tiny_http::Method::Post {
                let _ = req.respond(tiny_http::Response::empty(405));
                continue;
            }
            let mut body = String::new();
            let _ = req.as_reader().read_to_string(&mut body);
            let out = match serde_json::from_str::<Value>(&body) {
                Err(_) => Some(rpc_error(Value::Null, -32700, "JSON invalide")),
                Ok(Value::Array(items)) => {
                    let rs: Vec<Value> = items.iter().filter_map(|m| handle(&app, &state, m)).collect();
                    if rs.is_empty() { None } else { Some(Value::Array(rs)) }
                }
                Ok(m) => handle(&app, &state, &m),
            };
            match out {
                None => {
                    let _ = req.respond(tiny_http::Response::empty(202));
                }
                Some(v) => {
                    let mut r = tiny_http::Response::from_string(v.to_string());
                    r.add_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap());
                    let _ = req.respond(r);
                }
            }
        }
    });
    Ok(info)
}

fn rpc_error(id: Value, code: i32, msg: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":msg}})
}

fn handle(app: &AppHandle, st: &Arc<AppState>, m: &Value) -> Option<Value> {
    let id = m.get("id").cloned()?; // notification : pas de réponse
    let method = m.get("method").and_then(|x| x.as_str()).unwrap_or("");
    let result = match method {
        "initialize" => {
            let ver = m.pointer("/params/protocolVersion").and_then(|v| v.as_str()).unwrap_or("2025-03-26");
            json!({"protocolVersion": ver, "capabilities": {"tools": {}}, "serverInfo": {"name": "libera", "version": env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tool_defs()}),
        "tools/call" => {
            let name = m.pointer("/params/name").and_then(|v| v.as_str()).unwrap_or("");
            let args = m.pointer("/params/arguments").cloned().unwrap_or(json!({}));
            match call_tool(app, st, name, &args) {
                Ok(v) => json!({"content":[{"type":"text","text": v.to_string()}], "isError": false}),
                Err(e) => json!({"content":[{"type":"text","text": e}], "isError": true}),
            }
        }
        _ => return Some(rpc_error(id, -32601, "méthode inconnue")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn t(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    json!({"name": name, "description": desc, "inputSchema": {"type":"object","properties": props, "required": required}})
}

fn s_arr() -> Value {
    json!({"type":"array","items":{"type":"string"}})
}

fn tool_defs() -> Vec<Value> {
    vec![
        t("etat", "État du disque, de l'analyse, du panier et des protections. À appeler en premier.", json!({}), &[]),
        t("lancer_analyse", "Lance l'analyse du disque si aucune n'est disponible (attend jusqu'à 45 s ; rappeler si 'en_cours'). Modes : rapide, standard (recommandé), approfondie, experimentale (admin).", json!({"mode":{"type":"string","enum":["rapide","standard","approfondie","experimentale"]}}), &[]),
        t("gros_dossiers", "Sous-dossiers les plus lourds d'un dossier (racine par défaut), avec leur niveau de protection.", json!({"chemin":{"type":"string"},"limite":{"type":"integer"}}), &[]),
        t("gros_fichiers", "Plus gros fichiers, filtrables par inactivité (jours).", json!({"min_mo":{"type":"integer"},"inactif_jours":{"type":"integer"},"limite":{"type":"integer"}}), &[]),
        t("reconstructibles", "Dossiers regénérables sans perte (node_modules, target, .next, caches de build…).", json!({"limite":{"type":"integer"}}), &[]),
        t("zones_nettoyables", "Catégories de nettoyage connues (caches, temporaires, journaux…), taille et risque.", json!({}), &[]),
        t("fichiers_inutilises", "Fichiers probablement inutiles (installateurs, archives extraites, captures…) avec score et raisons.", json!({"mois":{"type":"integer"},"limite":{"type":"integer"}}), &[]),
        t("medias", "Images, vidéos (et audio) hors de tout projet protégé, sans critère d'âge. ajouter_au_panier=true les met tous au panier.", json!({"types":s_arr(),"taille_min_ko":{"type":"integer"},"ajouter_au_panier":{"type":"boolean"},"limite":{"type":"integer"}}), &[]),
        t("applications", "Applications installées avec dernière ouverture et nature (kind : dev, music, creation, system ou vide).", json!({"inactives_jours":{"type":"integer"},"limite":{"type":"integer"}}), &[]),
        t("plugins_audio", "Plugins audio installés (VST, VST3, CLAP, y compris ceux de FL Studio) et applications musicales associées.", json!({}), &[]),
        t("projets", "Racines de projets trouvées sur le PC. type : code, musique, creation ou tous.", json!({"type":{"type":"string"}}), &[]),
        t("expliquer", "Explique un dossier ou fichier : taille, contenu principal, note si connu, régénérable ou non. À utiliser avant de répondre à « est-ce important ? ».", json!({"chemin":{"type":"string"}}), &["chemin"]),
        t("doublons", "Groupes de fichiers identiques (analyse approfondie requise).", json!({"limite":{"type":"integer"}}), &[]),
        t("verifier", "Taille et niveau de protection de chemins avant de les proposer.", json!({"chemins":s_arr()}), &["chemins"]),
        t("panier_voir", "Contenu du panier.", json!({}), &[]),
        t("panier_ajouter", "Ajoute au panier (jamais de suppression directe). type = 'chemin' ou 'categorie'. Un élément protégé est refusé : demande alors l'autorisation avec demander_autorisation.", json!({"elements":{"type":"array","items":{"type":"object","properties":{"type":{"type":"string","enum":["chemin","categorie"]},"valeur":{"type":"string"},"raison":{"type":"string"}},"required":["type","valeur"]}}}), &["elements"]),
        t("panier_retirer", "Retire du panier (chemins ou ids).", json!({"valeurs":s_arr()}), &["valeurs"]),
        t("panier_vider", "Vide le panier.", json!({}), &[]),
        t("protections_voir", "Réglages de protection actuels de l'utilisateur.", json!({}), &[]),
        t("protections_definir", "Renforce les protections (jamais l'inverse) : code/musique/creation = true, et dossiers à ne jamais toucher. Enregistre ce que l'utilisateur a choisi de garder.", json!({"code":{"type":"boolean"},"musique":{"type":"boolean"},"creation":{"type":"boolean"},"dossiers_proteges":s_arr()}), &[]),
        t("demander_autorisation", "Quand une protection bloque : affiche dans le chat une carte avec boutons Autoriser / Refuser. portee = 'chemins' (liste chemins) ou 'code' | 'musique' | 'creation' (lever cette protection). Arrête-toi ensuite : la réponse arrive comme message de l'utilisateur. Impossible pour les dossiers vitaux de Windows.", json!({"motif":{"type":"string"},"portee":{"type":"string","enum":["chemins","code","musique","creation"]},"chemins":s_arr()}), &["motif","portee"]),
        t("desinstaller_applications", "Propose de désinstaller des applications (noms exacts de `applications`). L'utilisateur confirme dans le chat ; elles sont désinstallées une par une avec leur désinstalleur officiel.", json!({"noms":s_arr()}), &["noms"]),
        t("archiver", "Compresse des dossiers/fichiers dans un .zip puis VÉRIFIE l'archive. Sans 'sources', renvoie l'état de l'archivage en cours. Après vérification, les sources archivées peuvent être mises au panier même si protégées.", json!({"sources":s_arr(),"nom":{"type":"string"},"destination":{"type":"string"}}), &[]),
        t("bureau_analyser", "Liste les éléments du Bureau avec leur domaine proposé (Développement, Musique, Création graphique, Images, Vidéos, Documents, Archives, Installateurs, Autres). Lecture seule.", json!({}), &[]),
        t("bureau_proposer", "Propose un plan de rangement du Bureau : affectations = [{element, domaine}]. Une carte interactive s'affiche ; l'utilisateur coche et applique. Les raccourcis du Bureau sont réparés et tout est annulable.", json!({"affectations":{"type":"array","items":{"type":"object","properties":{"element":{"type":"string"},"domaine":{"type":"string"}},"required":["element","domaine"]}}}), &["affectations"]),
        t("demander_suppression", "Affiche la confirmation de suppression du panier. Seul l'utilisateur peut valider. À appeler une fois le panier prêt.", json!({}), &[]),
    ]
}

// ───────────────────────── outils ─────────────────────────

fn s_arg(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(|v| v.as_str()).map(|s| s.to_string())
}
fn u_arg(a: &Value, k: &str, d: u64) -> u64 {
    a.get(k).and_then(|v| v.as_u64()).unwrap_or(d)
}
fn v_arg(a: &Value, k: &str) -> Vec<String> {
    a.get(k).and_then(|v| v.as_array()).map(|v| v.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn need_scan(d: &Stored) -> Result<&core::scan::ScanResult, String> {
    d.scan.as_ref().ok_or_else(|| "Aucune analyse complète disponible : appelle lancer_analyse (mode standard).".to_string())
}

fn size_of_path(d: &Stored, path: &str) -> u64 {
    let disp = winfs::display_path(path);
    if let Some(s) = &d.scan {
        if let Some(id) = s.lookup(&disp) {
            return s.nodes[id as usize].size;
        }
        if let Some(f) = s.big_files.iter().find(|f| f.path.eq_ignore_ascii_case(&disp)) {
            return f.size;
        }
    }
    core::scan::measure(&disp).0
}

fn status_json(st: &Arc<AppState>) -> Value {
    let p = &st.progress;
    let exp = p.expected.load(Ordering::Relaxed);
    let pct = if exp > 0 { ((p.bytes.load(Ordering::Relaxed) as f64 / exp as f64) * 100.0).min(99.0).round() } else { -1.0 };
    json!({
        "statut": if st.busy.load(Ordering::Relaxed) { "en_cours" } else { "termine" },
        "etape": st.phase.lock().map(|x| x.clone()).unwrap_or_default(),
        "pourcentage": if pct >= 0.0 { json!(pct) } else { Value::Null },
        "fichiers_lus": p.files.load(Ordering::Relaxed),
    })
}

fn basket_total(st: &AppState) -> u64 {
    st.basket.lock().unwrap().iter().map(|i| i.size).sum()
}

fn kind_protected(kind: &str) -> Option<&'static str> {
    let s = settings::get();
    match kind {
        "dev" if s.protect_dev => Some("code"),
        "music" if s.protect_music => Some("musique"),
        "creation" if s.protect_design => Some("creation"),
        _ => None,
    }
}

fn is_daw(name: &str) -> bool {
    let n = name.to_lowercase();
    ["fl studio", "ableton", "cubase", "reaper", "studio one", "bitwig", "logic", "pro tools", "reason"].iter().any(|k| n.contains(k))
}

fn call_tool(app: &AppHandle, st: &Arc<AppState>, name: &str, a: &Value) -> Result<Value, String> {
    match name {
        "etat" => {
            let d = st.data.lock().unwrap();
            let disk = d.disk.clone().or_else(|| core::sys::disk_info("C:\\"));
            let b = st.basket.lock().unwrap();
            let s = settings::get();
            Ok(json!({
                "disque": disk.map(|k| json!({"utilise_go": go(k.used), "libre_go": go(k.free), "total_go": go(k.total)})),
                "objectif_go_utilise_max": st.goal.load(Ordering::Relaxed),
                "analyse": d.mode.map(|m| json!({"mode": format!("{:?}", m).to_lowercase(), "arbre_complet": d.scan.is_some(), "go_analyses": d.scan.as_ref().map(|s| go(s.nodes[0].size))})),
                "analyse_en_cours": st.busy.load(Ordering::Relaxed),
                "panier": {"elements": b.len(), "total_go": go(b.iter().map(|i| i.size).sum())},
                "administrateur": core::sys::is_elevated(),
                "protections": {"configurees_par_utilisateur": s.configured, "code": s.protect_dev, "musique": s.protect_music, "creation_graphique": s.protect_design, "dossiers_proteges": s.custom_protected, "autorisations_accordees": s.allowed_paths},
            }))
        }
        "protections_voir" => {
            let s = settings::get();
            Ok(json!({"configurees_par_utilisateur": s.configured, "code": s.protect_dev, "musique": s.protect_music, "creation_graphique": s.protect_design, "dossiers_proteges": s.custom_protected, "autorisations_accordees": s.allowed_paths}))
        }
        "protections_definir" => {
            let mut s = settings::get();
            let before = s.clone();
            if a.get("code").and_then(|v| v.as_bool()) == Some(true) { s.protect_dev = true; }
            if a.get("musique").and_then(|v| v.as_bool()) == Some(true) { s.protect_music = true; }
            if a.get("creation").and_then(|v| v.as_bool()) == Some(true) { s.protect_design = true; }
            for p in v_arg(a, "dossiers_proteges") {
                let d = winfs::display_path(&p).trim_end_matches('\\').to_string();
                if !d.is_empty() && !s.custom_protected.iter().any(|c| c.eq_ignore_ascii_case(&d)) { s.custom_protected.push(d); }
            }
            s.configured = true;
            settings::set(s.clone())?;
            let asked_loosen = (a.get("code").and_then(|v| v.as_bool()) == Some(false)) || (a.get("musique").and_then(|v| v.as_bool()) == Some(false)) || (a.get("creation").and_then(|v| v.as_bool()) == Some(false));
            let _ = app.emit("settings-changed", json!({}));
            Ok(json!({"enregistre": true, "modifie": before != s,
                "remarque": if asked_loosen { "Lever une protection ne passe pas par cet outil : utilise demander_autorisation (l'utilisateur répond dans le chat)." } else { "" }}))
        }
        "demander_autorisation" => {
            let scope = s_arg(a, "portee").unwrap_or_else(|| "chemins".into());
            let motif: String = s_arg(a, "motif").unwrap_or_default().chars().take(400).collect();
            let mut paths = Vec::new();
            if scope == "chemins" {
                for p in v_arg(a, "chemins") {
                    let disp = winfs::display_path(&p).trim_end_matches('\\').to_string();
                    // Les dossiers vitaux de Windows ne sont jamais autorisables : on l'explique tout de suite.
                    if let Verdict::Interdit(m) = protect::check(&disp) {
                        if !settings::is_custom_protected(&disp) {
                            return Err(format!("« {} » ne peut jamais être autorisé : {}", disp, m));
                        }
                    }
                    paths.push(disp);
                }
                if paths.is_empty() {
                    return Err("Aucun chemin donné.".into());
                }
            } else if !matches!(scope.as_str(), "code" | "musique" | "creation") {
                return Err("portee invalide".into());
            }
            let id = rand_hex(1);
            st.perms.lock().unwrap().insert(id.clone(), Perm { paths: paths.clone(), scope: scope.clone() });
            let _ = app.emit("agent-permission", json!({"id": id, "motif": motif, "portee": scope, "chemins": paths}));
            Ok(json!({"statut":"en_attente_de_la_reponse_de_l_utilisateur","remarque":"Une carte avec les boutons Autoriser / Refuser est affichée dans le chat. Dis une phrase d'explication, puis ARRÊTE-TOI : sa réponse arrivera comme prochain message."}))
        }
        "lancer_analyse" => {
            let mode = match s_arg(a, "mode").as_deref() {
                Some("rapide") => core::Mode::Rapide,
                Some("approfondie") => core::Mode::Approfondie,
                Some("experimentale") => core::Mode::Experimentale,
                _ => core::Mode::Standard,
            };
            if !st.busy.load(Ordering::Relaxed) {
                let have = { st.data.lock().unwrap().scan.is_some() };
                if have && mode != core::Mode::Rapide {
                    return Ok(json!({"statut":"deja_disponible","remarque":"Une analyse complète existe déjà ; inutile de relancer."}));
                }
                crate::start_analysis(app.clone(), app.state::<Arc<AppState>>(), mode, "C:\\".to_string())?;
            }
            for _ in 0..90 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if !st.busy.load(Ordering::Relaxed) {
                    break;
                }
            }
            Ok(status_json(st))
        }
        "gros_dossiers" => {
            let d = st.data.lock().unwrap();
            let s = need_scan(&d)?;
            let id = match s_arg(a, "chemin") {
                Some(p) if !p.trim().is_empty() => s.lookup(&p).ok_or("Chemin absent de l'analyse.")?,
                _ => 0,
            };
            let lim = u_arg(a, "limite", 15).min(40) as usize;
            let rows: Vec<Value> = s
                .children(id)
                .into_iter()
                .take(lim)
                .map(|c| {
                    let (niv, msg) = match protect::check(&c.path) {
                        Verdict::Ok => ("ok", String::new()),
                        Verdict::Attention(m) => ("protege_par_reglage", m),
                        Verdict::Interdit(m) => ("interdit", m),
                    };
                    json!({"chemin": c.path, "go": go(c.size), "pct": (c.pct * 10.0).round() / 10.0, "fichiers": c.files, "protection": niv, "note": msg})
                })
                .collect();
            Ok(json!({"dossier": s.path_of(id), "total_go": go(s.nodes[id as usize].size), "sous_dossiers": rows}))
        }
        "gros_fichiers" => {
            let d = st.data.lock().unwrap();
            let s = need_scan(&d)?;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|x| x.as_secs() as i64).unwrap_or(0);
            let min = u_arg(a, "min_mo", 100) << 20;
            let age = a.get("inactif_jours").and_then(|v| v.as_i64());
            let lim = u_arg(a, "limite", 20).min(50) as usize;
            let rows: Vec<Value> = s
                .big_files
                .iter()
                .filter(|f| f.size >= min && age.map(|dd| f.mtime > 0 && f.mtime < now - dd * 86_400).unwrap_or(true))
                .take(lim)
                .map(|f| {
                    let niv = match protect::check(&f.path) { Verdict::Ok => "ok", Verdict::Attention(_) => "protege_par_reglage", Verdict::Interdit(_) => "interdit" };
                    json!({"chemin": f.path, "mo": mo(f.size), "modifie": f.mtime, "protection": niv})
                })
                .collect();
            Ok(json!({"fichiers": rows}))
        }
        "reconstructibles" => {
            let d = st.data.lock().unwrap();
            let s = need_scan(&d)?;
            let lim = u_arg(a, "limite", 20).min(50) as usize;
            let rows: Vec<Value> = s.rebuildable(50 << 20).into_iter().take(lim).map(|c| json!({"chemin": c.path, "go": go(c.size)})).collect();
            Ok(json!({"dossiers": rows, "remarque": "Regénérables par npm/cargo/gradle… : sans perte de code."}))
        }
        "zones_nettoyables" => {
            let d = st.data.lock().unwrap();
            let rows: Vec<Value> = d.junk.iter().filter(|j| j.size > 0 || j.cat.action.is_some()).map(|j| json!({"id": j.cat.id, "nom": j.cat.name, "go": go(j.size), "risque": j.cat.risk, "admin": j.cat.needs_admin, "description": j.cat.description})).collect();
            Ok(json!({"zones": rows}))
        }
        "fichiers_inutilises" => {
            let names: Vec<String> = { st.data.lock().unwrap().apps.iter().map(|x| x.name.clone()).collect() };
            let opts = StaleOptions { months: u_arg(a, "mois", 12) as u32, ..Default::default() };
            let r = stale::analyze(opts, &names, &Progress::default());
            let lim = u_arg(a, "limite", 30).min(60) as usize;
            let rows: Vec<Value> = r.items.iter().take(lim).map(|i| json!({"chemin": i.path, "mo": mo(i.size), "confiance": i.level, "score": i.score, "raisons": i.reasons})).collect();
            let prot: Vec<Value> = r.protected.iter().map(|p| json!({"raison": p.reason, "go": go(p.bytes)})).collect();
            Ok(json!({"total_propose_go": go(r.items.iter().map(|i| i.size).sum()), "propositions": rows, "proteges": prot}))
        }
        "medias" => {
            let types = v_arg(a, "types");
            let all = types.is_empty();
            let has = |k: &str| all && k != "audio" || types.iter().any(|t| t.starts_with(k));
            let r = stale::media(has("image"), has("video") || has("vid"), has("audio"), u_arg(a, "taille_min_ko", 100), &Progress::default());
            let lim = u_arg(a, "limite", 25).min(60) as usize;
            let mut added = 0usize;
            if a.get("ajouter_au_panier").and_then(|v| v.as_bool()) == Some(true) {
                let mut items: Vec<BItem> = r.items.iter().take(5000).map(|i| BItem { kind: "path".into(), path: i.path.clone(), name: i.name.clone(), size: i.size, file: true, reason: "Média hors de tout projet protégé".into(), ..Default::default() }).collect();
                added = items.len();
                {
                    let mut b = st.basket.lock().unwrap();
                    items.retain(|it| !b.iter().any(|x| x.path.eq_ignore_ascii_case(&it.path)));
                    b.extend(items.iter().cloned());
                }
                let _ = app.emit("agent-basket", json!({"op": "add", "items": items}));
            }
            Ok(json!({
                "total_go": go(r.total_bytes), "fichiers": r.items.len(), "ajoutes_au_panier": added,
                "par_dossier": r.by_dir.iter().take(15).map(|d| json!({"dossier": d.dir, "fichiers": d.files, "go": go(d.bytes)})).collect::<Vec<_>>(),
                "plus_gros": r.items.iter().take(lim).map(|i| json!({"chemin": i.path, "mo": mo(i.size)})).collect::<Vec<_>>(),
                "proteges": r.protected.iter().map(|p| json!({"raison": p.reason, "go": go(p.bytes)})).collect::<Vec<_>>(),
            }))
        }
        "applications" => {
            let d = st.data.lock().unwrap();
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|x| x.as_secs() as i64).unwrap_or(0);
            let age = a.get("inactives_jours").and_then(|v| v.as_i64());
            let lim = u_arg(a, "limite", 25).min(60) as usize;
            let rows: Vec<Value> = d
                .apps
                .iter()
                .filter(|x| age.map(|dd| x.kind != "system" && kind_protected(x.kind).is_none() && x.last_used > 0 && x.last_used < now - dd * 86_400).unwrap_or(true))
                .take(lim)
                .map(|x| json!({"nom": x.name, "go": go(x.size), "derniere_ouverture": x.last_used, "kind": x.kind, "protegee_par_reglage": kind_protected(x.kind).is_some(), "installee": x.install_date}))
                .collect();
            Ok(json!({"applications": rows}))
        }
        "plugins_audio" => {
            let list = plugins::find();
            let d = st.data.lock().unwrap();
            let apps: Vec<Value> = d.apps.iter().filter(|x| x.kind == "music" && !is_daw(&x.name)).map(|x| json!({"nom": x.name, "go": go(x.size)})).collect();
            Ok(json!({
                "plugins": list.iter().take(80).map(|p| json!({"chemin": p.path, "type": p.kind, "mo": mo(p.size)})).collect::<Vec<_>>(),
                "total_go": go(list.iter().map(|p| p.size).sum()),
                "applications_musicales": apps,
                "remarque": "Désinstalle d'abord les applications (desinstaller_applications) : leur désinstalleur retire proprement leurs plugins. Mets ensuite au panier les plugins restants. Les DAW (FL Studio…) ne sont pas listés comme plugins.",
            }))
        }
        "projets" => {
            let kind = s_arg(a, "type").unwrap_or_else(|| "tous".into());
            let kind = match kind.as_str() { "musique" | "creation" | "code" => kind, _ => "tous".into() };
            let list = stale::find_projects(&kind, &Progress::default());
            Ok(json!({"projets": list.iter().take(60).map(|p| json!({"chemin": p.path, "type": p.kind, "go": go(p.size), "fichiers": p.files})).collect::<Vec<_>>(), "total_go": go(list.iter().map(|p| p.size).sum())}))
        }
        "expliquer" => {
            let p = s_arg(a, "chemin").ok_or("chemin requis")?;
            let e = explain::describe(&p);
            Ok(json!({"chemin": e.path, "existe": e.exists, "dossier": e.is_dir, "go": go(e.size), "mo": mo(e.size), "fichiers": e.files, "contenu_principal": e.biggest.iter().map(|c| json!({"nom": c.name, "mo": mo(c.size)})).collect::<Vec<_>>(), "note": e.note, "regenerable": e.regenerable, "protection": match protect::check(&e.path) { Verdict::Ok => "ok", Verdict::Attention(_) => "protege_par_reglage", Verdict::Interdit(_) => "interdit" }}))
        }
        "doublons" => {
            let d = st.data.lock().unwrap();
            let lim = u_arg(a, "limite", 10).min(30) as usize;
            let rows: Vec<Value> = d.dupes.iter().take(lim).map(|g| json!({"mo_chacun": mo(g.size), "recuperable_go": go(g.wasted), "copies": g.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>()})).collect();
            Ok(json!({"groupes": rows, "remarque": "Garder la copie la plus ancienne (la première)."}))
        }
        "verifier" => {
            let paths = v_arg(a, "chemins");
            let d = st.data.lock().unwrap();
            let reqs: Vec<PreviewReq> = paths.iter().map(|p| PreviewReq { path: p.clone(), size: Some(size_of_path(&d, p)), files: None }).collect();
            drop(d);
            let out: Vec<Value> = cleaner::preview(&reqs).into_iter().map(|p| json!({"chemin": p.path, "go": go(p.size), "protection": serde_json::to_value(&p.verdict).unwrap_or(Value::Null)})).collect();
            Ok(json!({"resultats": out}))
        }
        "panier_voir" => {
            let b = st.basket.lock().unwrap();
            let items: Vec<Value> = b.iter().take(60).map(|i| json!({"type": if i.kind == "cat" { "categorie" } else { "chemin" }, "valeur": if i.kind == "cat" { &i.id } else { &i.path }, "go": go(i.size), "raison": i.reason})).collect();
            Ok(json!({"elements": items, "nombre": b.len(), "total_go": go(b.iter().map(|i| i.size).sum())}))
        }
        "panier_ajouter" => {
            let els = a.get("elements").and_then(|v| v.as_array()).ok_or("elements requis")?;
            let d = st.data.lock().unwrap();
            let archived = st.archived.lock().unwrap().clone();
            let mut added: Vec<BItem> = Vec::new();
            let mut refused: Vec<Value> = Vec::new();
            for e in els {
                let ty = e.get("type").and_then(|v| v.as_str()).unwrap_or("chemin");
                let val = e.get("valeur").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let reason: String = e.get("raison").and_then(|v| v.as_str()).unwrap_or("").chars().take(200).collect();
                if val.is_empty() {
                    continue;
                }
                if ty == "categorie" {
                    match d.junk.iter().find(|j| j.cat.id == val) {
                        Some(j) => added.push(BItem { kind: "cat".into(), id: j.cat.id.clone(), name: j.cat.name.clone(), size: j.size, admin: j.cat.needs_admin, reason, ..Default::default() }),
                        None => refused.push(json!({"valeur": val, "motif": "catégorie inconnue (voir zones_nettoyables)"})),
                    }
                    continue;
                }
                let disp = winfs::display_path(&val);
                if winfs::attrs(&winfs::extended(&disp)).is_err() {
                    refused.push(json!({"valeur": val, "motif": "chemin introuvable"}));
                    continue;
                }
                let dl = disp.to_lowercase();
                let backed_up = archived.iter().any(|s| dl == s.to_lowercase() || dl.starts_with(&format!("{}\\", s.to_lowercase())));
                match protect::check_deep(&disp) {
                    Verdict::Interdit(m) => refused.push(json!({"valeur": val, "motif": m, "autorisable": !m.contains("Windows") && !m.contains("système") && !m.contains("vital") })),
                    Verdict::Attention(m) if !backed_up => refused.push(json!({"valeur": val, "motif": format!("{} Protégé par les réglages de l'utilisateur.", m), "autorisable": true, "que_faire": "Appelle demander_autorisation (portee 'chemins' avec ce chemin, ou la catégorie) puis arrête-toi."})),
                    v => {
                        let warning = if let Verdict::Attention(m) = v { format!("Sauvegardé dans une archive vérifiée. {}", m) } else { String::new() };
                        let is_dir = matches!(winfs::attrs(&winfs::extended(&disp)), Ok(x) if x & 0x10 != 0);
                        let name = disp.rsplit('\\').next().unwrap_or(&disp).to_string();
                        added.push(BItem { kind: "path".into(), path: disp.clone(), name, size: size_of_path(&d, &disp), file: !is_dir, warning, reason, ..Default::default() });
                    }
                }
            }
            drop(d);
            {
                let mut b = st.basket.lock().unwrap();
                for it in &added {
                    if !b.iter().any(|x| x.kind == it.kind && x.path.eq_ignore_ascii_case(&it.path) && x.id == it.id) {
                        b.push(it.clone());
                    }
                }
            }
            let _ = app.emit("agent-basket", json!({"op": "add", "items": added}));
            Ok(json!({"ajoutes": added.len(), "refuses": refused, "panier_total_go": go(basket_total(st))}))
        }
        "panier_retirer" => {
            let vals = v_arg(a, "valeurs");
            st.basket.lock().unwrap().retain(|i| !vals.iter().any(|v| v.eq_ignore_ascii_case(&i.path) || v == &i.id));
            let _ = app.emit("agent-basket", json!({"op": "remove", "values": vals}));
            Ok(json!({"panier_total_go": go(basket_total(st))}))
        }
        "panier_vider" => {
            st.basket.lock().unwrap().clear();
            let _ = app.emit("agent-basket", json!({"op": "clear"}));
            Ok(json!({"panier_total_go": 0}))
        }
        "desinstaller_applications" => {
            let names = v_arg(a, "noms");
            let d = st.data.lock().unwrap();
            let mut ok: Vec<Value> = Vec::new();
            let mut refused: Vec<Value> = Vec::new();
            for n in &names {
                let Some(app_) = d.apps.iter().find(|x| x.name.eq_ignore_ascii_case(n)) else {
                    refused.push(json!({"nom": n, "motif": "application introuvable (utilise le nom exact de `applications`)"}));
                    continue;
                };
                if app_.kind == "system" {
                    refused.push(json!({"nom": n, "motif": "composant ou pilote système : à ne pas désinstaller", "autorisable": false}));
                } else if let Some(scope) = kind_protected(app_.kind) {
                    refused.push(json!({"nom": n, "motif": format!("protégée par les réglages (catégorie {})", scope), "autorisable": true, "que_faire": format!("demander_autorisation avec portee '{}' puis arrête-toi", scope)}));
                } else if app_.uninstall.is_empty() && app_.location.is_empty() {
                    refused.push(json!({"nom": n, "motif": "aucun désinstalleur connu"}));
                } else {
                    ok.push(json!({"name": app_.name, "reg_key": app_.reg_key, "uninstall": app_.uninstall, "quiet": app_.quiet_uninstall, "location": app_.location, "size": app_.size}));
                }
            }
            drop(d);
            if !ok.is_empty() {
                let _ = app.emit("agent-uninstall", json!({"apps": ok}));
            }
            Ok(json!({"proposees": ok.len(), "refusees": refused, "remarque": if ok.is_empty() { "Rien à proposer." } else { "Une carte de confirmation est affichée dans le chat. Arrête-toi : l'utilisateur la valide, puis les désinstallations se font une par une." }}))
        }
        "archiver" => {
            let sources = v_arg(a, "sources");
            if sources.is_empty() {
                let job = st.archive.lock().unwrap();
                return match job.as_ref() {
                    None => Ok(json!({"statut": "aucune_archive_en_cours"})),
                    Some(j) => archive_status(j),
                };
            }
            {
                let job = st.archive.lock().unwrap();
                if let Some(j) = job.as_ref() {
                    if j.result.lock().unwrap().is_none() {
                        return Err("Un archivage est déjà en cours : rappelle `archiver` sans arguments pour suivre.".into());
                    }
                }
            }
            let stamp = {
                let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                format!("{}", secs)
            };
            let nom: String = s_arg(a, "nom").unwrap_or_else(|| "archive".into()).chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect();
            let dest = match s_arg(a, "destination") {
                Some(d) if d.to_lowercase().ends_with(".zip") => d,
                Some(d) => format!("{}\\{}-{}.zip", d.trim_end_matches('\\'), nom, stamp),
                None => format!("{}\\Documents\\Libera-archives\\{}-{}.zip", std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Public".into()), nom, stamp),
            };
            let progress = Arc::new(archive::ArchiveProgress::default());
            let result = Arc::new(Mutex::new(None));
            let (p2, r2, app2, dest2, src2, st2) = (progress.clone(), result.clone(), app.clone(), dest.clone(), sources.clone(), st.clone());
            std::thread::spawn(move || {
                let (p3, app3) = (p2.clone(), app2.clone());
                let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
                let d3 = done.clone();
                let ticker = std::thread::spawn(move || {
                    while !d3.load(Ordering::Relaxed) {
                        let _ = app3.emit("archive-progress", json!({"files": p3.files.load(Ordering::Relaxed), "bytes": p3.bytes.load(Ordering::Relaxed), "total": p3.total.load(Ordering::Relaxed), "phase": p3.phase.lock().map(|x| x.clone()).unwrap_or_default()}));
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                });
                let res = archive::create(&src2, &dest2, &p2);
                done.store(true, Ordering::Relaxed);
                let _ = ticker.join();
                if let Ok(rep) = &res {
                    if rep.verified {
                        st2.archived.lock().unwrap().extend(rep.sources.iter().cloned());
                    }
                }
                *r2.lock().unwrap() = Some(res);
                let _ = app2.emit("archive-progress", json!({"done": true}));
            });
            *st.archive.lock().unwrap() = Some(ArchiveJob { progress, result });
            for _ in 0..90 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let job = st.archive.lock().unwrap();
                if job.as_ref().map(|j| j.result.lock().unwrap().is_some()).unwrap_or(true) {
                    break;
                }
            }
            let job = st.archive.lock().unwrap();
            archive_status(job.as_ref().unwrap())
        }
        "bureau_analyser" => {
            let roots = organize::desktop_roots();
            let root = roots.first().ok_or("Bureau introuvable")?.clone();
            let items = organize::scan_desktop(&root, &Progress::default());
            let rows: Vec<Value> = items.iter().take(120).map(|i| json!({"element": i.name, "type": i.kind, "domaine_propose": i.domain, "go": go(i.size), "mo": mo(i.size), "dossier": i.is_dir, "deplacable": i.movable})).collect();
            Ok(json!({"bureau": root, "elements": rows, "nombre": items.len(), "remarque": "Les raccourcis restent sur le Bureau. Les projets de code sont signalés (type 'projet') : ne les propose pas au déplacement sans demande explicite."}))
        }
        "bureau_proposer" => {
            let roots = organize::desktop_roots();
            let root = roots.first().ok_or("Bureau introuvable")?.clone();
            let pairs: Vec<(String, String)> = a
                .get("affectations")
                .and_then(|v| v.as_array())
                .map(|v| v.iter().filter_map(|x| Some((x.get("element")?.as_str()?.to_string(), x.get("domaine")?.as_str()?.to_string()))).collect())
                .unwrap_or_default();
            if pairs.is_empty() {
                return Err("affectations vides".into());
            }
            let plan = organize::build_plan(&root, &pairs)?;
            *st.plan.lock().unwrap() = Some(plan.clone());
            let _ = app.emit("agent-organize", serde_json::to_value(&plan).unwrap_or(Value::Null));
            Ok(json!({"deplacements": plan.moves.len(), "ignores": plan.kept, "raccourcis_a_reparer": plan.fixes.len(), "remarque": "Une carte interactive est affichée dans le chat : l'utilisateur coche et applique. Arrête-toi et résume en une phrase."}))
        }
        "demander_suppression" => {
            if st.basket.lock().unwrap().is_empty() {
                return Err("Le panier est vide.".into());
            }
            let _ = app.emit("agent-request-delete", json!({}));
            Ok(json!({"statut":"en_attente_de_confirmation","remarque":"La fenêtre de confirmation est affichée. Seul l'utilisateur peut valider. Explique brièvement ce qu'il va voir et arrête-toi."}))
        }
        _ => Err(format!("Outil inconnu : {}", name)),
    }
}

fn archive_status(j: &ArchiveJob) -> Result<Value, String> {
    let res = j.result.lock().unwrap();
    match res.as_ref() {
        None => {
            let p = &j.progress;
            let total = p.total.load(Ordering::Relaxed);
            let b = p.bytes.load(Ordering::Relaxed);
            Ok(json!({"statut": "en_cours", "etape": p.phase.lock().map(|x| x.clone()).unwrap_or_default(), "go_traites": go(b), "go_total": go(total), "pourcentage": if total > 0 { json!(((b as f64 / total as f64) * 100.0).min(99.0).round()) } else { Value::Null }, "remarque": "Rappelle `archiver` sans arguments pour suivre."}))
        }
        Some(Ok(r)) => Ok(json!({"statut": "termine", "archive": r.zip_path, "fichiers": r.files, "go_originaux": go(r.bytes_in), "go_archive": go(r.bytes_out), "verifiee": r.verified, "ignores": r.skipped.iter().take(10).collect::<Vec<_>>(), "sources": r.sources, "remarque": if r.verified { "Archive vérifiée : tu peux mettre les sources au panier (même protégées), puis demander_suppression." } else { "VÉRIFICATION ÉCHOUÉE : ne propose PAS de supprimer les originaux." }})),
        Some(Err(e)) => Err(e.clone()),
    }
}

// ───────────────────────── lancement de Claude Code ─────────────────────────

#[derive(Serialize)]
pub struct AgentCheck {
    pub found: bool,
    pub path: String,
    pub version: String,
}

pub fn find_claude() -> Option<String> {
    let mut cands: Vec<String> = Vec::new();
    if let Ok(p) = std::env::var("PATH") {
        for dir in p.split(';').filter(|d| !d.is_empty()) {
            cands.push(format!("{}\\claude.exe", dir.trim_end_matches('\\')));
        }
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        cands.push(format!("{}\\.local\\bin\\claude.exe", h));
        cands.push(format!("{}\\AppData\\Roaming\\npm\\claude.exe", h));
    }
    cands.into_iter().find(|c| std::path::Path::new(c).is_file())
}

pub fn check() -> AgentCheck {
    match find_claude() {
        None => AgentCheck { found: false, path: String::new(), version: String::new() },
        Some(p) => {
            let (_, out) = core::sys::run_tool(&p, &["--version"]);
            AgentCheck { found: true, path: p, version: out.trim().to_string() }
        }
    }
}

fn new_uuid() -> String {
    let h = rand_hex(1) + &rand_hex(1);
    format!("{}-{}-4{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

fn system_prompt(goal_go: u64) -> String {
    let s = settings::get();
    let yn = |b: bool| if b { "OUI (à respecter)" } else { "non (l'utilisateur ne veut pas la protéger)" };
    format!(
"Tu es l'assistant de Libéra, un outil qui libère de l'espace disque et range le PC sous Windows. Tu écris en français, de façon brève, concrète et chaleureuse. N'écris AUCUN emoji.\n\
OBJECTIF : l'utilisateur veut que son disque C: utilise au maximum {goal} Go. Tu n'agis QUE par les outils (préfixe mcp__libera__).\n\
PROTECTIONS CHOISIES PAR L'UTILISATEUR (réglables à tout moment) : projets de code = {dev} ; musique = {mus} ; création graphique = {des} ; dossiers à ne jamais toucher = {dos:?} ; réglages confirmés = {cfg}.\n\
Tu ne décides PAS à sa place de ce qui est précieux : tu n'as aucun profil en tête. {ask}\n\
QUAND UN ÉLÉMENT EST PROTÉGÉ : ne dis jamais « bloqué par le système » et n'abandonne pas. Explique en une phrase pourquoi (le réglage concerné) et appelle `demander_autorisation` : une carte avec boutons Autoriser / Refuser s'affiche dans le chat, puis ARRÊTE-TOI. Sa réponse arrive comme prochain message ; continue alors. Seuls les dossiers vitaux de Windows ne sont jamais autorisables : dis-le franchement.\n\
SUPPRESSION : tu ne peux rien supprimer toi-même. Tu remplis le panier (`panier_ajouter`, une raison courte par élément) puis `demander_suppression` : l'utilisateur confirme. Pour garder une copie, propose d'abord `archiver` (zip vérifié) ; une fois vérifiée, les sources peuvent être mises au panier. Pour retirer des applications, utilise `desinstaller_applications` (désinstalleur officiel), pas le panier. Pour ranger le Bureau : `bureau_analyser`, puis `bureau_proposer` (jamais de déplacement direct ; les projets de code ne sont proposés que si l'utilisateur le demande).\n\
SI ON TE DEMANDE « EST-CE IMPORTANT ? » : appelle `expliquer` et réponds honnêtement, y compris « je ne sais pas ».\n\
MÉTHODE : `etat` d'abord ; analyse standard si besoin ; privilégie ce qui est sans risque (zones nettoyables, reconstructibles, installateurs déjà installés, médias hors projets). Reste économe : peu d'appels, limites basses.\n\
FORMAT DES RÉPONSES (rendu riche dans l'application) : titres avec ###, listes courtes, tableaux markdown (colonnes : Élément | Taille | Remarque), gras pour les chiffres clés, chemins entre `backticks` (ils deviennent cliquables). Pour poser une question ou proposer des choix, termine par un bloc :\n```reponses\nChoix 1\nChoix 2\n```\nchaque ligne devient un bouton qui envoie cette phrase. Une seule question à la fois.",
        goal = goal_go, dev = yn(s.protect_dev), mus = yn(s.protect_music), des = yn(s.protect_design), dos = s.custom_protected, cfg = s.configured,
        ask = if s.configured { "" } else { "Les réglages n'ont pas encore été confirmés : AVANT toute proposition de suppression, demande en une seule question (avec un bloc reponses) ce qu'il veut garder (projets de code, musique, création graphique, dossiers précis ou rien du tout), puis enregistre avec `protections_definir` (renforcer) ou `demander_autorisation` (assouplir)." }
    )
}

pub fn send(app: AppHandle, st: Arc<AppState>, prompt: String, model: String, session: Option<String>, goal: u64) -> Result<String, String> {
    let mcp = st.mcp.lock().unwrap().clone().ok_or("Le serveur local de l'assistant n'est pas démarré.")?;
    let exe = find_claude().ok_or("Claude Code est introuvable. Installe-le puis connecte-toi avec ton abonnement (lance `claude` dans un terminal).")?;
    {
        let mut run = st.agent.lock().unwrap();
        if let Some(c) = run.as_mut() {
            if matches!(c.try_wait(), Ok(None)) {
                return Err("L'assistant est déjà en train de répondre.".into());
            }
        }
        *run = None;
    }
    st.goal.store(goal, Ordering::Relaxed);
    let dir = std::env::temp_dir().join("libera-agent");
    let _ = std::fs::create_dir_all(&dir);
    let cfg = dir.join("mcp.json");
    let cfg_json = json!({"mcpServers": {"libera": {"type": "http", "url": format!("http://127.0.0.1:{}/mcp", mcp.port), "headers": {"Authorization": format!("Bearer {}", mcp.token)}}}});
    std::fs::write(&cfg, cfg_json.to_string()).map_err(|e| e.to_string())?;

    let (sid, resume) = match session {
        Some(s) if !s.is_empty() => (s, true),
        _ => (new_uuid(), false),
    };
    let model_arg = match model.as_str() { "haiku" | "sonnet" | "opus" => model.clone(), _ => "sonnet".to_string() };
    let mut cmd = Command::new(&exe);
    cmd.current_dir(&dir)
        .args(["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages"])
        .args(["--model", &model_arg])
        .args(["--mcp-config"]).arg(&cfg)
        .args(["--strict-mcp-config", "--tools", "", "--allowedTools", "mcp__libera", "--disable-slash-commands"])
        .args(["--append-system-prompt", &system_prompt(goal)]);
    if resume {
        cmd.args(["--resume", &sid]);
    } else {
        cmd.args(["--session-id", &sid]);
    }
    // Abonnement, pas API : on retire toute clé qui ferait facturer l'API, et la variable de session imbriquée.
    cmd.env_remove("ANTHROPIC_API_KEY").env_remove("ANTHROPIC_AUTH_TOKEN").env_remove("CLAUDECODE")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .creation_flags(0x0800_0000);
    let mut child = cmd.spawn().map_err(|e| format!("Lancement impossible : {}", e))?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(prompt.as_bytes());
    }
    let stdout = child.stdout.take().ok_or("sortie indisponible")?;
    let stderr = child.stderr.take();
    *st.agent.lock().unwrap() = Some(child);

    let app2 = app.clone();
    let st2 = st.clone();
    std::thread::spawn(move || {
        let err_buf = Arc::new(Mutex::new(String::new()));
        if let Some(se) = stderr {
            let eb = err_buf.clone();
            std::thread::spawn(move || {
                for l in BufReader::new(se).lines().map_while(Result::ok) {
                    let mut b = eb.lock().unwrap();
                    if b.len() < 4000 {
                        b.push_str(&l);
                        b.push('\n');
                    }
                }
            });
        }
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                let _ = app2.emit("agent-event", line);
            }
        }
        let code = st2.agent.lock().unwrap().as_mut().and_then(|c| c.wait().ok()).and_then(|s| s.code()).unwrap_or(-1);
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = app2.emit("agent-done", json!({"code": code, "stderr": err_buf.lock().unwrap().clone()}));
    });
    Ok(sid)
}

pub fn stop(st: &Arc<AppState>) {
    if let Some(c) = st.agent.lock().unwrap().as_mut() {
        let _ = c.kill();
    }
}

#[allow(dead_code)]
fn _unused(_: &AppInfo) {}
