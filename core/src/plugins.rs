//! Plugins audio installés (VST, VST3, CLAP) dans les emplacements standard, y compris ceux de FL Studio.

use crate::{scan, winfs};
use rayon::prelude::*;
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct PluginEntry {
    pub path: String,
    pub name: String,
    /// "vst2", "vst3" ou "clap"
    pub kind: &'static str,
    pub size: u64,
}

fn env(n: &str) -> Option<String> {
    std::env::var(n).ok().filter(|v| !v.is_empty())
}

pub fn find() -> Vec<PluginEntry> {
    let mut dirs: Vec<(String, &'static str)> = Vec::new();
    for pf in [env("ProgramFiles"), env("ProgramW6432"), env("ProgramFiles(x86)")].into_iter().flatten() {
        for (sub, kind) in [
            ("Common Files\\VST3", "vst3"),
            ("Common Files\\VST2", "vst2"),
            ("Common Files\\CLAP", "clap"),
            ("VSTPlugins", "vst2"),
            ("Steinberg\\VSTPlugins", "vst2"),
            ("Steinberg\\VST3", "vst3"),
        ] {
            dirs.push((winfs::join(&pf, sub), kind));
        }
        // Image-Line : seuls les dossiers VST/VST3 comptent (le dossier Fruity contient les modules internes de FL Studio).
        let il = winfs::join(&pf, "Image-Line");
        let _ = winfs::read_dir(&winfs::extended(&il), |e| {
            if e.is_dir() && e.name.to_lowercase().starts_with("fl studio") {
                let base = winfs::join(&il, &e.name);
                dirs.push((winfs::join(&base, "Plugins\\VST"), "vst2"));
                dirs.push((winfs::join(&base, "Plugins\\VST3"), "vst3"));
            }
        });
    }
    if let Some(l) = env("LOCALAPPDATA") {
        dirs.push((winfs::join(&l, "Programs\\Common\\VST3"), "vst3"));
        dirs.push((winfs::join(&l, "Programs\\Common\\CLAP"), "clap"));
    }
    dirs.sort_by_key(|d| d.0.to_lowercase());
    dirs.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));

    let mut found: Vec<(String, String, &'static str)> = Vec::new();
    for (dir, kind) in &dirs {
        let _ = winfs::read_dir(&winfs::extended(dir), |e| {
            let l = e.name.to_lowercase();
            let ok = e.is_dir() || l.ends_with(".dll") || l.ends_with(".vst3") || l.ends_with(".clap");
            if ok && l != "desktop.ini" && !e.is_reparse() {
                found.push((winfs::join(dir, &e.name), e.name.clone(), kind));
            }
        });
    }
    let mut out: Vec<PluginEntry> = scan::io_pool().install(|| {
        found
            .par_iter()
            .map(|(p, n, k)| PluginEntry { path: p.clone(), name: n.clone(), kind: k, size: scan::measure(p).0 })
            .collect()
    });
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}
