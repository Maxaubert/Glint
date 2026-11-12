//! A tiny append-only log at `%LOCALAPPDATA%\Glint\glint.log` (trimmed when it passes 256 KB).

use std::io::Write;

pub fn path() -> std::path::PathBuf {
    glint_core::paths::temp_dir()
        .parent()
        .map(|p| p.join("glint.log"))
        .unwrap_or_else(|| "glint.log".into())
}

pub fn line(msg: &str) {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&p)
        .map(|m| m.len() > 256 * 1024)
        .unwrap_or(false)
    {
        let _ = std::fs::rename(&p, p.with_extension("log.old"));
    }
    let t = glint_core::naming::Stamp::now();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        let _ = writeln!(
            f,
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} {msg}",
            t.year, t.month, t.day, t.hour, t.minute, t.second
        );
    }
    if std::env::var_os("GLINT_TRACE").is_some() {
        eprintln!("{msg}");
    }
}
