//! Where Glint keeps its files.

use std::path::PathBuf;

fn env_dir(var: &str) -> PathBuf {
    std::env::var_os(var)
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// `%APPDATA%\Glint\config.toml` (or `GLINT_CONFIG` when set, for tests).
pub fn config_file() -> PathBuf {
    if let Some(p) = std::env::var_os("GLINT_CONFIG") {
        return PathBuf::from(p);
    }
    env_dir("APPDATA").join("Glint").join("config.toml")
}

/// `%LOCALAPPDATA%\Glint\tmp`: hand-off files for the editor and drag-out. Cleaned on start.
pub fn temp_dir() -> PathBuf {
    env_dir("LOCALAPPDATA").join("Glint").join("tmp")
}

/// The user's Pictures folder plus `Glint`.
pub fn default_save_dir() -> PathBuf {
    use windows::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    let pictures = unsafe { SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None) }
        .ok()
        .and_then(|p| {
            let s = unsafe { p.to_string() }.ok();
            unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _)) };
            s
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| env_dir("USERPROFILE").join("Pictures"));
    pictures.join("Glint")
}

/// A sibling executable, for example `glint-editor.exe` next to `glint.exe`.
pub fn sibling_exe(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
}
