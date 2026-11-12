//! The settings window (`glint-editor --settings`). It lives in the editor binary so the tray app
//! stays a plain Win32 program. Every change is written to config.toml at once; the tray app
//! notices the new file within a few seconds and applies it (hotkeys included).

use crate::{SettingsWindow, Theme};
use glint_core::config::{parse_hotkey, Config};
use glint_core::{paths, system};
use slint::platform::Key;
use slint::{ComponentHandle, ModelRc, SharedString, TimerMode, VecModel};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

const TITLE: &str = "Glint settings";

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

fn fill(ui: &SettingsWindow, c: &Config) {
    ui.set_capture_on_release(c.capture_on_release);
    ui.set_print_screen(c.print_screen);
    ui.set_hdr_capture(c.hdr_capture);
    ui.set_capture_sound(c.capture_sound);
    ui.set_capture_frame(c.capture_frame);
    ui.set_preview_seconds(c.preview_seconds as i32);
    ui.set_save_dir(save_dir(c).display().to_string().into());
    ui.set_hk_region(keys(&c.hotkeys.region));
    ui.set_hk_fullscreen(keys(&c.hotkeys.fullscreen));
    ui.set_hk_repeat(keys(&c.hotkeys.repeat));
}

fn snapshot(ui: &SettingsWindow) {
    let Some(dir) = std::env::var_os("GLINT_TEST_DIR") else {
        return;
    };
    if let Ok(buf) = ui.window().take_snapshot() {
        let img = glint_core::Image {
            width: buf.width(),
            height: buf.height(),
            data: buf.as_bytes().to_vec(),
        };
        let _ = std::fs::write(
            Path::new(&dir).join("settings-window.png"),
            glint_core::encode::encode_png_fast(&img),
        );
    }
}

/// Brings an open settings window to the front instead of opening a second one.
fn focus_existing() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };
    unsafe {
        match FindWindowW(None, &HSTRING::from(TITLE)) {
            Ok(h) if !h.is_invalid() => {
                let _ = ShowWindow(h, SW_RESTORE);
                let _ = SetForegroundWindow(h);
                true
            }
            _ => false,
        }
    }
}

pub fn run() {
    if focus_existing() {
        return;
    }
    let path = paths::config_file();
    let cfg = Rc::new(RefCell::new(Config::load(&path).unwrap_or_default()));
    let ui = SettingsWindow::new().expect("create the settings window");
    ui.global::<Theme>().set_dark(crate::system_dark());
    fill(&ui, &cfg.borrow());
    ui.set_start_with_windows(!test_mode() && system::starts_with_windows());
    ui.set_version(env!("CARGO_PKG_VERSION").into());

    let save = {
        let cfg = cfg.clone();
        let path = path.clone();
        move |ui: &SettingsWindow| match cfg.borrow().save(&path) {
            Ok(()) => ui.set_note("".into()),
            Err(e) => ui.set_note(format!("Could not save the settings: {e}").into()),
        }
    };

    {
        let (weak, cfg, save) = (ui.as_weak(), cfg.clone(), save.clone());
        ui.on_changed(move || {
            let Some(ui) = weak.upgrade() else { return };
            {
                let mut c = cfg.borrow_mut();
                c.capture_on_release = ui.get_capture_on_release();
                c.hdr_capture = ui.get_hdr_capture();
                c.capture_sound = ui.get_capture_sound();
                c.capture_frame = ui.get_capture_frame();
                c.preview_seconds = ui.get_preview_seconds().clamp(1, 60) as u32;
            }
            save(&ui);
        });
    }
    {
        let (weak, cfg, save) = (ui.as_weak(), cfg.clone(), save.clone());
        ui.on_print_screen_changed(move |on| {
            let Some(ui) = weak.upgrade() else { return };
            // On: Glint takes the key, so Windows' own Print Screen snipping is turned off (and
            // back on when this is switched off). The help text under the toggle says so.
            if !test_mode() {
                if let Err(e) = system::set_print_screen_opens_snipping_tool(!on) {
                    ui.set_print_screen(!on);
                    ui.set_note(format!("Could not change the Windows setting: {e}").into());
                    return;
                }
            }
            cfg.borrow_mut().print_screen = on;
            save(&ui);
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_start_with_windows_changed(move |on| {
            let Some(ui) = weak.upgrade() else { return };
            if test_mode() {
                return;
            }
            if let Err(e) = system::set_start_with_windows(on) {
                ui.set_start_with_windows(!on);
                ui.set_note(format!("Could not change start with Windows: {e}").into());
            }
        });
    }
    {
        let (weak, cfg, save) = (ui.as_weak(), cfg.clone(), save.clone());
        ui.on_hotkey_changed(move |which, value| {
            let Some(ui) = weak.upgrade() else { return };
            let v = if value == "clear" {
                String::new()
            } else {
                value.to_string()
            };
            {
                let mut c = cfg.borrow_mut();
                match which.as_str() {
                    "region" => c.hotkeys.region = v.clone(),
                    "fullscreen" => c.hotkeys.fullscreen = v.clone(),
                    _ => c.hotkeys.repeat = v.clone(),
                }
            }
            match which.as_str() {
                "region" => ui.set_hk_region(keys(&v)),
                "fullscreen" => ui.set_hk_fullscreen(keys(&v)),
                _ => ui.set_hk_repeat(keys(&v)),
            }
            save(&ui);
        });
    }
    ui.on_record_hotkey(|text, ctrl, alt, shift, win| record(&text, ctrl, alt, shift, win).into());
    {
        let (weak, cfg, save) = (ui.as_weak(), cfg.clone(), save.clone());
        ui.on_browse_folder(move || {
            let Some(ui) = weak.upgrade() else { return };
            let start = save_dir(&cfg.borrow());
            if let Some(dir) = rfd::FileDialog::new()
                .set_title("Choose where Glint saves pictures")
                .set_directory(&start)
                .pick_folder()
            {
                cfg.borrow_mut().save_dir = Some(dir.clone());
                ui.set_save_dir(dir.display().to_string().into());
                save(&ui);
            }
        });
    }
    {
        let cfg = cfg.clone();
        ui.on_open_folder(move || {
            let dir = save_dir(&cfg.borrow());
            let _ = std::fs::create_dir_all(&dir);
            system::shell_open(&dir);
        });
    }
    {
        let path = path.clone();
        ui.on_open_file(move || {
            if !path.exists() {
                let _ = Config::default().save(&path);
            }
            let _ = std::process::Command::new("notepad.exe").arg(&path).spawn();
        });
    }
    ui.on_open_website(|| system::shell_open(Path::new("https://github.com/Maxaubert/Glint")));
    {
        let weak = ui.as_weak();
        ui.on_key(move |text| {
            let Some(ui) = weak.upgrade() else {
                return false;
            };
            if is(&text, Key::Escape) {
                let _ = ui.hide();
                return true;
            }
            if test_mode() && is(&text, Key::F12) {
                snapshot(&ui);
                return true;
            }
            // Tests only: F9 flips the capture sound through the normal save path.
            if test_mode() && is(&text, Key::F9) {
                ui.set_capture_sound(!ui.get_capture_sound());
                ui.invoke_changed();
                return true;
            }
            false
        });
    }

    let theme_timer = slint::Timer::default();
    {
        let weak = ui.as_weak();
        theme_timer.start(TimerMode::Repeated, Duration::from_secs(2), move || {
            if let Some(ui) = weak.upgrade() {
                let dark = crate::system_dark();
                if ui.global::<Theme>().get_dark() != dark {
                    ui.global::<Theme>().set_dark(dark);
                }
            }
        });
    }
    let _ = ui.show();
    crate::winicon::apply_soon();
    let _ = slint::run_event_loop();
    let _ = ui.hide();
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
