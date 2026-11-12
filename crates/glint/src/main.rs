//! Glint: the always-running part. Tray icon, hotkeys, capture overlay and preview card.
//!
//! `glint.exe` with no arguments starts the daemon (or, if it runs already, starts a region snip).
//! `glint.exe region|fullscreen|fullscreen-all|repeat|open|paste|quit` sends that command.
//! `glint.exe rect X Y W H` snips that rectangle of the virtual desktop without the overlay.
//! `glint.exe --background` starts the daemon without snipping (used by "Start with Windows").

#![windows_subsystem = "windows"]

mod app;
mod feedback;
mod gfx;
mod hook;
mod icon;
mod log;
mod overlay;
mod system;
mod toast;
mod wind;

use app::Command;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Ole::OleInitialize;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowW, SendMessageW, ASFW_ANY, WM_COPYDATA,
};

fn main() {
    glint_capture::make_dpi_aware();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let background = args.iter().any(|a| a == "--background");
    let text = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ");
    let cmd = if text.is_empty() {
        None
    } else {
        Command::parse(&text)
    };
    if !text.is_empty() && cmd.is_none() {
        log::line(&format!("unknown command: {text}"));
        std::process::exit(2);
    }

    // GLINT_INSTANCE lets tests run their own daemon next to a real one.
    let name = format!(
        "Local\\Glint.Daemon{}",
        std::env::var("GLINT_INSTANCE").unwrap_or_default()
    );
    let _mutex = unsafe { CreateMutexW(None, true, &windows::core::HSTRING::from(name)) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if !background {
            forward(&cmd.map(|_| text).unwrap_or_else(|| "region".into()));
        }
        return;
    }
    unsafe {
        let _ = OleInitialize(None);
    }
    clean_temp();
    app::run(cmd);
}

/// Sends a command to the running daemon.
fn forward(text: &str) {
    for _ in 0..50 {
        let hwnd = unsafe { FindWindowW(app::CLASS, None) };
        if let Ok(hwnd) = hwnd {
            let bytes = text.as_bytes();
            let cds = COPYDATASTRUCT {
                dwData: app::COPYDATA_TAG,
                cbData: bytes.len() as u32,
                lpData: bytes.as_ptr() as *mut _,
            };
            unsafe {
                // Let the daemon take the foreground for its overlay.
                let _ = AllowSetForegroundWindow(ASFW_ANY);
                SendMessageW(
                    hwnd,
                    WM_COPYDATA,
                    Some(WPARAM(0)),
                    Some(LPARAM(&cds as *const _ as isize)),
                );
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    log::line("could not reach the running Glint");
}

/// Removes hand-off files older than a day (the editor deletes its own on close).
fn clean_temp() {
    let Ok(entries) = std::fs::read_dir(glint_core::paths::temp_dir()) else {
        return;
    };
    let day = std::time::Duration::from_secs(24 * 3600);
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > day);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
