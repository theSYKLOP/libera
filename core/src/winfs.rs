//! Couche basse : appels directs à l'API Windows (listage rapide, attributs, chemins étendus).

use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;

/// Chaîne UTF-16 terminée par NUL.
pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

pub fn from_wide(w: &[u16]) -> String {
    OsString::from_wide(w).to_string_lossy().into_owned()
}

/// Convertit un chemin en « chemin étendu » `\\?\`.
/// Il n'est volontairement PAS normalisé : les points et espaces en fin de nom,
/// les noms réservés (nul, con, aux…) et les chemins > 260 caractères restent donc accessibles.
pub fn extended(path: &str) -> String {
    let p = path.replace('/', "\\");
    if p.starts_with(r"\\?\") {
        return p;
    }
    if let Some(rest) = p.strip_prefix(r"\\") {
        return format!(r"\\?\UNC\{}", rest);
    }
    format!(r"\\?\{}", p)
}

/// Inverse de `extended` : chemin lisible pour l'affichage et les outils externes.
pub fn display_path(path: &str) -> String {
    if let Some(r) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{}", r);
    }
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}

pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') {
        format!("{}{}", dir, name)
    } else {
        format!("{}\\{}", dir, name)
    }
}

const CLOUD_MASK: u32 = FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | FILE_ATTRIBUTE_RECALL_ON_OPEN | FILE_ATTRIBUTE_OFFLINE;

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub attrs: u32,
    pub size: u64,
    /// Dernière modification, en secondes Unix.
    pub mtime: i64,
    /// Dernier accès, en secondes Unix (peu fiable : Windows le met rarement à jour).
    pub atime: i64,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_DIRECTORY != 0
    }
    pub fn is_reparse(&self) -> bool {
        self.attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    /// Fichier « à la demande » (OneDrive…) : il n'occupe pas d'espace sur le disque.
    pub fn is_cloud_only(&self) -> bool {
        self.attrs & CLOUD_MASK != 0
    }
}

pub fn filetime_to_unix(low: u32, high: u32) -> i64 {
    let t = ((high as u64) << 32) | low as u64;
    if t == 0 {
        return 0;
    }
    (t / 10_000_000) as i64 - 11_644_473_600
}

/// Liste un dossier (chemin étendu). `Err(code)` = code d'erreur Windows.
pub fn read_dir(dir_ext: &str, mut f: impl FnMut(Entry)) -> Result<(), u32> {
    let pattern = wide(&join(dir_ext, "*"));
    unsafe {
        let mut data: WIN32_FIND_DATAW = std::mem::zeroed();
        let h = FindFirstFileExW(
            pattern.as_ptr(),
            FindExInfoBasic,
            &mut data as *mut _ as *mut c_void,
            FindExSearchNameMatch,
            std::ptr::null(),
            FIND_FIRST_EX_LARGE_FETCH,
        );
        if h == INVALID_HANDLE_VALUE {
            let e = GetLastError();
            return if e == ERROR_FILE_NOT_FOUND { Ok(()) } else { Err(e) };
        }
        loop {
            let len = data.cFileName.iter().position(|&c| c == 0).unwrap_or(data.cFileName.len());
            let name = &data.cFileName[..len];
            let dots = name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16];
            if !dots {
                f(Entry {
                    name: from_wide(name),
                    attrs: data.dwFileAttributes,
                    size: ((data.nFileSizeHigh as u64) << 32) | data.nFileSizeLow as u64,
                    mtime: filetime_to_unix(data.ftLastWriteTime.dwLowDateTime, data.ftLastWriteTime.dwHighDateTime),
                    atime: filetime_to_unix(data.ftLastAccessTime.dwLowDateTime, data.ftLastAccessTime.dwHighDateTime),
                });
            }
            if FindNextFileW(h, &mut data) == 0 {
                break;
            }
        }
        let e = GetLastError();
        FindClose(h);
        if e == ERROR_NO_MORE_FILES {
            Ok(())
        } else {
            Err(e)
        }
    }
}

/// Attributs d'un chemin étendu. `Err(code)` si inaccessible.
pub fn attrs(path_ext: &str) -> Result<u32, u32> {
    let w = wide(path_ext);
    let a = unsafe { GetFileAttributesW(w.as_ptr()) };
    if a == INVALID_FILE_ATTRIBUTES {
        Err(unsafe { GetLastError() })
    } else {
        Ok(a)
    }
}

pub fn file_size(path_ext: &str) -> Option<u64> {
    let w = wide(path_ext);
    unsafe {
        let mut d: WIN32_FILE_ATTRIBUTE_DATA = std::mem::zeroed();
        if GetFileAttributesExW(w.as_ptr(), GetFileExInfoStandard, &mut d as *mut _ as *mut c_void) == 0 {
            return None;
        }
        Some(((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64)
    }
}

pub fn set_attrs(path_ext: &str, a: u32) -> bool {
    let w = wide(path_ext);
    unsafe { SetFileAttributesW(w.as_ptr(), a) != 0 }
}

pub fn last_error() -> u32 {
    unsafe { GetLastError() }
}

/// Message en français pour un code d'erreur Windows courant.
pub fn explain(code: u32) -> &'static str {
    match code {
        ERROR_ACCESS_DENIED => "Accès refusé (droits insuffisants ou fichier protégé)",
        ERROR_SHARING_VIOLATION => "Fichier utilisé par un autre programme",
        ERROR_LOCK_VIOLATION => "Portion de fichier verrouillée par un programme",
        ERROR_FILE_NOT_FOUND => "Fichier introuvable (entrée orpheline ou nom invalide)",
        ERROR_PATH_NOT_FOUND => "Chemin introuvable (nom spécial, trop long ou entrée corrompue)",
        ERROR_INVALID_NAME => "Nom invalide pour Windows (point/espace final, nom réservé…)",
        ERROR_BAD_PATHNAME => "Chemin incorrect",
        ERROR_DIR_NOT_EMPTY => "Dossier non vide (un élément à l'intérieur résiste)",
        ERROR_FILENAME_EXCED_RANGE => "Nom ou chemin trop long",
        ERROR_CANT_ACCESS_FILE => "Fichier inaccessible (lien ou entrée endommagée)",
        ERROR_FILE_CORRUPT | ERROR_DISK_CORRUPT => "Entrée corrompue sur le disque (chkdsk recommandé)",
        ERROR_WRITE_PROTECT => "Support protégé en écriture",
        ERROR_CURRENT_DIRECTORY => "Dossier utilisé comme dossier courant par un programme",
        ERROR_NOT_READY => "Périphérique non prêt",
        _ => "Erreur système Windows",
    }
}
