//! Outil en ligne de commande : sert à tester et diagnostiquer le moteur sans interface.
//!   libera-cli scan <racine> [rapide|standard|approfondie]
//!   libera-cli junk
//!   libera-cli apps
//!   libera-cli preview <chemin>...
//!   libera-cli delete <chemin>... --oui [--droits] [--redemarrage]
//!   libera-cli lockers <fichier>

use libera_core::{self as core, cleaner, scan, Mode};
use std::os::windows::fs::OpenOptionsExt;
use std::sync::atomic::Ordering;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("aide");
    match cmd {
        "scan" => {
            let root = args.get(1).cloned().unwrap_or_else(|| "C:\\".into());
            let mode = match args.get(2).map(|s| s.as_str()) {
                Some("rapide") => Mode::Rapide,
                Some("approfondie") => Mode::Approfondie,
                Some("experimentale") => Mode::Experimentale,
                _ => Mode::Standard,
            };
            let progress = scan::Progress::default();
            let a = core::analyze(mode, &root, &progress, &|p| eprintln!("» {}", p));
            if let Some(n) = &a.notice {
                println!("Info : {}", n);
            }
            if let Some(d) = &a.disk {
                println!("Disque {} : {} utilisés sur {} ({} libres)", d.root, core::format_size(d.used), core::format_size(d.total), core::format_size(d.free));
            }
            if let Some(s) = &a.scan {
                let root_node = &s.nodes[0];
                println!(
                    "\nParcours : {} dossiers, {} fichiers, {} analysés en {:.1} s ({} dossiers inaccessibles, {} en ligne seulement)",
                    s.nodes.len(), root_node.files, core::format_size(root_node.size), s.elapsed_ms as f64 / 1000.0, s.denied_dirs, core::format_size(s.cloud_bytes)
                );
                println!("\nTop dossiers :");
                for c in s.children(0).iter().take(12) {
                    println!("  {:>10}  {:5.1}%  {}", core::format_size(c.size), c.pct, c.name);
                }
                println!("\nTop fichiers :");
                for f in s.big_files.iter().take(10) {
                    println!("  {:>10}  {}", core::format_size(f.size), core::winfs::display_path(&f.path));
                }
                println!("\nDossiers reconstructibles (≥ 100 Mo) :");
                for c in s.rebuildable(100 << 20).iter().take(8) {
                    println!("  {:>10}  {}", core::format_size(c.size), c.path);
                }
            }
            println!("\nZones nettoyables :");
            let mut total = 0;
            for j in &a.junk {
                total += j.size;
                println!("  {:>10}  {:<48} {}", core::format_size(j.size), j.cat.name, if j.cat.needs_admin { "[admin]" } else { "" });
            }
            println!("  {:>10}  TOTAL", core::format_size(total));
            println!("\nApplications ({}) — les plus lourdes :", a.apps.len());
            for x in a.apps.iter().take(8) {
                println!("  {:>10}  {}", core::format_size(x.size), x.name);
            }
            if !a.dupes.is_empty() {
                let w: u64 = a.dupes.iter().map(|g| g.wasted).sum();
                println!("\nDoublons : {} groupes, {} récupérables", a.dupes.len(), core::format_size(w));
                for g in a.dupes.iter().take(5) {
                    println!("  {:>10} × {}  {}", core::format_size(g.size), g.files.len(), g.files[0].path);
                }
            }
            println!("\nTemps total : {:.1} s", a.elapsed_ms as f64 / 1000.0);
        }
        "stale" => {
            let months: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(12);
            let names: Vec<String> = core::apps::list(&|_| None, false).into_iter().map(|a| a.name).collect();
            let progress = scan::Progress::default();
            let opts = core::stale::StaleOptions { months, ..Default::default() };
            let r = core::stale::analyze(opts, &names, &progress);
            println!("Dossiers analysés : {}", r.roots.join(" | "));
            println!("{} fichiers lus en {:.1} s ; {} récents ignorés ; {} propositions\n", r.scanned_files, r.elapsed_ms as f64 / 1000.0, r.recent_files, r.items.len());
            let total: u64 = r.items.iter().map(|i| i.size).sum();
            let sure: u64 = r.items.iter().filter(|i| i.level == "tres_probable").map(|i| i.size).sum();
            println!("Potentiel : {} (dont très probable : {})\n", core::format_size(total), core::format_size(sure));
            println!("PROTÉGÉS (jamais proposés) :");
            for p in &r.protected {
                println!("  {:>10}  {:>8} fichiers  {}", core::format_size(p.bytes), p.files, p.reason);
            }
            println!("\nTOP propositions :");
            for i in r.items.iter().take(40) {
                println!("  [{:>3}] {:<13} {:>9}  {}\n         {}", i.score, i.level, core::format_size(i.size), i.path, i.reasons.join(" · "));
            }
        }
        "plugins" => {
            let list = core::plugins::find();
            for p in list.iter().take(40) {
                println!("{:>10}  {:<5} {}", core::format_size(p.size), p.kind, p.path);
            }
            println!("{} plugins, {}", list.len(), core::format_size(list.iter().map(|p| p.size).sum()));
        }
        "projets" => {
            let kind = args.get(1).map(|s| s.as_str()).unwrap_or("tous");
            for p in core::stale::find_projects(kind, &scan::Progress::default()).iter().take(30) {
                println!("{:>10}  {:<9} {}", core::format_size(p.size), p.kind, p.path);
            }
        }
        "medias" => {
            let r = core::stale::media(true, true, false, 100, &scan::Progress::default());
            println!("{} fichiers, {} hors de tout projet protégé", r.items.len(), core::format_size(r.total_bytes));
            for d in r.by_dir.iter().take(10) {
                println!("{:>10}  {:>6} fichiers  {}", core::format_size(d.bytes), d.files, d.dir);
            }
            for p in &r.protected {
                println!("protégés : {:>10}  {}", core::format_size(p.bytes), p.reason);
            }
        }
        "expliquer" => {
            let e = core::explain::describe(&args[1]);
            println!("{} | {} | {} fichiers | régénérable : {}", e.path, core::format_size(e.size), e.files, e.regenerable);
            if let Some(n) = &e.note { println!("note : {}", n); }
            for c in &e.biggest { println!("  {:>10}  {}", core::format_size(c.size), c.name); }
        }
        "archiver" => {
            // archiver <fichier.zip> <source>...
            let prog = core::archive::ArchiveProgress::default();
            match core::archive::create(&args[2..], &args[1], &prog) {
                Ok(r) => println!("{}", serde_json::to_string_pretty(&r).unwrap()),
                Err(e) => { eprintln!("ÉCHEC : {}", e); std::process::exit(1); }
            }
        }
        "organiser" => {
            // organiser <dossier> <element:Domaine>... [--oui]
            let apply = args.iter().any(|a| a == "--oui");
            let pairs: Vec<(String, String)> = args[2..].iter().filter(|a| !a.starts_with("--")).filter_map(|a| a.split_once(':').map(|(x, y)| (x.to_string(), y.to_string()))).collect();
            let plan = core::organize::build_plan(&args[1], &pairs).unwrap();
            println!("déplacements : {}", plan.moves.len());
            for m in &plan.moves { println!("  {} -> {} [{}]", m.from, m.to, m.risk); }
            for f in &plan.fixes { println!("  raccourci à réparer : {} : {} -> {}", f.lnk, f.old_target, f.new_target); }
            for k in &plan.kept { println!("  laissé : {} ({})", k.0, k.1); }
            if apply {
                let r = core::organize::apply(&plan.moves, &plan.fixes);
                println!("appliqué : {} déplacés, {} échecs, {} raccourci(s) réparé(s)\njournal : {}", r.moved.len(), r.failed.len(), r.fixed_shortcuts, r.journal);
            }
        }
        "annuler" => {
            let j = core::organize::last_journal().expect("aucun journal");
            let r = core::organize::undo(&j).unwrap();
            println!("annulé : {} remis en place, {} échecs, {} raccourci(s) restauré(s)", r.moved.len(), r.failed.len(), r.fixed_shortcuts);
        }
        "junk" => {
            for j in core::catalog::measure_all(None) {
                println!("{:>10}  {:<48} {}", core::format_size(j.size), j.cat.name, j.cat.id);
            }
        }
        "apps" => {
            for x in core::apps::list(&|_| None, true).iter().take(25) {
                println!("{:>10}  {}  {}", core::format_size(x.size), x.name, if x.size_measured { "" } else { "(estimée)" });
            }
        }
        "preview" => {
            let reqs: Vec<cleaner::PreviewReq> = args[1..].iter().map(|p| cleaner::PreviewReq { path: p.clone(), size: None, files: None }).collect();
            for p in cleaner::preview(&reqs) {
                println!("{:>10}  {:?}  {}", core::format_size(p.size), p.verdict, p.path);
            }
        }
        "piege" => {
            // Fabrique un dossier de test contenant des cas que l'Explorateur ne sait pas supprimer.
            use core::winfs::{extended, wide};
            use std::os::windows::io::AsRawHandle;
            let base = args.get(1).expect("dossier de test");
            std::fs::create_dir_all(base).unwrap();
            let b = extended(base);
            // 1. nom avec point et espace final, 2. nom réservé « nul », 3. chemin > 260 caractères
            // 4. fichier lecture seule + système, 5. fichier ouvert sans partage (verrouillé), 6. jonction vers un dossier à ne PAS toucher
            let mk = |p: &str| {
                let w = wide(p);
                unsafe { windows_sys::Win32::Storage::FileSystem::CreateDirectoryW(w.as_ptr(), std::ptr::null()) };
            };
            mk(&format!(r"{}\dossier piege. ", b));
            std::fs::write(format!(r"{}\dossier piege. \contenu.txt", b), "x").unwrap();
            mk(&format!(r"{}\nul", b));
            std::fs::write(format!(r"{}\nul\aux", b), "x").unwrap();
            let mut deep = format!(r"{}\long", b);
            mk(&deep);
            for _ in 0..30 {
                deep = format!(r"{}\{}", deep, "d".repeat(20));
                mk(&deep);
            }
            std::fs::write(format!(r"{}\fichier_profond.bin", deep), vec![1u8; 4096]).unwrap();
            let ro = format!(r"{}\lecture_seule.txt", b);
            std::fs::write(&ro, "ro").unwrap();
            core::winfs::set_attrs(&ro, 0x1 | 0x2 | 0x4); // lecture seule + caché + système
            // La cible de la jonction est volontairement HORS du dossier piégé : elle doit survivre.
            let keep = format!(r"{}_a_preserver", base);
            std::fs::create_dir_all(&keep).unwrap();
            std::fs::write(format!(r"{}\precieux.txt", keep), "NE PAS SUPPRIMER").unwrap();
            let _ = std::process::Command::new("cmd").args(["/C", "mklink", "/J", &format!(r"{}\jonction", base), &keep]).output();
            // fichier verrouillé : on garde le handle ouvert sans FILE_SHARE_DELETE
            let locked = format!(r"{}\verrouille.dat", base);
            std::fs::write(&locked, vec![7u8; 1 << 20]).unwrap();
            let f = std::fs::OpenOptions::new().read(true).write(true).share_mode(1).open(&locked).unwrap();
            let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(90);
            println!("Piège prêt dans {}. Fichier verrouillé tenu ouvert {} s (PID {}).", base, secs, std::process::id());
            let _ = f.as_raw_handle();
            std::thread::sleep(std::time::Duration::from_secs(secs));
        }
        "lockers" => {
            for l in cleaner::lockers(&args[1]) {
                println!("{}", l);
            }
        }
        "delete" => {
            let confirm = args.iter().any(|a| a == "--oui");
            let paths: Vec<String> = args[1..].iter().filter(|a| !a.starts_with("--")).cloned().collect();
            if !confirm {
                eprintln!("Ajoute --oui pour confirmer la suppression DÉFINITIVE de :");
                for p in &paths {
                    eprintln!("  {}", p);
                }
                std::process::exit(2);
            }
            let opts = cleaner::DeleteOptions {
                fix_permissions: args.iter().any(|a| a == "--droits"),
                schedule_on_reboot: args.iter().any(|a| a == "--redemarrage"),
                ..Default::default()
            };
            let prog = cleaner::DelProgress::default();
            let r = cleaner::delete_paths(&paths, opts, &prog);
            println!("{}", serde_json::to_string_pretty(&r).unwrap());
            println!("Libéré : {} ({} fichiers) en {} ms", core::format_size(prog.bytes.load(Ordering::Relaxed)), r.files_deleted, r.elapsed_ms);
        }
        _ => println!("Commandes : scan <racine> [rapide|standard|approfondie] | junk | apps | preview <chemin>… | delete <chemin>… --oui | lockers <fichier>"),
    }
}
