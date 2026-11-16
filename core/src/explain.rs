//! « À quoi sert ce dossier ? » : taille, contenu principal, dernières modifications, et une note quand le dossier est connu.
//! L'assistant s'en sert pour répondre honnêtement (et dire « je ne sais pas » quand c'est le cas) au lieu de deviner.

use crate::{scan, winfs};
use serde::Serialize;

#[derive(Serialize, Clone, Debug)]
pub struct Child {
    pub name: String,
    pub size: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Explained {
    pub path: String,
    pub exists: bool,
    pub is_dir: bool,
    pub size: u64,
    pub files: u64,
    pub biggest: Vec<Child>,
    pub note: Option<String>,
    /// "oui" (recréé automatiquement), "non" (données personnelles ou vitales), "inconnu"
    pub regenerable: &'static str,
}

/// (fragment de chemin en minuscules, note, régénérable)
const KNOWN: &[(&str, &str, &str)] = &[
    ("claude-data\\", "Données locales de l'application de bureau Claude. Le sous-dossier vm_bundles contient de gros fichiers d'image disque virtuelle (.vhdx) : l'application les recrée si elle en a besoin. Le reste (réglages, historique local) mieux vaut le garder ; pour tout retirer proprement, désinstalle l'application plutôt que de supprimer à la main.", "inconnu"),
    ("\\android\\sdk", "Kit de développement Android. Les composants se retirent depuis le SDK Manager d'Android Studio ; les images système (system-images) sont volumineuses et se retéléchargent.", "oui"),
    (".android\\avd", "Émulateurs Android (appareils virtuels). Supprimer un AVD le retire de l'émulateur ; on peut le recréer.", "oui"),
    ("bluestacks", "Émulateur Android BlueStacks : ses disques virtuels sont volumineux. Mieux vaut le désinstaller que supprimer à la main.", "oui"),
    ("\\docker", "Données Docker (images, conteneurs, volumes). Supprimer à la main peut faire perdre des volumes : utiliser « docker system prune » ou Docker Desktop.", "inconnu"),
    ("\\wsl", "Distribution Linux (WSL) : le disque ext4.vhdx contient les fichiers de cette distribution. Supprimer = perdre ce qu'elle contient.", "non"),
    ("\\.gradle", "Cache de compilation Gradle (Java/Android) : recréé automatiquement, au prix d'un nouveau téléchargement.", "oui"),
    ("\\.nuget", "Cache des paquets NuGet (.NET) : restauré à la prochaine compilation.", "oui"),
    ("\\.cargo\\registry", "Cache des dépendances Rust : retéléchargé à la prochaine compilation.", "oui"),
    ("\\node_modules", "Dépendances JavaScript : recréées par `npm install`.", "oui"),
    ("\\.next", "Sortie de compilation Next.js : recréée par la compilation.", "oui"),
    ("\\target", "Dossier de compilation (Rust/Java) : recréé par la compilation.", "oui"),
    ("\\windows\\installer", "Fichiers de l'installateur Windows : NE PAS SUPPRIMER, ils servent à réparer et désinstaller les programmes.", "non"),
    ("\\winsxs", "Magasin de composants Windows : ne se nettoie qu'avec l'outil DISM (jamais à la main).", "non"),
    ("programdata\\microsoft\\search", "Index de la recherche Windows : recréé automatiquement.", "oui"),
    ("hiberfil.sys", "Fichier d'hibernation Windows : se désactive avec `powercfg -h off`.", "oui"),
    ("pagefile.sys", "Mémoire virtuelle Windows : géré par le système, ne pas supprimer.", "non"),
    ("appdata\\local\\temp", "Fichiers temporaires : peuvent être supprimés (ceux en cours d'utilisation sont ignorés).", "oui"),
    ("\\onedrive", "Dossier synchronisé OneDrive : supprimer ici supprime aussi dans le cloud.", "non"),
    ("adobe photoshop", "Préférences, pinceaux et caches de Photoshop. Les gros fichiers .psp sont des jeux de pinceaux.", "inconnu"),
    ("\\.vscode", "Extensions et réglages de Visual Studio Code.", "inconnu"),
    ("\\.cache", "Caches d'outils de développement : recréés au besoin.", "oui"),
];

pub fn describe(path: &str) -> Explained {
    let disp = winfs::display_path(path).trim_end_matches('\\').to_string();
    let ext = winfs::extended(&disp);
    let attrs = match winfs::attrs(&ext) {
        Ok(a) => a,
        Err(_) => return Explained { path: disp, exists: false, is_dir: false, size: 0, files: 0, biggest: vec![], note: None, regenerable: "inconnu" },
    };
    let is_dir = attrs & 0x10 != 0;
    let lower = format!("{}\\", disp.to_lowercase());
    let known = KNOWN.iter().find(|k| lower.contains(k.0));
    let (mut size, mut files, mut biggest) = (0, 1, Vec::new());
    if is_dir {
        let s = scan::scan(&disp, u64::MAX, &scan::Progress::default());
        size = s.nodes[0].size;
        files = s.nodes[0].files;
        biggest = s.children(0).into_iter().take(8).map(|c| Child { name: c.name, size: c.size }).collect();
        // fichiers directement dans le dossier
        let mut direct: Vec<Child> = scan::list_files(&disp).into_iter().take(5).map(|f| Child { name: f.path.rsplit('\\').next().unwrap_or("").to_string(), size: f.size }).collect();
        biggest.append(&mut direct);
        biggest.sort_by(|a, b| b.size.cmp(&a.size));
        biggest.truncate(8);
    } else if let Some(s) = winfs::file_size(&ext) {
        size = s;
    }
    Explained {
        path: disp,
        exists: true,
        is_dir,
        size,
        files,
        biggest,
        note: known.map(|k| k.1.to_string()),
        regenerable: known.map(|k| k.2).unwrap_or("inconnu"),
    }
}
