//! The daemon: a hidden window that owns the tray icon, the hotkeys and the capture flow.

use crate::overlay::{self, Mode, Outcome};
use crate::{gfx::wide, icon, log, system, toast};
use glint_capture::windows_list;
use glint_core::config::{parse_hotkey, Config, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN};
use glint_core::geom::monitor_at;
use glint_core::{clipboard, paths, Image, Monitor, Rect};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const CLASS: PCWSTR = w!("GlintDaemon");
pub const COPYDATA_TAG: usize = 0x474C_4E54; // "GLNT"
const WM_TRAY: u32 = WM_APP + 1;
const TIMER_CONFIG: usize = 1;

/// Something the daemon can be asked to do (hotkey, tray, or a second `glint.exe` launch).
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Region,
    Window,
    Fullscreen,
    FullscreenAll,
    Repeat,
    Rect(Rect),
    OpenImage,
    PasteToEditor,
    Quit,
}

impl Command {
    pub fn parse(s: &str) -> Option<Command> {
        let mut it = s.split_whitespace();
        Some(match it.next()? {
            "region" => Command::Region,
            "window" => Command::Window,
            "fullscreen" => Command::Fullscreen,
            "fullscreen-all" => Command::FullscreenAll,
            "repeat" => Command::Repeat,
            "open" => Command::OpenImage,
            "paste" => Command::PasteToEditor,
            "quit" => Command::Quit,
            "rect" => {
                let v: Vec<i32> = it.filter_map(|n| n.parse().ok()).collect();
                if v.len() != 4 || v[2] <= 0 || v[3] <= 0 {
                    return None;
                }
                Command::Rect(Rect::new(v[0], v[1], v[2], v[3]))
            }
            _ => return None,
        })
    }
}

struct App {
    hwnd: HWND,
    config: Config,
    config_mtime: Option<SystemTime>,
    last_rect: Option<Rect>,
    taskbar_created: u32,
    test_dir: Option<PathBuf>,
    captures: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

fn config() -> Config {
    with(|a| a.config.clone()).unwrap_or_default()
}

pub fn save_dir(c: &Config) -> PathBuf {
    c.save_dir
        .clone()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(paths::default_save_dir)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Runs the daemon until Quit. `first` is an optional command to run at once.
pub fn run(first: Option<Command>) {
    unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS,
            w!("Glint"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinst.into()),
            None,
        ) else {
            log::line("cannot create the daemon window");
            return;
        };
        let cfg_path = paths::config_file();
        if !cfg_path.exists() {
            let _ = Config::default().save(&cfg_path);
        }
        let config = Config::load(&cfg_path).unwrap_or_else(|e| {
            log::line(&format!("config error, using defaults: {e}"));
            Config::default()
        });
        let test_dir = std::env::var_os("GLINT_TEST_DIR").map(PathBuf::from);
        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                hwnd,
                config,
                config_mtime: mtime(&cfg_path),
                last_rect: None,
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                test_dir,
                captures: 0,
            })
        });
        add_tray(hwnd);
        register_hotkeys(true);
        SetTimer(Some(hwnd), TIMER_CONFIG, 2000, None);
        log::line(&format!("Glint {} started", env!("CARGO_PKG_VERSION")));
        if let Some(c) = first {
            run_command(c);
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        remove_tray(hwnd);
        toast::close_all();
    }
}

// ---------------------------------------------------------------- tray

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    };
    let tip = wide("Glint: click to snip, right-click for more");
    nid.szTip[..tip.len().min(127)].copy_from_slice(&tip[..tip.len().min(127)]);
    nid
}

fn add_tray(hwnd: HWND) {
    let mut nid = tray_data(hwnd);
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as u32;
    nid.hIcon = icon::hicon(size).unwrap_or_default();
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
    }
}

fn remove_tray(hwnd: HWND) {
    let nid = tray_data(hwnd);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

/// A tray balloon (shown as a Windows notification).
pub fn notify(title: &str, text: &str) {
    log::line(&format!("{title}: {text}"));
    let Some(hwnd) = with(|a| a.hwnd) else { return };
    let mut nid = tray_data(hwnd);
    nid.uFlags = NIF_INFO;
    let t = wide(title);
    let b = wide(text);
    nid.szInfoTitle[..t.len().min(63)].copy_from_slice(&t[..t.len().min(63)]);
    nid.szInfo[..b.len().min(255)].copy_from_slice(&b[..b.len().min(255)]);
    nid.dwInfoFlags = NIIF_INFO;
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

const ID_REGION: usize = 10;
const ID_WINDOW: usize = 11;
const ID_FULL: usize = 12;
const ID_OPEN: usize = 20;
const ID_PASTE: usize = 21;
const ID_FOLDER: usize = 22;
const ID_PRTSC: usize = 30;
const ID_STARTUP: usize = 31;
const ID_SETTINGS: usize = 32;
const ID_QUIT: usize = 40;

fn show_menu(hwnd: HWND) {
    let c = config();
    let label = |name: &str, key: &str| format!("{name}\t{key}");
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let add = |id: usize, text: &str, checked: bool| {
            let t = wide(text);
            let flags = if checked {
                MF_STRING | MF_CHECKED
            } else {
                MF_STRING
            };
            let _ = AppendMenuW(menu, flags, id, PCWSTR(t.as_ptr()));
        };
        let sep = || {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        };
        add(ID_REGION, &label("Snip a region", &c.hotkeys.region), false);
        add(ID_WINDOW, &label("Snip a window", &c.hotkeys.window), false);
        add(
            ID_FULL,
            &label("Snip the full screen", &c.hotkeys.fullscreen),
            false,
        );
        sep();
        add(ID_OPEN, "Open an image...", false);
        add(ID_PASTE, "Edit the image on the clipboard", false);
        add(ID_FOLDER, "Open the screenshots folder", false);
        sep();
        add(
            ID_PRTSC,
            "Use the Print Screen key",
            c.print_screen && !system::print_screen_opens_snipping_tool(),
        );
        add(
            ID_STARTUP,
            "Start with Windows",
            system::starts_with_windows(),
        );
        add(ID_SETTINGS, "Edit settings (config.toml)", false);
        sep();
        add(ID_QUIT, "Quit Glint", false);
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
            p.x,
            p.y,
            None,
            hwnd,
            None,
        )
        .0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        match id {
            ID_REGION => run_command(Command::Region),
            ID_WINDOW => run_command(Command::Window),
            ID_FULL => run_command(Command::Fullscreen),
            ID_OPEN => run_command(Command::OpenImage),
            ID_PASTE => run_command(Command::PasteToEditor),
            ID_FOLDER => {
                let dir = save_dir(&c);
                let _ = std::fs::create_dir_all(&dir);
                system::shell_open(&dir);
            }
            ID_PRTSC => toggle_print_screen(hwnd),
            ID_STARTUP => {
                let on = !system::starts_with_windows();
                if let Err(e) = system::set_start_with_windows(on) {
                    notify(
                        "Glint",
                        &format!("Could not change start with Windows: {e}"),
                    );
                }
            }
            ID_SETTINGS => {
                let p = paths::config_file();
                if !p.exists() {
                    let _ = Config::default().save(&p);
                }
                let _ = std::process::Command::new("notepad.exe").arg(&p).spawn();
            }
            ID_QUIT => PostQuitMessage(0),
            _ => {}
        }
    }
}

fn toggle_print_screen(hwnd: HWND) {
    let mut c = config();
    if !c.print_screen {
        let text = wide(
            "Glint will use the Print Screen key for a region snip.\n\n\
             To do this it turns off the Windows setting \"Use the Print screen key to open screen capture\" \
             (Settings > Accessibility > Keyboard). You can turn it back on here at any time.\n\nContinue?",
        );
        let answer = unsafe {
            MessageBoxW(
                Some(hwnd),
                PCWSTR(text.as_ptr()),
                w!("Glint: Print Screen key"),
                MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND,
            )
        };
        if answer != IDYES {
            return;
        }
        if let Err(e) = system::set_print_screen_opens_snipping_tool(false) {
            notify(
                "Glint",
                &format!("Could not change the Windows setting: {e}"),
            );
            return;
        }
        c.print_screen = true;
    } else {
        let _ = system::set_print_screen_opens_snipping_tool(true);
        c.print_screen = false;
    }
    let path = paths::config_file();
    if let Err(e) = c.save(&path) {
        notify("Glint", &format!("Could not save settings: {e}"));
    }
    with(|a| {
        a.config = c;
        a.config_mtime = mtime(&path);
    });
    register_hotkeys(true);
}

// ---------------------------------------------------------------- hotkeys

const HOTKEY_IDS: [i32; 5] = [1, 2, 3, 4, 5];

fn register_hotkeys(report: bool) {
    let Some((hwnd, c)) = with(|a| (a.hwnd, a.config.clone())) else {
        return;
    };
    unsafe {
        for id in HOTKEY_IDS {
            let _ = UnregisterHotKey(Some(hwnd), id);
        }
    }
    let mut list = vec![
        (1, c.hotkeys.region.clone()),
        (2, c.hotkeys.window.clone()),
        (3, c.hotkeys.fullscreen.clone()),
        (4, c.hotkeys.repeat.clone()),
    ];
    if c.print_screen {
        list.push((5, "PrintScreen".into()));
    }
    let mut failed = Vec::new();
    for (id, text) in list {
        if text.trim().is_empty() {
            continue; // turned off in the config
        }
        match parse_hotkey(&text) {
            Ok(hk) => {
                let mut mods = MOD_NOREPEAT.0;
                for (bit, m) in [
                    (MOD_ALT, 0x1),
                    (MOD_CONTROL, 0x2),
                    (MOD_SHIFT, 0x4),
                    (MOD_WIN, 0x8),
                ] {
                    if hk.mods & bit != 0 {
                        mods |= m;
                    }
                }
                if unsafe { RegisterHotKey(Some(hwnd), id, HOT_KEY_MODIFIERS(mods), hk.vk) }
                    .is_err()
                {
                    failed.push(format!("{text} (in use by another app)"));
                }
            }
            Err(e) => failed.push(e),
        }
    }
    if !failed.is_empty() && report {
        notify("Glint: some hotkeys do not work", &failed.join(", "));
    }
}

fn hotkey_command(id: i32) -> Option<Command> {
    Some(match id {
        1 | 5 => Command::Region,
        2 => Command::Window,
        3 => Command::Fullscreen,
        4 => Command::Repeat,
        _ => return None,
    })
}

fn check_config_changed() {
    let path = paths::config_file();
    let now = mtime(&path);
    let changed = with(|a| a.config_mtime != now).unwrap_or(false);
    if !changed {
        return;
    }
    with(|a| a.config_mtime = now);
    match Config::load(&path) {
        Ok(c) => {
            with(|a| a.config = c);
            register_hotkeys(true);
            log::line("settings reloaded");
        }
        Err(e) => notify("Glint: settings file has an error", &e),
    }
}

// ---------------------------------------------------------------- capture flow

fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

pub fn run_command(cmd: Command) {
    log::line(&format!("command {cmd:?}"));
    match cmd {
        Command::Quit => {
            unsafe { PostQuitMessage(0) };
            return;
        }
        Command::OpenImage => {
            spawn_editor(&["--open".into()]);
            return;
        }
        Command::PasteToEditor => {
            let c = config();
            spawn_editor(&[
                "--paste".into(),
                "--save-dir".into(),
                save_dir(&c).display().to_string(),
            ]);
            return;
        }
        _ => {}
    }
    if overlay::is_open() {
        return;
    }
    let c = config();
    let app_name = windows_list::foreground_app();
    let t = std::time::Instant::now();
    let frame = if std::env::var_os("GLINT_TEST_SYNTHETIC").is_some() {
        Ok(glint_capture::synthetic_desktop())
    } else {
        glint_capture::capture_desktop(c.hdr_capture)
    };
    let frame = match frame {
        Ok(f) => f,
        Err(e) => {
            notify("Glint could not capture the screen", &e);
            return;
        }
    };
    for n in &frame.notes {
        log::line(n);
    }
    log::line(&format!(
        "captured {:?} in {} ms",
        frame.image,
        t.elapsed().as_millis()
    ));
    let (cx, cy) = cursor();
    let mon_at = |f: &glint_capture::Frame, x: i32, y: i32| monitor_at(&f.monitors, x, y).cloned();
    let direct = match &cmd {
        Command::Fullscreen => mon_at(&frame, cx, cy).map(|m| (m.rect, m)),
        Command::FullscreenAll => {
            let vb = frame.bounds();
            mon_at(&frame, cx, cy).map(|m| (vb, m))
        }
        Command::Repeat => match with(|a| a.last_rect).flatten() {
            Some(r) => mon_at(&frame, r.center().0, r.center().1).map(|m| (r, m)),
            None => None,
        },
        Command::Rect(r) => mon_at(&frame, r.center().0, r.center().1).map(|m| (*r, m)),
        _ => None,
    };
    if let Some((rect, monitor)) = direct {
        let image = frame.crop_virtual(&rect.clamp_to(&frame.bounds()));
        drop(frame);
        finish(image, rect, monitor, app_name);
        return;
    }
    let mode = if cmd == Command::Window {
        Mode::Window
    } else {
        Mode::Region
    };
    let started = std::time::Instant::now();
    let res = overlay::show(
        frame,
        mode,
        c.capture_on_release,
        move |outcome| match outcome {
            Outcome::Capture {
                image,
                rect,
                monitor,
            } => finish(image, rect, monitor, app_name),
            Outcome::Cancel => log::line("cancelled"),
        },
    );
    match res {
        Ok(()) => log::line(&format!(
            "overlay shown in {} ms",
            started.elapsed().as_millis() + t.elapsed().as_millis()
        )),
        Err(e) => notify("Glint", &e),
    }
}

/// After a capture: clipboard (on a worker thread), test copy, and the preview card.
fn finish(image: Image, rect: Rect, monitor: Monitor, app: Option<String>) {
    let image = Arc::new(image);
    let Some((hwnd, test_dir, n)) = with(|a| {
        a.last_rect = Some(rect);
        a.captures += 1;
        (a.hwnd, a.test_dir.clone(), a.captures)
    }) else {
        return;
    };
    let img = image.clone();
    let owner = hwnd.0 as isize;
    std::thread::spawn(move || {
        let t = std::time::Instant::now();
        let owner = HWND(owner as *mut _);
        // Tests run with GLINT_TEST_DIR and must not overwrite the user's clipboard.
        if test_dir.is_none() || std::env::var_os("GLINT_TEST_CLIPBOARD").is_some() {
            match clipboard::set_image(&img, Some(owner)) {
                Ok(()) => log::line(&format!(
                    "copied {:?} in {} ms",
                    img,
                    t.elapsed().as_millis()
                )),
                Err(e) => log::line(&format!("clipboard failed: {e}")),
            }
        }
        if let Some(dir) = test_dir {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(
                dir.join(format!("capture-{n}.png")),
                glint_core::encode::encode_png_fast(&img),
            );
        }
    });
    let c = config();
    toast::show(
        image,
        monitor,
        toast::Opts {
            seconds: c.preview_seconds,
            save_dir: save_dir(&c),
            app,
        },
    );
}

fn spawn_editor(args: &[String]) {
    let exe = paths::sibling_exe("glint-editor.exe");
    if let Err(e) = std::process::Command::new(&exe).args(args).spawn() {
        notify(
            "Glint could not open the editor",
            &format!("{}: {e}", exe.display()),
        );
    }
}

/// Opens the editor on a file. `temp`: the editor deletes the file when it closes.
pub fn open_editor(path: &Path, name: &str, save_dir: &Path, temp: bool) {
    let mut args = vec![
        "--file".to_string(),
        path.display().to_string(),
        "--name".into(),
        name.into(),
        "--save-dir".into(),
        save_dir.display().to_string(),
    ];
    if temp {
        args.push("--temp".into());
    }
    spawn_editor(&args);
}

// ---------------------------------------------------------------- window procedure

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            if let Some(c) = hotkey_command(wp.0 as i32) {
                run_command(c);
            }
            LRESULT(0)
        }
        WM_TRAY => {
            match lp.0 as u32 {
                WM_LBUTTONUP => run_command(Command::Region),
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_COPYDATA => {
            let cds = &*(lp.0 as *const COPYDATASTRUCT);
            if cds.dwData == COPYDATA_TAG && !cds.lpData.is_null() {
                let bytes =
                    std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                let text = String::from_utf8_lossy(bytes).to_string();
                // Run after this message returns, so the sender is not blocked by the overlay.
                if let Some(c) = Command::parse(&text) {
                    PENDING.with(|p| p.borrow_mut().push(c));
                    let _ = PostMessageW(Some(hwnd), WM_APP + 2, WPARAM(0), LPARAM(0));
                }
                return LRESULT(1);
            }
            LRESULT(0)
        }
        m if m == WM_APP + 2 => {
            let cmds: Vec<Command> = PENDING.with(|p| p.borrow_mut().drain(..).collect());
            for c in cmds {
                run_command(c);
            }
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_CONFIG => {
            check_config_changed();
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        m if m != 0 && Some(m) == with(|a| a.taskbar_created) => {
            add_tray(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

thread_local! {
    static PENDING: RefCell<Vec<Command>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(Command::parse("region"), Some(Command::Region));
        assert_eq!(
            Command::parse("rect 10 -20 30 40"),
            Some(Command::Rect(Rect::new(10, -20, 30, 40)))
        );
        assert_eq!(Command::parse("rect 1 2 0 4"), None);
        assert_eq!(Command::parse("bogus"), None);
    }
}
