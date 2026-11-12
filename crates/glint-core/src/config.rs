//! `config.toml`: hotkeys, save folder, preview timing and the Print Screen takeover.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkeys: Hotkeys,
    /// Where Save writes. Empty means `Pictures\Glint`.
    pub save_dir: Option<PathBuf>,
    /// How long the preview stays, in seconds (paused while the mouse is over it).
    pub preview_seconds: u32,
    /// Use the Print Screen key for a region snip. Glint turns off the Windows setting
    /// that gives Print Screen to the Snipping Tool only after you agree in the tray menu.
    pub print_screen: bool,
    /// Re-capture HDR monitors through DXGI and tone-map them. Better colours for HDR games and
    /// video, about 200 ms slower. Desktop content looks right without it.
    pub hdr_capture: bool,
    /// Snip as soon as the mouse is released, instead of waiting for Enter or a double-click.
    pub capture_on_release: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { hotkeys: Hotkeys::default(), save_dir: None, preview_seconds: 6, print_screen: false, hdr_capture: false, capture_on_release: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub region: String,
    pub window: String,
    pub fullscreen: String,
    pub repeat: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            region: "Ctrl+Shift+S".into(),
            window: "Ctrl+Shift+W".into(),
            fullscreen: "Ctrl+Shift+F".into(),
            repeat: "Ctrl+Shift+R".into(),
        }
    }
}

impl Config {
    /// Loads the file, or the defaults if it does not exist. A broken file is an error.
    pub fn load(path: &Path) -> Result<Config, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let body = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        let text = format!("{}{}", HEADER, body);
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

const HEADER: &str = "# Glint settings. Saved changes apply within a few seconds.\n\
# Hotkeys: modifiers Ctrl, Alt, Shift, Win joined with +, then a key such as S, F5 or PrintScreen.\n\n";

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

/// A parsed hotkey: `RegisterHotKey` modifier bits and a virtual-key code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub mods: u32,
    pub vk: u32,
}

/// Parses text such as `Ctrl+Shift+S`, `Alt+F5` or `PrintScreen`.
pub fn parse_hotkey(s: &str) -> Result<Hotkey, String> {
    let mut mods = 0;
    let mut vk = None;
    for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "win" | "windows" | "super" => mods |= MOD_WIN,
            key => {
                if vk.is_some() {
                    return Err(format!("'{s}' has more than one key"));
                }
                vk = Some(key_code(key).ok_or_else(|| format!("unknown key '{part}' in '{s}'"))?);
            }
        }
    }
    let vk = vk.ok_or_else(|| format!("'{s}' has no key"))?;
    Ok(Hotkey { mods, vk })
}

fn key_code(k: &str) -> Option<u32> {
    if k.len() == 1 {
        let c = k.as_bytes()[0].to_ascii_uppercase();
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u32);
        }
    }
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    Some(match k {
        "printscreen" | "prtsc" | "print" | "snapshot" => 0x2C,
        "insert" | "ins" => 0x2D,
        "delete" | "del" => 0x2E,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" | "pgup" => 0x21,
        "pagedown" | "pgdn" => 0x22,
        "pause" => 0x13,
        "scrolllock" => 0x91,
        "space" => 0x20,
        "`" | "backtick" => 0xC0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkeys() {
        assert_eq!(parse_hotkey("Ctrl+Shift+S"), Ok(Hotkey { mods: MOD_CONTROL | MOD_SHIFT, vk: b'S' as u32 }));
        assert_eq!(parse_hotkey("alt + f5"), Ok(Hotkey { mods: MOD_ALT, vk: 0x74 }));
        assert_eq!(parse_hotkey("PrintScreen"), Ok(Hotkey { mods: 0, vk: 0x2C }));
        assert_eq!(parse_hotkey("Win+1"), Ok(Hotkey { mods: MOD_WIN, vk: b'1' as u32 }));
        assert!(parse_hotkey("Ctrl+Shift").is_err());
        assert!(parse_hotkey("Ctrl+A+B").is_err());
        assert!(parse_hotkey("Ctrl+Banana").is_err());
    }

    #[test]
    fn defaults_parse() {
        let h = Hotkeys::default();
        for s in [&h.region, &h.window, &h.fullscreen, &h.repeat] {
            parse_hotkey(s).unwrap();
        }
    }

    #[test]
    fn partial_file_keeps_defaults() {
        let c: Config = toml::from_str("preview_seconds = 3\n[hotkeys]\nregion = \"PrintScreen\"\n").unwrap();
        assert_eq!(c.preview_seconds, 3);
        assert_eq!(c.hotkeys.region, "PrintScreen");
        assert_eq!(c.hotkeys.window, "Ctrl+Shift+W");
    }

    #[test]
    fn save_and_load_round_trip() {
        let dir = std::env::temp_dir().join(format!("glint-cfg-{}", std::process::id()));
        let path = dir.join("config.toml");
        let mut c = Config::default();
        c.print_screen = true;
        c.save_dir = Some(PathBuf::from(r"D:\Shots"));
        c.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap(), c);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(Config::load(&path).unwrap(), Config::default());
    }
}
