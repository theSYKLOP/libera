//! Intégration système : élévation, disques, corbeille, outils natifs Windows (powercfg, DISM…).

use crate::winfs::wide;
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::process::Command;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Shell::*;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DRIVE_FIXED: u32 = 3;

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret,
        );
        CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    }
}

/// Relance l'application avec les droits administrateur (invite UAC de Windows).
pub fn relaunch_elevated() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let op = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
    if (r as isize) > 32 {
        Ok(())
    } else {
        Err("Élévation annulée ou refusée.".into())
    }
}

#[derive(Serialize, Clone)]
pub struct DiskInfo {
    pub root: String,
    pub total: u64,
    pub free: u64,
    pub used: u64,
}

pub fn disk_info(root: &str) -> Option<DiskInfo> {
    let w = wide(root);
    let (mut free_avail, mut total, mut free) = (0u64, 0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free_avail, &mut total, &mut free) };
    if ok == 0 {
        return None;
    }
    Some(DiskInfo { root: root.to_string(), total, free, used: total - free })
}

/// Disques fixes (locaux) disponibles.
pub fn fixed_drives() -> Vec<DiskInfo> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) != 0 {
            let root = format!("{}:\\", (b'A' + i as u8) as char);
            let w = wide(&root);
            if unsafe { GetDriveTypeW(w.as_ptr()) } == DRIVE_FIXED {
                if let Some(d) = disk_info(&root) {
                    out.push(d);
                }
            }
        }
    }
    out
}

/// (octets, éléments) dans la corbeille.
pub fn recycle_bin_size() -> (u64, u64) {
    unsafe {
        let mut info: SHQUERYRBINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHQUERYRBINFO>() as u32;
        if SHQueryRecycleBinW(std::ptr::null(), &mut info) == 0 {
            (info.i64Size.max(0) as u64, info.i64NumItems.max(0) as u64)
        } else {
            (0, 0)
        }
    }
}

pub fn empty_recycle_bin() -> Result<(), String> {
    // SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND
    let hr = unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), 0x1 | 0x2 | 0x4) };
    // 0 = succès ; 0x8000FFFF-like valeurs possibles si la corbeille est déjà vide.
    if hr == 0 || (hr as u32) == 0x8000_0008 {
        Ok(())
    } else {
        Err(format!("Échec du vidage de la corbeille (code 0x{:08X})", hr as u32))
    }
}

/// Envoie un élément à la Corbeille (récupérable). Ne libère PAS d'espace disque tant qu'elle n'est pas vidée.
pub fn send_to_recycle_bin(path: &str) -> Result<(), u32> {
    // SHFileOperation exige une liste terminée par un double NUL.
    let mut from = wide(path);
    from.push(0);
    unsafe {
        let mut op: SHFILEOPSTRUCTW = std::mem::zeroed();
        op.wFunc = FO_DELETE;
        op.pFrom = from.as_ptr();
        // FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI
        op.fFlags = (0x0040 | 0x0010 | 0x0004 | 0x0400) as u16;
        let r = SHFileOperationW(&mut op);
        if r == 0 && op.fAnyOperationsAborted == 0 {
            Ok(())
        } else {
            Err(r as u32)
        }
    }
}

/// Exécute un outil natif Windows sans fenêtre et renvoie (code, sortie).
pub fn run_tool(program: &str, args: &[&str]) -> (i32, String) {
    match Command::new(program).args(args).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).to_string();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.code().unwrap_or(-1), s)
        }
        Err(e) => (-1, e.to_string()),
    }
}

/// Ouvre l'Explorateur sur un élément.
pub fn reveal_in_explorer(path: &str) {
    let p = crate::winfs::display_path(path);
    let _ = Command::new("explorer.exe").arg(format!("/select,{}", p)).spawn();
}

/// Développe les variables `%NOM%` d'une ligne de commande.
pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) if j > 0 => match std::env::var(&after[..j]) {
                Ok(v) => { out.push_str(&v); rest = &after[j + 1..]; }
                Err(_) => { out.push('%'); rest = after; }
            },
            _ => { out.push('%'); rest = after; }
        }
    }
    out.push_str(rest);
    out
}

/// Sépare une ligne de commande de désinstallation en (programme, arguments), comme le fait Windows :
/// programme entre guillemets, ou non guillemeté jusqu'à `.exe` (les chemins avec espaces sont fréquents).
pub fn split_command(cmd: &str) -> (String, String) {
    let c = expand_env(cmd.trim());
    let c = c.trim();
    if let Some(rest) = c.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
        }
    }
    let l = c.to_lowercase();
    for ext in [".exe", ".msi", ".bat", ".cmd", ".com"] {
        let mut from = 0;
        while let Some(i) = l[from..].find(ext) {
            let end = from + i + ext.len();
            if end == c.len() || c[end..].starts_with(' ') {
                return (c[..end].to_string(), c[end..].trim().to_string());
            }
            from = end;
        }
    }
    match c.split_once(' ') {
        Some((a, b)) => (a.to_string(), b.trim().to_string()),
        None => (c.to_string(), String::new()),
    }
}

/// Lance le désinstalleur officiel d'une application avec l'API native de Windows (ShellExecute) :
/// les guillemets sont respectés, et une élévation (UAC) est demandée si le programme l'exige.
/// Renvoie une erreur claire si Windows ne peut pas le lancer, au lieu d'échouer en silence.
pub fn run_uninstaller(command: &str) -> Result<(), String> {
    let (exe, args) = split_command(command);
    if exe.is_empty() {
        return Err("Aucune commande de désinstallation connue pour cette application.".into());
    }
    let has_path = exe.contains('\\') || exe.contains('/');
    if has_path && !std::path::Path::new(&exe).exists() {
        return Err(format!("Le programme de désinstallation est introuvable : {}. Utilise « Forcer la suppression ».", exe));
    }
    // Un fichier .msi se désinstalle via msiexec.
    let (exe, args) = if exe.to_lowercase().ends_with(".msi") { ("msiexec.exe".to_string(), format!("/x \"{}\" {}", exe, args)) } else { (exe, args) };
    let dir = std::path::Path::new(&exe).parent().map(|p| p.to_string_lossy().to_string()).filter(|d| !d.is_empty());
    let (wop, wfile, wargs) = (wide("open"), wide(&exe), wide(&args));
    let wdir = dir.as_deref().map(wide);
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wop.as_ptr(),
            wfile.as_ptr(),
            if args.is_empty() { std::ptr::null() } else { wargs.as_ptr() },
            wdir.as_ref().map(|d| d.as_ptr()).unwrap_or(std::ptr::null()),
            1,
        )
    };
    let code = r as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!("Windows n'a pas pu lancer le désinstalleur (code {}). Utilise « Forcer la suppression ».", code))
    }
}

/// Actions système natives proposées dans le nettoyage.
pub fn disable_hibernation() -> Result<(), String> {
    let (c, out) = run_tool("powercfg.exe", &["-h", "off"]);
    if c == 0 { Ok(()) } else { Err(format!("powercfg a échoué ({}) : {}", c, out.trim())) }
}

pub fn component_cleanup() -> Result<String, String> {
    let (c, out) = run_tool("dism.exe", &["/Online", "/Cleanup-Image", "/StartComponentCleanup", "/NoRestart"]);
    if c == 0 { Ok(out) } else { Err(format!("DISM a échoué ({}) : {}", c, out.trim())) }
}
