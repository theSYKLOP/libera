//! Moteur de pertinence : repère les fichiers probablement inutiles, sans jamais toucher à ce qui compte
//! pour un développeur / musicien / graphiste.
//!
//! Raisonnement en trois couches :
//!  1. CONTEXTE  — le dossier est-il un projet de code (.git, package.json, Cargo.toml, *.sln…), un dossier de
//!                 musique (FL Studio, samples…), un dossier de création graphique (affiches, logos…) ?
//!                 Ce contexte se transmet à tous les sous-dossiers : tout ce qui est dedans est PROTÉGÉ.
//!  2. NATURE    — que représente le fichier : code, projet musical, audio, création, ressource téléchargée
//!                 (pack PSD…), image qui traîne, installateur, archive déjà extraite, fichier temporaire…
//!  3. SCORE     — confiance (0-100) que le fichier est superflu : inactivité, emplacement (Téléchargements…),
//!                 taille, indices (« Copie », « Capture d'écran », archive déjà extraite, application déjà installée).
//!                 Chaque point du score est justifié par une raison affichée à l'utilisateur.

use crate::scan::Progress;
use crate::winfs::{self, Entry};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(default)]
pub struct StaleOptions {
    /// Inactivité minimale en mois pour qu'un fichier soit considéré.
    pub months: u32,
    pub protect_dev: bool,
    pub protect_music: bool,
    pub protect_design: bool,
    /// Taille minimale en Ko (ignore les miettes).
    pub min_kb: u64,
}

impl Default for StaleOptions {
    fn default() -> Self {
        // Les protections viennent des réglages de l'utilisateur (voir `settings`), pas d'un profil figé.
        let s = crate::settings::get();
        StaleOptions { months: 12, protect_dev: s.protect_dev, protect_music: s.protect_music, protect_design: s.protect_design, min_kb: 256 }
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct StaleItem {
    pub path: String,
    pub name: String,
    pub dir: String,
    pub size: u64,
    pub last_activity: i64,
    pub class: &'static str,
    pub class_label: &'static str,
    pub score: u8,
    /// "tres_probable" | "probable" | "a_verifier"
    pub level: &'static str,
    pub reasons: Vec<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProtectedStat {
    pub reason: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct StaleReport {
    pub items: Vec<StaleItem>,
    pub protected: Vec<ProtectedStat>,
    pub scanned_files: u64,
    pub recent_files: u64,
    pub elapsed_ms: u128,
    pub roots: Vec<String>,
    pub months: u32,
}

// ───────────────────────── tables de connaissance ─────────────────────────

const CODE_EXT: &[&str] = &[
    "js", "jsx", "ts", "tsx", "mjs", "cjs", "py", "pyw", "rs", "go", "java", "kt", "kts", "c", "h", "cpp", "hpp", "cc", "cs", "vb", "php", "rb",
    "swift", "dart", "lua", "sql", "sh", "ps1", "bat", "cmd", "html", "htm", "css", "scss", "sass", "less", "vue", "svelte", "json", "yml", "yaml",
    "toml", "xml", "gradle", "ipynb", "r", "scala", "sol", "ino", "sln", "csproj", "vcxproj", "md", "env", "ini", "cfg", "lock", "wasm", "dll",
    "pdb", "ps1xml", "psm1", "tf", "proto", "graphql", "prisma", "ejs", "pug", "twig", "blade", "jsp", "asp", "aspx", "razor", "cshtml",
];
const MUSIC_PROJECT_EXT: &[&str] = &[
    "flp", "fst", "fsc", "als", "alp", "logicx", "ptx", "ptf", "cpr", "rpp", "song", "rns", "npr", "bwproject", "mmpz", "mmp", "ysp", "nki", "nks",
    "nkm", "fxp", "fxb", "vstpreset", "adg", "adv", "h2song", "mid", "midi", "sf2", "sfz", "dwp", "ewp", "reapeaks", "syx", "fpc", "nmsv",
];
const AUDIO_PRO_EXT: &[&str] = &["wav", "flac", "aiff", "aif", "ape", "wv", "w64", "rex", "rx2", "ogg"];
const AUDIO_LOSSY_EXT: &[&str] = &["mp3", "m4a", "aac", "opus", "wma", "amr", "m4b"];
const DESIGN_PROJECT_EXT: &[&str] = &[
    "psd", "psb", "ai", "indd", "xd", "fig", "sketch", "afdesign", "afphoto", "afpub", "kra", "xcf", "cdr", "eps", "aep", "prproj", "drp", "blend",
    "c4d", "ma", "mb", "max", "pdn", "clip", "sai", "ase", "aco", "abr", "ps", "mogrt", "fla", "rp", "dwg",
];
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "heic", "heif", "avif", "svg", "ico", "jfif", "raw", "cr2", "nef", "arw", "dng"];
const VIDEO_EXT: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "3gp", "vob"];
const INSTALLER_EXT: &[&str] = &["exe", "msi", "msix", "appx", "msixbundle", "appxbundle", "iso", "img", "dmg", "apk", "xapk", "pkg", "deb", "rpm", "cab"];
const ARCHIVE_EXT: &[&str] = &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst"];
const TEMP_EXT: &[&str] = &["tmp", "temp", "bak", "old", "log", "dmp", "crdownload", "part", "partial", "download", "chk", "dump", "swp", "torrent", "etl"];
const DOC_EXT: &[&str] = &["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "txt", "rtf", "csv", "epub", "mobi"];

const DEV_FILE_MARKERS: &[&str] = &[
    "package.json", "cargo.toml", "pyproject.toml", "setup.py", "requirements.txt", "go.mod", "pom.xml", "build.gradle", "build.gradle.kts",
    "settings.gradle", "pubspec.yaml", "composer.json", "gemfile", "cmakelists.txt", "makefile", "dockerfile", "docker-compose.yml", "tsconfig.json",
    "angular.json", "manage.py", "artisan", "deno.json", "project.godot", "next.config.js", "vite.config.ts", "vite.config.js", "yarn.lock",
    "pnpm-lock.yaml", "package-lock.json", "firebase.json", "vercel.json", "pubspec.lock", "mix.exs", "stack.yaml", ".gitignore", ".env",
];
const DEV_EXT_MARKERS: &[&str] = &["sln", "csproj", "vcxproj", "uproject", "code-workspace", "xcodeproj", "pro", "ino", "fsproj", "vbproj", "unity", "gradle"];
const DEV_DIR_MARKERS: &[&str] = &[".git", ".hg", ".svn", ".idea", ".vscode", ".vs", "projectsettings", ".github", "node_modules", ".dart_tool", ".expo"];

const DEV_DIR_NAMES: &[&str] = &[
    "projets", "projet", "projects", "project", "repos", "repo", "source", "sources", "src", "dev", "development", "developpement", "workspace",
    "workspaces", "code", "git", "github", "gitlab", "www", "htdocs", "laragon", "xampp", "wamp", "android studio projects", "androidstudioprojects",
    "visual studio projects", "visualstudioprojects", "my projects", "mes projets", "webdev", "web", "apps", "applications", "logiciels", "sites",
];
const MUSIC_DIR_NAMES: &[&str] = &[
    "image-line", "fl studio", "flstudio", "fl studio 20", "fl studio 21", "musique", "music", "sons", "sound", "sounds", "samples", "sample packs",
    "samplepacks", "loops", "beats", "instrumentals", "instrus", "vst", "vstplugins", "presets", "soundfonts", "studio", "ableton", "reaper", "cubase",
    "mixage", "mastering", "stems", "prods", "productions", "kits", "drum kits", "drumkits", "projets musique", "musiques", "audio",
];
const DESIGN_DIR_NAMES: &[&str] = &[
    "affiches", "affiche", "flyers", "flyer", "logos", "logo", "design", "designs", "graphisme", "infographie", "maquettes", "maquette", "branding",
    "visuels", "créa", "crea", "créations", "creations", "pochettes", "covers", "mockups", "miniatures", "thumbnails", "posters", "poster",
    "illustrations", "photoshop", "illustrator", "adobe", "figma", "canva", "bannières", "bannieres", "banners", "cartes", "cards", "identité visuelle",
];
const RESOURCE_WORDS: &[&str] = &[
    "pack", "free", "freebie", "freebies", "bundle", "template", "templates", "mockup", "mockups", "brush", "brushes", "pattern", "patterns",
    "texture", "textures", "overlay", "overlays", "stock", "vector", "vectors", "ressource", "ressources", "resource", "resources", "download",
    "téléchargé", "telecharge", "collection", "kit", "assets", "gfx",
];
const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", ".hg", ".svn", "target", ".gradle", "__pycache__", ".venv", "venv", ".next", ".nuxt", ".dart_tool", ".expo",
    "$recycle.bin", "system volume information", "appdata", "windows", "program files", "program files (x86)", "programdata", ".cache", "pods",
    "deriveddata", ".tox", "bower_components",
];

// ───────────────────────── contexte ─────────────────────────

#[derive(Clone, Copy, Default)]
struct Ctx {
    dev: bool,
    music: bool,
    design: bool,
    resource: bool,
    zone: Zone,
    depth: u8,
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
enum Zone {
    Downloads,
    Desktop,
    Documents,
    Pictures,
    Videos,
    Music,
    #[default]
    Other,
}

struct Shared<'a> {
    opts: StaleOptions,
    now: i64,
    installed: &'a [String],
    progress: &'a Progress,
    /// Mode « médias » : (images, vidéos, audio). Tout fichier non protégé de ces types est retenu, sans critère d'âge.
    media: Option<(bool, bool, bool)>,
}

#[derive(Default)]
struct Acc {
    items: Vec<StaleItem>,
    prot: HashMap<&'static str, (u64, u64)>,
    files: u64,
    recent: u64,
}

impl Acc {
    fn merge(mut self, o: Acc) -> Acc {
        self.items.extend(o.items);
        for (k, v) in o.prot {
            let e = self.prot.entry(k).or_default();
            e.0 += v.0;
            e.1 += v.1;
        }
        self.files += o.files;
        self.recent += o.recent;
        self
    }
    fn protect(&mut self, why: &'static str, size: u64) {
        let e = self.prot.entry(why).or_default();
        e.0 += 1;
        e.1 += size;
    }
}

fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() && i > 0 => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}
fn in_set(set: &[&str], x: &str) -> bool {
    set.iter().any(|s| *s == x)
}
fn has_word(text: &str, words: &[&str]) -> bool {
    let t = text.to_lowercase();
    t.split(|c: char| !c.is_alphanumeric() && c != 'é').any(|w| words.contains(&w))
}

fn child_ctx(parent: Ctx, name: &str, opts: &StaleOptions) -> Ctx {
    let l = name.to_lowercase();
    let mut c = parent;
    c.depth = c.depth.saturating_add(1);
    if DEV_DIR_NAMES.contains(&l.as_str()) {
        c.dev = true;
    }
    if opts.protect_music && MUSIC_DIR_NAMES.contains(&l.as_str()) {
        c.music = true;
    }
    if opts.protect_design && DESIGN_DIR_NAMES.contains(&l.as_str()) {
        c.design = true;
    }
    if has_word(name, RESOURCE_WORDS) {
        c.resource = true;
    }
    c
}

/// Complète le contexte avec ce que contient réellement le dossier (marqueurs de projet).
fn markers(mut c: Ctx, entries: &[Entry], opts: &StaleOptions) -> Ctx {
    for e in entries {
        let l = e.name.to_lowercase();
        if e.is_dir() {
            if DEV_DIR_MARKERS.contains(&l.as_str()) || l.ends_with(".xcodeproj") {
                c.dev = true;
            }
        } else {
            let ext = ext_of(&l);
            if DEV_FILE_MARKERS.contains(&l.as_str()) || in_set(DEV_EXT_MARKERS, &ext) {
                c.dev = true;
            }
            if opts.protect_music && in_set(MUSIC_PROJECT_EXT, &ext) && !matches!(ext.as_str(), "mid" | "midi" | "sf2" | "sfz" | "syx") {
                c.music = true;
            }
            // un vrai projet graphique : source (psd, ai…) dans un dossier qui n'est pas un « pack » téléchargé
            if opts.protect_design && c.depth > 0 && in_set(DESIGN_PROJECT_EXT, &ext) && !c.resource && !has_word(&l, RESOURCE_WORDS) {
                c.design = true;
            }
        }
    }
    c
}

// ───────────────────────── classification et score ─────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Class {
    Installer,
    Archive,
    Temp,
    Image,
    Video,
    Audio,
    DesignResource,
    Doc,
    Other,
}

impl Class {
    fn id(self) -> &'static str {
        match self {
            Class::Installer => "installateur",
            Class::Archive => "archive",
            Class::Temp => "temporaire",
            Class::Image => "image",
            Class::Video => "video",
            Class::Audio => "audio",
            Class::DesignResource => "ressource",
            Class::Doc => "document",
            Class::Other => "autre",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Class::Installer => "Installateur",
            Class::Archive => "Archive",
            Class::Temp => "Temporaire / sauvegarde",
            Class::Image => "Image",
            Class::Video => "Vidéo",
            Class::Audio => "Audio",
            Class::DesignResource => "Ressource graphique téléchargée",
            Class::Doc => "Document",
            Class::Other => "Autre",
        }
    }
    fn base(self) -> f32 {
        match self {
            Class::Temp => 75.0,
            Class::Installer => 55.0,
            Class::Archive => 40.0,
            Class::Image => 30.0,
            Class::Video => 30.0,
            Class::DesignResource => 35.0,
            Class::Audio => 22.0,
            Class::Doc => 15.0,
            Class::Other => 12.0,
        }
    }
}

enum Verdict {
    Protected(&'static str),
    Candidate(Class),
}

fn classify(name_l: &str, ext: &str, ctx: &Ctx, opts: &StaleOptions) -> Verdict {
    if opts.protect_dev && in_set(CODE_EXT, ext) {
        return Verdict::Protected("Code source et fichiers de projet");
    }
    if opts.protect_dev && ctx.dev {
        return Verdict::Protected("Projets de développement");
    }
    if opts.protect_music && in_set(MUSIC_PROJECT_EXT, ext) {
        return Verdict::Protected("Projets et instruments de musique (FL Studio…)");
    }
    if opts.protect_music && ctx.music {
        return Verdict::Protected("Dossiers de musique");
    }
    // Un fichier dont le nom évoque un projet musical (ex. capture « … .flp - FL Studio 20 … ») reste protégé.
    if opts.protect_music && ["fl studio", ".flp", "ableton", "logic pro", "cubase", "reaper", "studio one"].iter().any(|k| name_l.contains(k)) {
        return Verdict::Protected("Fichiers liés à un projet musical");
    }
    if in_set(AUDIO_PRO_EXT, ext) && opts.protect_music {
        return Verdict::Protected("Audio de production (wav, flac…)");
    }
    if in_set(AUDIO_LOSSY_EXT, ext) {
        return Verdict::Candidate(Class::Audio);
    }
    let resource_like = ctx.resource || has_word(name_l, RESOURCE_WORDS);
    if in_set(DESIGN_PROJECT_EXT, ext) {
        if resource_like {
            return Verdict::Candidate(Class::DesignResource);
        }
        if opts.protect_design {
            return Verdict::Protected("Fichiers de création graphique (psd, ai…)");
        }
        return Verdict::Candidate(Class::DesignResource);
    }
    if opts.protect_design && ctx.design {
        return Verdict::Protected("Dossiers de création graphique (affiches, logos…)");
    }
    if in_set(TEMP_EXT, ext) || name_l.ends_with('~') {
        return Verdict::Candidate(Class::Temp);
    }
    if in_set(INSTALLER_EXT, ext) {
        // Un .exe quelconque est souvent un programme portable : installateur seulement si le nom le dit.
        if ext == "exe" && !["setup", "install", "instal", "update", "patch", "installer"].iter().any(|k| name_l.contains(k)) {
            return Verdict::Candidate(Class::Other);
        }
        return Verdict::Candidate(Class::Installer);
    }
    if in_set(ARCHIVE_EXT, ext) {
        return Verdict::Candidate(Class::Archive);
    }
    if in_set(IMAGE_EXT, ext) {
        return Verdict::Candidate(Class::Image);
    }
    if in_set(VIDEO_EXT, ext) {
        return Verdict::Candidate(Class::Video);
    }
    if in_set(DOC_EXT, ext) {
        return Verdict::Candidate(Class::Doc);
    }
    Verdict::Candidate(Class::Other)
}

fn months_ago(now: i64, t: i64) -> f32 {
    ((now - t).max(0) as f32) / (30.44 * 86_400.0)
}

fn strip_copy_suffix(stem: &str) -> Option<String> {
    // « nom (1) » → « nom »
    let s = stem.trim_end();
    if s.ends_with(')') {
        if let Some(i) = s.rfind(" (") {
            let inner = &s[i + 2..s.len() - 1];
            if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) {
                return Some(s[..i].to_string());
            }
        }
    }
    for p in [" - copie", " - copy", " copie", " copy"] {
        if s.to_lowercase().ends_with(p) {
            return Some(s[..s.len() - p.len()].to_string());
        }
    }
    None
}

struct DirView {
    files_l: HashSet<String>,
    dirs_l: HashSet<String>,
}

fn score_item(sh: &Shared, ctx: &Ctx, class: Class, e: &Entry, dir_l: &str, name_l: &str, stem_l: &str, ext: &str, view: &DirView) -> Option<(u8, Vec<String>)> {
    let last = e.mtime.max(if e.atime >= e.mtime && e.atime <= sh.now { e.atime } else { 0 });
    let idle = months_ago(sh.now, last);
    let threshold = sh.opts.months as f32;
    if idle < threshold {
        return None;
    }
    // Les types « personnels » ne sont proposés qu'après une inactivité plus longue.
    let needs = match class {
        Class::Doc | Class::Other => threshold * 2.0,
        Class::Audio | Class::Video => threshold * 1.5,
        _ => threshold,
    };
    if idle < needs {
        return None;
    }
    let mut score = class.base();
    let mut why: Vec<String> = Vec::new();
    let years = idle / 12.0;
    why.push(if years >= 1.0 { format!("Aucune activité depuis {:.1} an{}", years, if years >= 2.0 { "s" } else { "" }).replace('.', ",") } else { format!("Aucune activité depuis {} mois", idle.floor() as u32) });
    score += ((idle - threshold) * 1.5).min(25.0);

    // emplacement
    match ctx.zone {
        Zone::Downloads => {
            score += 12.0;
            why.push("Dans Téléchargements".into());
        }
        Zone::Desktop if ctx.depth <= 1 => {
            score += 6.0;
            why.push("Traîne sur le Bureau".into());
        }
        Zone::Documents => score -= 8.0,
        _ => {}
    }
    if dir_l.contains("\\temp") || dir_l.contains("\\tmp") || dir_l.contains("\\corbeille") {
        score += 8.0;
        why.push("Dans un dossier temporaire".into());
    }
    // taille
    let mb = e.size as f32 / 1_048_576.0;
    score += if mb >= 1024.0 { 12.0 } else if mb >= 200.0 { 8.0 } else if mb >= 50.0 { 5.0 } else if mb >= 10.0 { 3.0 } else { 0.0 };

    // indices propres à la nature du fichier
    match class {
        Class::Installer => {
            why.push("Installateur : sert une fois, puis ne sert plus".into());
            let tok: String = stem_l.chars().take_while(|c| c.is_alphabetic()).collect();
            if tok.chars().count() >= 4 && sh.installed.iter().any(|a| a.contains(&tok)) {
                score += 15.0;
                why.push("Une application de ce nom est déjà installée".into());
            }
        }
        Class::Archive => {
            if view.dirs_l.contains(stem_l) {
                score += 30.0;
                why.push(format!("Déjà extraite dans le dossier « {} »", stem_l));
            }
        }
        Class::Image => {
            let noisy = ["screenshot", "capture", "untitled", "sans titre", "image (", "whatsapp image", "img_", "dsc_", "dsc0", "wallpaper", "download", "telechargement", "unnamed", "new image", "nouvelle image", "photo_20", "pxl_"];
            if noisy.iter().any(|n| name_l.contains(n)) {
                score += 8.0;
                why.push("Nom générique (capture, image téléchargée…)".into());
            }
            why.push("Image hors de tout projet".into());
        }
        Class::DesignResource => why.push("Pack de ressources téléchargé, pas un projet personnel".into()),
        Class::Audio => why.push("Fichier audio isolé, hors de tout dossier de musique".into()),
        Class::Video => why.push("Vidéo non ouverte depuis longtemps".into()),
        Class::Temp => why.push("Fichier temporaire ou de sauvegarde".into()),
        Class::Doc => why.push("Document non rouvert depuis longtemps".into()),
        Class::Other => {}
    }
    // copie d'un fichier voisin
    if let Some(base) = strip_copy_suffix(stem_l) {
        let twin = if ext.is_empty() { base.clone() } else { format!("{}.{}", base, ext) };
        if view.files_l.contains(&twin) {
            score += 25.0;
            why.push("Copie d'un fichier présent dans le même dossier".into());
        }
    }
    let _ = name_l;
    let mut s = score.clamp(0.0, 100.0);
    // Choix personnels (médias, documents, inconnus) : jamais présentés comme « probables », toujours à confirmer.
    if matches!(class, Class::Audio | Class::Video | Class::Doc | Class::Other) {
        s = s.min(54.0);
    }
    if s < 40.0 {
        return None;
    }
    Some((s.round() as u8, why))
}

fn walk(sh: &Shared, dir_ext: &str, dir_name: &str, mut ctx: Ctx, is_root: bool) -> Acc {
    let mut acc = Acc::default();
    if sh.progress.cancelled() {
        return acc;
    }
    let mut entries: Vec<Entry> = Vec::new();
    if winfs::read_dir(dir_ext, |e| entries.push(e)).is_err() {
        return acc;
    }
    if !is_root {
        ctx = child_ctx(ctx, dir_name, &sh.opts);
    }
    // Les marqueurs de projet ne s'appliquent pas à la racine d'une zone (Bureau, Documents…) : un simple fichier posé
    // là ne doit pas faire passer tout le dossier pour un projet.
    if !is_root {
        ctx = markers(ctx, &entries, &sh.opts);
    }
    sh.progress.dirs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    sh.progress.files.fetch_add(entries.len() as u64, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut c) = sh.progress.current.try_lock() {
        if sh.progress.dirs.load(std::sync::atomic::Ordering::Relaxed) % 64 == 0 {
            *c = winfs::display_path(dir_ext);
        }
    }

    let view = DirView {
        files_l: entries.iter().filter(|e| !e.is_dir()).map(|e| e.name.to_lowercase()).collect(),
        dirs_l: entries.iter().filter(|e| e.is_dir()).map(|e| e.name.to_lowercase()).collect(),
    };
    let dir_l = winfs::display_path(dir_ext).to_lowercase();
    let min = sh.opts.min_kb * 1024;

    for e in entries.iter().filter(|e| !e.is_dir() && !e.is_reparse()) {
        acc.files += 1;
        if e.size < min || e.is_cloud_only() {
            continue;
        }
        let name_l = e.name.to_lowercase();
        let ext = ext_of(&name_l);
        let stem_l = if ext.is_empty() { name_l.clone() } else { name_l[..name_l.len() - ext.len() - 1].to_string() };
        if let Some((im, vi, au)) = sh.media {
            match classify(&name_l, &ext, &ctx, &sh.opts) {
                Verdict::Protected(why) => acc.protect(why, e.size),
                Verdict::Candidate(class) => {
                    if matches!((class, im, vi, au), (Class::Image, true, _, _) | (Class::Video, _, true, _) | (Class::Audio, _, _, true)) {
                        acc.items.push(StaleItem {
                            dir: winfs::display_path(dir_ext),
                            name: e.name.clone(),
                            path: winfs::display_path(&winfs::join(dir_ext, &e.name)),
                            size: e.size,
                            last_activity: e.mtime,
                            class: class.id(),
                            class_label: class.label(),
                            score: 60,
                            level: "probable",
                            reasons: vec![format!("{} hors de tout projet protégé", class.label())],
                        });
                    }
                }
            }
            continue;
        }
        match classify(&name_l, &ext, &ctx, &sh.opts) {
            Verdict::Protected(why) => acc.protect(why, e.size),
            Verdict::Candidate(class) => match score_item(sh, &ctx, class, e, &dir_l, &name_l, &stem_l, &ext, &view) {
                Some((score, reasons)) => {
                    let path = winfs::display_path(&winfs::join(dir_ext, &e.name));
                    let last = e.mtime.max(if e.atime >= e.mtime && e.atime <= sh.now { e.atime } else { 0 });
                    acc.items.push(StaleItem {
                        dir: winfs::display_path(dir_ext),
                        name: e.name.clone(),
                        path,
                        size: e.size,
                        last_activity: last,
                        class: class.id(),
                        class_label: class.label(),
                        score,
                        level: if score >= 70 { "tres_probable" } else if score >= 55 { "probable" } else { "a_verifier" },
                        reasons,
                    });
                }
                None => acc.recent += 1,
            },
        }
    }

    let subs: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.is_dir() && !e.is_reparse() && !SKIP_DIRS.contains(&e.name.to_lowercase().as_str()))
        .collect();
    let children = subs
        .par_iter()
        .map(|e| walk(sh, &winfs::join(dir_ext, &e.name), &e.name, ctx, false))
        .reduce(Acc::default, Acc::merge);
    acc.merge(children)
}

const REBUILDABLE: &[&str] = &[
    "node_modules", "target", ".next", ".nuxt", ".gradle", "__pycache__", ".venv", "venv", ".dart_tool", "pods", "deriveddata", ".cache", ".tox",
    "bower_components", ".parcel-cache", ".expo", "dist", "build", ".turbo", ".svelte-kit",
];
/// Dossiers techniques cachés des outils : pas de l'utilisateur, jamais proposés comme doublons.
const TOOL_DIRS: &[&str] = &[".cargo", ".nuget", ".android", ".vscode", ".codex", ".claude", ".gemini", "fvm", ".pub-cache", "site-packages", "vendor", ".m2", ".rustup", ".npm", ".yarn", "appdata"];

/// Garde-fou sur les NOMS de chemin (rapide, sans accès disque).
/// Renvoie une raison de prudence si le chemin est dans un projet de code, de musique ou de création graphique,
/// ou s'il s'agit lui-même d'un tel fichier. Les dossiers reconstructibles (node_modules, target…) n'en font pas partie.
pub fn hint_fast(path: &str) -> Option<&'static str> {
    let p = winfs::display_path(path).to_lowercase();
    let comps: Vec<&str> = p.split('\\').filter(|c| !c.is_empty()).collect();
    if comps.len() < 2 {
        return None;
    }
    if comps.iter().any(|c| REBUILDABLE.contains(c)) {
        return None;
    }
    let name = *comps.last().unwrap();
    let ext = ext_of(name);
    // Ce que l'utilisateur a choisi de protéger (réglable à tout moment) : le reste n'est pas signalé.
    // Un chemin qu'il a autorisé dans le chat n'est plus signalé du tout.
    if crate::settings::is_allowed(path) {
        return None;
    }
    let st = crate::settings::get();
    for c in &comps[1..] {
        if st.protect_dev && (DEV_DIR_NAMES.contains(c) || *c == ".git") {
            return Some("Cet élément est dans un projet ou un dossier de développement.");
        }
        if st.protect_music && MUSIC_DIR_NAMES.contains(c) {
            return Some("Cet élément est dans un dossier de musique.");
        }
        if st.protect_design && DESIGN_DIR_NAMES.contains(c) {
            return Some("Cet élément est dans un dossier de création graphique.");
        }
    }
    if st.protect_dev && in_set(CODE_EXT, &ext) {
        return Some("Fichier de code ou de configuration de projet.");
    }
    if st.protect_music && in_set(MUSIC_PROJECT_EXT, &ext) {
        return Some("Projet ou instrument de musique (FL Studio…).");
    }
    if st.protect_design && in_set(DESIGN_PROJECT_EXT, &ext) && !has_word(name, RESOURCE_WORDS) {
        return Some("Fichier source de création graphique (psd, ai…).");
    }
    None
}

/// Comme `hint_fast`, mais remonte aussi les dossiers parents à la recherche de vrais marqueurs de projet
/// (.git, package.json, Cargo.toml, *.sln…). Plus lent (accès disque) : réservé à l'aperçu avant suppression.
pub fn hint_markers(path: &str) -> Option<&'static str> {
    if let Some(h) = hint_fast(path) {
        return Some(h);
    }
    if !crate::settings::get().protect_dev || crate::settings::is_allowed(path) {
        return None;
    }
    let p = winfs::display_path(path);
    let lower = p.to_lowercase();
    if lower.split('\\').any(|c| REBUILDABLE.contains(&c)) {
        return None;
    }
    let mut dir = p.as_str();
    for _ in 0..10 {
        let Some((parent, _)) = dir.rsplit_once('\\') else { break };
        if parent.len() <= 3 {
            break;
        }
        dir = parent;
        let ext_dir = winfs::extended(dir);
        let mut found = false;
        let _ = winfs::read_dir(&ext_dir, |e| {
            let l = e.name.to_lowercase();
            if (e.is_dir() && DEV_DIR_MARKERS.contains(&l.as_str()) && l != "node_modules") || (!e.is_dir() && (DEV_FILE_MARKERS.contains(&l.as_str()) || in_set(DEV_EXT_MARKERS, &ext_of(&l)))) {
                found = true;
            }
        });
        if found {
            return Some("Cet élément se trouve dans un projet de développement (marqueur de projet détecté).");
        }
    }
    None
}

/// Vrai si le chemin ne doit jamais être proposé comme doublon ou fichier inutile (outils, projets, caches).
pub fn is_tool_or_project_path(path: &str) -> bool {
    let p = winfs::display_path(path).to_lowercase();
    let comps: Vec<&str> = p.split('\\').filter(|c| !c.is_empty()).collect();
    comps.iter().any(|c| REBUILDABLE.contains(c) || TOOL_DIRS.contains(c) || SKIP_DIRS.contains(c)) || hint_fast(path).is_some()
}

/// Dossiers analysés : dossiers personnels de l'utilisateur + autres disques. Jamais Windows ni les programmes.
pub fn default_roots() -> Vec<(String, &'static str)> {
    let mut v: Vec<(String, &'static str)> = Vec::new();
    if let Ok(p) = std::env::var("USERPROFILE") {
        for (n, _z) in [("Desktop", "bureau"), ("Documents", "documents"), ("Downloads", "telechargements"), ("Pictures", "images"), ("Videos", "videos"), ("Music", "musique"), ("Saved Games", "autre")] {
            let path = winfs::join(&p, n);
            if winfs::attrs(&winfs::extended(&path)).is_ok() {
                v.push((path, n_zone(n)));
            }
        }
        let _ = winfs::read_dir(&winfs::extended(&p), |e| {
            if e.is_dir() && e.name.to_lowercase().starts_with("onedrive") {
                v.push((winfs::join(&p, &e.name), "autre"));
            }
        });
    }
    let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_uppercase();
    for d in crate::sys::fixed_drives() {
        if !d.root.to_uppercase().starts_with(&sys) {
            v.push((d.root.trim_end_matches('\\').to_string() + "\\", "autre"));
        }
    }
    v
}

fn n_zone(n: &str) -> &'static str {
    match n {
        "Desktop" => "bureau",
        "Documents" => "documents",
        "Downloads" => "telechargements",
        "Pictures" => "images",
        "Videos" => "videos",
        "Music" => "musique",
        _ => "autre",
    }
}

pub fn analyze(opts: StaleOptions, installed_apps: &[String], progress: &Progress) -> StaleReport {
    let t0 = std::time::Instant::now();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let roots = default_roots();
    let installed: Vec<String> = installed_apps.iter().map(|a| a.to_lowercase()).collect();
    let sh = Shared { opts, now, installed: &installed, progress, media: None };
    progress.expected.store(0, std::sync::atomic::Ordering::Relaxed);
    let pool = crate::scan::io_pool();
    let acc = pool.install(|| {
        roots
            .par_iter()
            .map(|(p, z)| {
                let zone = match *z {
                    "bureau" => Zone::Desktop,
                    "documents" => Zone::Documents,
                    "telechargements" => Zone::Downloads,
                    "images" => Zone::Pictures,
                    "videos" => Zone::Videos,
                    "musique" => Zone::Music,
                    _ => Zone::Other,
                };
                let mut ctx = Ctx { zone, ..Default::default() };
                if zone == Zone::Music && opts.protect_music {
                    ctx.music = true;
                }
                let name = p.rsplit('\\').next().unwrap_or("");
                let ext = winfs::extended(p.trim_end_matches('\\'));
                walk(&sh, &ext, name, ctx, true)
            })
            .reduce(Acc::default, Acc::merge)
    });
    let mut items = acc.items;
    // Priorité : ce qui rapporte le plus ET dont on est le plus sûr.
    items.sort_by(|a, b| {
        let ka = a.size as f64 * (a.score as f64 / 100.0).powi(2);
        let kb = b.size as f64 * (b.score as f64 / 100.0).powi(2);
        kb.partial_cmp(&ka).unwrap_or(std::cmp::Ordering::Equal)
    });
    items.truncate(4000);
    let mut protected: Vec<ProtectedStat> = acc.prot.into_iter().map(|(k, v)| ProtectedStat { reason: k.to_string(), files: v.0, bytes: v.1 }).collect();
    protected.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    StaleReport {
        items,
        protected,
        scanned_files: acc.files,
        recent_files: acc.recent,
        elapsed_ms: t0.elapsed().as_millis(),
        roots: roots.into_iter().map(|r| r.0).collect(),
        months: opts.months,
    }
}

// ───────────────────────── médias hors projets ─────────────────────────

#[derive(Serialize, Clone, Debug, Default)]
pub struct DirStat {
    pub dir: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct MediaReport {
    pub items: Vec<StaleItem>,
    pub total_bytes: u64,
    pub by_dir: Vec<DirStat>,
    pub protected: Vec<ProtectedStat>,
    pub scanned_files: u64,
}

/// Images / vidéos / audio qui ne sont rattachés à aucun projet protégé, quel que soit leur âge.
/// Les protections viennent des réglages de l'utilisateur (voir `settings`).
pub fn media(images: bool, videos: bool, audio: bool, min_kb: u64, progress: &Progress) -> MediaReport {
    let opts = StaleOptions { months: 0, min_kb, ..Default::default() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let roots = default_roots();
    let installed: Vec<String> = Vec::new();
    let sh = Shared { opts, now, installed: &installed, progress, media: Some((images, videos, audio)) };
    let acc = crate::scan::io_pool().install(|| {
        roots
            .par_iter()
            .map(|(p, z)| {
                let zone = zone_of(z);
                let mut ctx = Ctx { zone, ..Default::default() };
                if zone == Zone::Music && opts.protect_music {
                    ctx.music = true;
                }
                let name = p.rsplit('\\').next().unwrap_or("");
                walk(&sh, &winfs::extended(p.trim_end_matches('\\')), name, ctx, true)
            })
            .reduce(Acc::default, Acc::merge)
    });
    let mut items = acc.items;
    items.sort_by(|a, b| b.size.cmp(&a.size));
    items.truncate(20_000);
    let mut dirs: HashMap<String, (u64, u64)> = HashMap::new();
    for i in &items {
        let e = dirs.entry(i.dir.clone()).or_default();
        e.0 += 1;
        e.1 += i.size;
    }
    let mut by_dir: Vec<DirStat> = dirs.into_iter().map(|(dir, (files, bytes))| DirStat { dir, files, bytes }).collect();
    by_dir.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    by_dir.truncate(40);
    let mut protected: Vec<ProtectedStat> = acc.prot.into_iter().map(|(k, v)| ProtectedStat { reason: k.to_string(), files: v.0, bytes: v.1 }).collect();
    protected.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    MediaReport { total_bytes: items.iter().map(|i| i.size).sum(), items, by_dir, protected, scanned_files: acc.files }
}

fn zone_of(z: &str) -> Zone {
    match z {
        "bureau" => Zone::Desktop,
        "documents" => Zone::Documents,
        "telechargements" => Zone::Downloads,
        "images" => Zone::Pictures,
        "videos" => Zone::Videos,
        "musique" => Zone::Music,
        _ => Zone::Other,
    }
}

// ───────────────────────── recherche de projets ─────────────────────────

#[derive(Serialize, Clone, Debug)]
pub struct Project {
    pub path: String,
    /// "code", "musique" ou "creation"
    pub kind: &'static str,
    pub size: u64,
    pub files: u64,
}

struct ProjShared<'a> {
    opts: StaleOptions,
    want: [bool; 3],
    progress: &'a Progress,
}

fn walk_proj(sh: &ProjShared, dir_ext: &str, name: &str, parent: Ctx, is_root: bool, out: &std::sync::Mutex<Vec<(String, &'static str)>>) {
    if sh.progress.cancelled() {
        return;
    }
    let mut entries: Vec<Entry> = Vec::new();
    if winfs::read_dir(dir_ext, |e| entries.push(e)).is_err() {
        return;
    }
    sh.progress.dirs.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut ctx = if is_root { parent } else { child_ctx(parent, name, &sh.opts) };
    // Les marqueurs de projet ne s'appliquent pas à la racine d'une zone (Bureau, Documents…) : un simple fichier posé
    // là ne doit pas faire passer tout le dossier pour un projet.
    if !is_root {
        ctx = markers(ctx, &entries, &sh.opts);
    }
    let path = winfs::display_path(dir_ext);
    if !is_root {
        let mut found = false;
        let mut push = |k: &'static str| {
            out.lock().unwrap().push((path.clone(), k));
            found = true;
        };
        // Un dossier « Projects » dans un dossier de musique ou de création n'est pas un projet de code.
        if ctx.dev && !parent.dev && !parent.music && !parent.design && !ctx.music && !ctx.design && sh.want[0] { push("code"); }
        // Dans un projet de code, un dossier « audio » ou « studio » est du code, pas un projet musical.
        if ctx.music && !parent.music && !ctx.dev && sh.want[1] { push("musique"); }
        if ctx.design && !parent.design && !ctx.dev && sh.want[2] { push("creation"); }
        if found {
            return; // on ne descend pas dans un projet déjà repéré
        }
    }
    let subs: Vec<&Entry> = entries.iter().filter(|e| e.is_dir() && !e.is_reparse() && !SKIP_DIRS.contains(&e.name.to_lowercase().as_str())).collect();
    subs.par_iter().for_each(|e| walk_proj(sh, &winfs::join(dir_ext, &e.name), &e.name, ctx, false, out));
}

/// Retrouve les racines de projets (code, musique, création graphique) dans les dossiers personnels et les autres disques.
/// `kind` : "code", "musique", "creation" ou "tous". La détection ne dépend PAS des protections choisies.
pub fn find_projects(kind: &str, progress: &Progress) -> Vec<Project> {
    let want = [matches!(kind, "code" | "tous"), matches!(kind, "musique" | "tous"), matches!(kind, "creation" | "tous")];
    let opts = StaleOptions { months: 0, protect_dev: true, protect_music: true, protect_design: true, min_kb: 0 };
    let sh = ProjShared { opts, want, progress };
    let out = std::sync::Mutex::new(Vec::new());
    crate::scan::io_pool().install(|| {
        default_roots().par_iter().for_each(|(p, z)| {
            let ctx = Ctx { zone: zone_of(z), ..Default::default() };
            let name = p.rsplit('\\').next().unwrap_or("");
            walk_proj(&sh, &winfs::extended(p.trim_end_matches('\\')), name, ctx, true, &out);
        });
    });
    let found = out.into_inner().unwrap();
    let mut res: Vec<Project> = crate::scan::io_pool().install(|| {
        found
            .par_iter()
            .map(|(p, k)| {
                let (size, files) = crate::scan::measure(p);
                Project { path: p.clone(), kind: k, size, files }
            })
            .collect()
    });
    res.sort_by(|a, b| b.size.cmp(&a.size));
    res.truncate(200);
    res
}

// ───────────────────────── domaines (rangement du Bureau) ─────────────────────────

/// Domaine d'un fichier d'après son extension.
pub fn domain_of_file(name: &str) -> &'static str {
    let ext = ext_of(&name.to_lowercase());
    if in_set(MUSIC_PROJECT_EXT, &ext) || in_set(AUDIO_PRO_EXT, &ext) || in_set(AUDIO_LOSSY_EXT, &ext) {
        "Musique"
    } else if in_set(DESIGN_PROJECT_EXT, &ext) {
        "Création graphique"
    } else if in_set(IMAGE_EXT, &ext) {
        "Images"
    } else if in_set(VIDEO_EXT, &ext) {
        "Vidéos"
    } else if in_set(DOC_EXT, &ext) {
        "Documents"
    } else if in_set(ARCHIVE_EXT, &ext) {
        "Archives"
    } else if in_set(INSTALLER_EXT, &ext) {
        "Installateurs"
    } else if in_set(CODE_EXT, &ext) {
        "Développement"
    } else {
        "Autres"
    }
}

/// Domaine d'un dossier : par marqueurs de projet (code, musique, création) sinon par type majoritaire de son contenu.
/// Le booléen indique un vrai projet (déplacer un projet peut casser des chemins : il est traité à part).
pub fn domain_of_dir(dir_ext: &str, name: &str) -> (&'static str, bool) {
    let opts = StaleOptions { months: 0, protect_dev: true, protect_music: true, protect_design: true, min_kb: 0 };
    let mut entries: Vec<Entry> = Vec::new();
    let _ = winfs::read_dir(dir_ext, |e| entries.push(e));
    let ctx = markers(child_ctx(Ctx::default(), name, &opts), &entries, &opts);
    if ctx.dev {
        return ("Développement", true);
    }
    if ctx.music {
        return ("Musique", false);
    }
    if ctx.design {
        return ("Création graphique", false);
    }
    // Vote : fichiers des deux premiers niveaux (au plus 3000).
    let mut votes: HashMap<&'static str, u32> = HashMap::new();
    let mut seen = 0u32;
    let mut stack: Vec<(String, u8)> = vec![(dir_ext.to_string(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let es: Vec<Entry> = if d == dir_ext { entries.clone() } else {
            let mut v = Vec::new();
            let _ = winfs::read_dir(&d, |e| v.push(e));
            v
        };
        for e in es {
            if seen > 3000 { break; }
            if e.is_dir() {
                if depth < 1 && !e.is_reparse() { stack.push((winfs::join(&d, &e.name), depth + 1)); }
            } else {
                seen += 1;
                *votes.entry(domain_of_file(&e.name)).or_default() += 1;
            }
        }
    }
    let total: u32 = votes.values().sum();
    if total == 0 {
        return ("Dossiers divers", false);
    }
    let (best, n) = votes.iter().max_by_key(|(_, n)| **n).map(|(k, n)| (*k, *n)).unwrap();
    if best != "Autres" && n * 100 >= total * 60 { (best, false) } else { ("Dossiers divers", false) }
}
