//! File names for saved captures: `Glint_<app>_<yyyy-MM-dd_HH-mm-ss>.png`.

use std::path::{Path, PathBuf};

/// Local date and time parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

impl Stamp {
    pub fn now() -> Stamp {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        Stamp { year: t.wYear, month: t.wMonth, day: t.wDay, hour: t.wHour, minute: t.wMinute, second: t.wSecond }
    }
}

/// Keeps letters, digits, `-` and `.`; turns the rest into single `-`; caps the length.
pub fn sanitize(app: &str) -> String {
    let mut out = String::new();
    for c in app.chars() {
        if c.is_alphanumeric() || c == '.' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches(['-', '.']).chars().take(40).collect::<String>();
    out.trim_end_matches(['-', '.']).to_string()
}

/// `Glint_Chrome_2026-11-12_14-03-05.png`, or without the app part when it is empty.
pub fn file_name(app: &str, t: Stamp, ext: &str) -> String {
    let app = sanitize(app.trim_end_matches(".exe").trim_end_matches(".EXE"));
    let time = format!(
        "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
        t.year, t.month, t.day, t.hour, t.minute, t.second
    );
    if app.is_empty() {
        format!("Glint_{time}.{ext}")
    } else {
        format!("Glint_{app}_{time}.{ext}")
    }
}

/// `dir/name`, adding ` (2)`, ` (3)` and so on if the file already exists.
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    (2..)
        .map(|n| dir.join(if ext.is_empty() { format!("{stem} ({n})") } else { format!("{stem} ({n}).{ext}") }))
        .find(|p| !p.exists())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Stamp = Stamp { year: 2026, month: 11, day: 2, hour: 9, minute: 3, second: 5 };

    #[test]
    fn names() {
        assert_eq!(file_name("chrome.exe", T, "png"), "Glint_chrome_2026-11-02_09-03-05.png");
        assert_eq!(file_name("", T, "jpg"), "Glint_2026-11-02_09-03-05.jpg");
        assert_eq!(file_name("My App: v2/beta", T, "png"), "Glint_My-App-v2-beta_2026-11-02_09-03-05.png");
    }

    #[test]
    fn sanitize_caps_and_trims() {
        assert_eq!(sanitize("---a---"), "a");
        assert_eq!(sanitize(&"x".repeat(100)).len(), 40);
        assert_eq!(sanitize("Æøå"), "Æøå");
    }

    #[test]
    fn unique_adds_counter() {
        let dir = std::env::temp_dir().join(format!("glint-name-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        std::fs::write(dir.join("a (2).png"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.png"), dir.join("a (3).png"));
        assert_eq!(unique_path(&dir, "b.png"), dir.join("b.png"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
