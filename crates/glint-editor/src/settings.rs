//! Settings, shown as an overlay inside the editor window (ui/settings.slint). The options were
//! chosen with the owner one by one (2026-11-12): HDR capture; capture sound and preview time; the
//! three hotkeys; save folder and format; theme; start with Windows; about. Every change is written
//! to config.toml at once; the tray app notices the new file within a few seconds and applies it.
//! `glint-editor --settings` (the tray's Settings item) opens a window with only this overlay.

use crate::{EditorWindow, SettingsState, Theme};
use glint_core::config::{parse_hotkey, Config};
use glint_core::{paths, system};
use slint::platform::Key;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::path::Path;

fn test_mode() -> bool {
    std::env::var_os("GLINT_TEST_DIR").is_some()
}

/// "Win+Shift+S" as key caps.
fn keys(s: &str) -> ModelRc<SharedString> {
    let parts: Vec<SharedString> = s
        .split('+')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(SharedString::from)
        .collect();
    ModelRc::new(VecModel::from(parts))
}

fn is(text: &str, k: Key) -> bool {
    SharedString::from(k).as_str() == text
}

/// Turns a key press in a hotkey field into a hotkey string. "cancel" (Esc), "clear" (Backspace or
/// Delete), "" to keep waiting (a modifier alone, or a key that needs a modifier first).
pub fn record(text: &str, ctrl: bool, alt: bool, shift: bool, win: bool) -> String {
    if is(text, Key::Escape) {
        return "cancel".into();
    }
    if is(text, Key::Backspace) || is(text, Key::Delete) {
        return "clear".into();
    }
    let fkeys = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];
    let name = if let Some(i) = fkeys.iter().position(|k| is(text, *k)) {
        format!("F{}", i + 1)
    } else {
        let mut ch = text.chars();
        match (ch.next(), ch.next()) {
            (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase().to_string(),
            _ => return String::new(),
        }
    };
    let has_mod = ctrl || alt || shift || win;
    if !has_mod && !name.starts_with('F') {
        // A bare letter as a global hotkey would steal typing everywhere.
        return String::new();
    }
    let mut parts = Vec::new();
    if win {
        parts.push("Win");
    }
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    parts.push(&name);
    let s = parts.join("+");
    if parse_hotkey(&s).is_ok() {
        s
    } else {
        String::new()
    }
}

fn save_dir(c: &Config) -> std::path::PathBuf {
    c.save_dir.clone().unwrap_or_else(paths::default_save_dir)
}

fn load() -> Config {
    Config::load(&paths::config_file()).unwrap_or_default()
}

fn save(s: &SettingsState, c: &Config) {
    match c.save(&paths::config_file()) {
        Ok(()) => s.set_note("".into()),
        Err(e) => s.set_note(format!("Could not save the settings: {e}").into()),
    }
}

/// Fills the overlay from config.toml and shows it.
pub fn open(ui: &EditorWindow) {
    let c = load();
    let s = ui.global::<SettingsState>();
    s.set_hdr_capture(c.hdr_capture);
    s.set_capture_sound(c.capture_sound);
    s.set_preview_seconds(c.preview_seconds as i32);
    s.set_hk_region(keys(&c.hotkeys.region));
    s.set_hk_fullscreen(keys(&c.hotkeys.fullscreen));
    s.set_hk_repeat(keys(&c.hotkeys.repeat));
    s.set_save_dir(save_dir(&c).display().to_string().into());
    s.set_save_format(c.save_format.clone().into());
    s.set_theme(c.theme.clone().into());
    s.set_start_with_windows(!test_mode() && system::starts_with_windows());
    s.set_version(env!("CARGO_PKG_VERSION").into());
    s.set_note("".into());
    ui.set_settings_open(true);
}

/// Closes the overlay; from the tray (settings only) that closes the window.
pub fn close(ui: &EditorWindow) {
    if ui.get_settings_only() {
        let _ = ui.hide();
        let _ = slint::quit_event_loop();
    } else {
        ui.set_settings_open(false);
        ui.invoke_focus_canvas();
    }
}

/// Tests only (F9 while the overlay is open): flips the capture sound through the save path.
pub fn test_flip_sound(ui: &EditorWindow) {
    let s = ui.global::<SettingsState>();
    s.set_capture_sound(!s.get_capture_sound());
    s.invoke_changed();
}

/// Wires the overlay's callbacks once.
pub fn attach(ui: &EditorWindow, settings_only: bool) {
    ui.set_settings_only(settings_only);
    let s = ui.global::<SettingsState>();
    {
        let weak = ui.as_weak();
        s.on_changed(move || {
            let Some(ui) = weak.upgrade() else { return };
            let s = ui.global::<SettingsState>();
            let mut c = load();
            c.hdr_capture = s.get_hdr_capture();
            c.capture_sound = s.get_capture_sound();
            c.preview_seconds = s.get_preview_seconds().clamp(1, 60) as u32;
            c.save_format = s.get_save_format().to_string();
            c.theme = s.get_theme().to_string();
            save(&s, &c);
            // The theme applies at once.
            ui.global::<Theme>().set_dark(crate::dark_now());
        });
    }
    {
        let weak = ui.as_weak();
        s.on_start_with_windows_changed(move |on| {
            let Some(ui) = weak.upgrade() else { return };
            if test_mode() {
                return;
            }
            if let Err(e) = system::set_start_with_windows(on) {
                let s = ui.global::<SettingsState>();
                s.set_start_with_windows(!on);
                s.set_note(format!("Could not change start with Windows: {e}").into());
            }
        });
    }
    {
        let weak = ui.as_weak();
        s.on_hotkey_changed(move |which, value| {
            let Some(ui) = weak.upgrade() else { return };
            let s = ui.global::<SettingsState>();
            let v = if value == "clear" {
                String::new()
            } else {
                value.to_string()
            };
            let mut c = load();
            match which.as_str() {
                "region" => {
                    c.hotkeys.region = v.clone();
                    s.set_hk_region(keys(&v));
                }
                "fullscreen" => {
                    c.hotkeys.fullscreen = v.clone();
                    s.set_hk_fullscreen(keys(&v));
                }
                _ => {
                    c.hotkeys.repeat = v.clone();
                    s.set_hk_repeat(keys(&v));
                }
            }
            save(&s, &c);
            // The field gave up focus; give it back so Esc still closes settings.
            ui.invoke_focus_canvas();
        });
    }
    s.on_record_hotkey(|text, ctrl, alt, shift, win| record(&text, ctrl, alt, shift, win).into());
    {
        let weak = ui.as_weak();
        s.on_browse_folder(move || {
            let Some(ui) = weak.upgrade() else { return };
            let s = ui.global::<SettingsState>();
            let mut c = load();
            if let Some(dir) = rfd::FileDialog::new()
                .set_title("Choose where Glint saves pictures")
                .set_directory(save_dir(&c))
                .pick_folder()
            {
                s.set_save_dir(dir.display().to_string().into());
                c.save_dir = Some(dir);
                save(&s, &c);
            }
        });
    }
    s.on_open_folder(|| {
        let dir = save_dir(&load());
        let _ = std::fs::create_dir_all(&dir);
        system::shell_open(&dir);
    });
    s.on_open_website(|| system::shell_open(Path::new("https://github.com/Maxaubert/Glint")));
    {
        let weak = ui.as_weak();
        s.on_close(move || {
            if let Some(ui) = weak.upgrade() {
                close(&ui);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::record;

    #[test]
    fn records_hotkeys() {
        assert_eq!(record("s", false, false, true, true), "Win+Shift+S");
        assert_eq!(record("k", true, true, false, false), "Ctrl+Alt+K");
        assert_eq!(
            record("s", false, false, false, false),
            "",
            "a bare letter waits"
        );
    }
}
