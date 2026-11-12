//! glint-editor: opens a capture, lets you annotate it, then copy or save it.
//!
//! glint-editor --file <png> [--name <file name>] [--save-dir <dir>] [--temp]
//! glint-editor --paste      (the image on the clipboard)
//! glint-editor --open       (pick a file)
//! glint-editor <file>

#![windows_subsystem = "windows"]

slint::include_modules!();

use glint_core::{clipboard, Image};
use glint_editor::editor::{Editor, Mods, PALETTE};
use glint_editor::model::{text_size, Tool};
use glint_editor::render::Renderer;
use serde::{Deserialize, Serialize};
use slint::platform::Key;
use slint::TimerMode;
use slint::{
    CloseRequestResponse, Color, ComponentHandle, ModelRc, SharedPixelBuffer, SharedString,
    VecModel,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDCANCEL, IDYES, MB_ICONWARNING, MB_YESNOCANCEL,
};

#[derive(Default)]
struct Args {
    file: Option<PathBuf>,
    name: Option<String>,
    save_dir: Option<PathBuf>,
    temp: bool,
    open: bool,
    paste: bool,
}

fn parse_args() -> Args {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--file" => a.file = it.next().map(PathBuf::from),
            "--name" => a.name = it.next(),
            "--save-dir" => a.save_dir = it.next().map(PathBuf::from),
            "--temp" => a.temp = true,
            "--open" => a.open = true,
            "--paste" => a.paste = true,
            other if !other.starts_with("--") => a.file = Some(PathBuf::from(other)),
            _ => {}
        }
    }
    a
}

/// Remembered between runs: the last folder used to save, and the colour and size.
#[derive(Default, Serialize, Deserialize)]
struct State {
    last_dir: Option<PathBuf>,
    color: Option<usize>,
    size: Option<usize>,
}

/// Writes the colour and size to the state file when they changed, so the next image (and the
/// next editor) starts with them.
fn persist_style(ed: &Editor) {
    thread_local! {
        static LAST: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
    }
    let now = (ed.color, ed.size);
    if LAST.with(|l| l.replace(Some(now))) == Some(now) {
        return;
    }
    let mut s = load_state();
    if (s.color, s.size) != (Some(now.0), Some(now.1)) {
        s.color = Some(now.0);
        s.size = Some(now.1);
        save_state(&s);
    }
}

/// A new editor with the remembered colour and size.
fn editor_for(img: &Image, scale: f32, color: usize, size: usize) -> Editor {
    let mut ed = Editor::new(img.width, img.height, scale);
    ed.color = color.min(glint_editor::editor::PALETTE.len() - 1);
    ed.size = size.min(glint_editor::editor::SIZES.len() - 1);
    ed
}

fn state_path() -> PathBuf {
    glint_core::paths::config_file().with_file_name("editor-state.toml")
}

fn load_state() -> State {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_state(s: &State) {
    if let Ok(t) = toml::to_string(s) {
        let p = state_path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, t);
    }
}

fn load_image(path: &Path) -> Result<Image, String> {
    let img = image::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    Ok(Image {
        width: w,
        height: h,
        data: img.into_raw(),
    })
}

fn pick_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Open an image in Glint")
        .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp", "gif"])
        .pick_file()
}

fn hwnd() -> Option<HWND> {
    let h = unsafe { GetActiveWindow() };
    (!h.is_invalid()).then_some(h)
}

fn message(text: &str) {
    let t: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            hwnd(),
            PCWSTR(t.as_ptr()),
            windows::core::w!("Glint"),
            MB_ICONWARNING,
        );
    }
}

struct App {
    ed: Editor,
    r: Renderer,
    name: String,
    save_dir: PathBuf,
    temp_file: Option<PathBuf>,
    state: State,
    status: String,
}

impl App {
    fn new(img: &Image, name: String, save_dir: PathBuf, scale: f32) -> App {
        let state = load_state();
        let defaults = Editor::new(1, 1, scale);
        let ed = editor_for(
            img,
            scale,
            state.color.unwrap_or(defaults.color),
            state.size.unwrap_or(defaults.size),
        );
        App {
            ed,
            r: Renderer::new(img),
            name,
            save_dir,
            temp_file: None,
            state,
            status: String::new(),
        }
    }

    fn copy(&mut self) -> bool {
        let img = self.r.export(&self.ed);
        // Tests: write the result to GLINT_TEST_DIR instead of touching the user's clipboard.
        if let Some(dir) = std::env::var_os("GLINT_TEST_DIR") {
            let _ = std::fs::create_dir_all(&dir);
            let ok = std::fs::write(
                Path::new(&dir).join("editor-export.png"),
                glint_core::encode::encode_png_fast(&img),
            )
            .is_ok();
            self.ed.dirty = !ok;
            return ok;
        }
        match clipboard::set_image(&img, hwnd()) {
            Ok(()) => {
                self.ed.dirty = false;
                self.status = "Copied to the clipboard".into();
                true
            }
            Err(e) => {
                self.status = format!("Copy failed: {e}");
                false
            }
        }
    }

    fn save(&mut self) {
        let dir = self
            .state
            .last_dir
            .clone()
            .filter(|d| d.is_dir())
            .unwrap_or_else(|| self.save_dir.clone());
        let _ = std::fs::create_dir_all(&dir);
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save the image")
            .set_directory(&dir)
            .set_file_name(&self.name)
            .add_filter("PNG image", &["png"])
            .add_filter("JPEG image", &["jpg", "jpeg"])
            .add_filter("WebP image (lossless)", &["webp"])
            .save_file()
        else {
            return;
        };
        let img = self.r.export(&self.ed);
        match write_image(&img, &path) {
            Ok(()) => {
                self.ed.dirty = false;
                self.status = format!("Saved {}", path.display());
                self.state.last_dir = path.parent().map(Path::to_path_buf);
                self.state.color = Some(self.ed.color);
                self.state.size = Some(self.ed.size);
                save_state(&self.state);
            }
            Err(e) => message(&format!("Could not save the image.\n\n{e}")),
        }
    }
}

fn write_image(img: &Image, path: &Path) -> Result<(), String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png")
        .to_ascii_lowercase();
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut w = std::io::BufWriter::new(file);
    match ext.as_str() {
        "jpg" | "jpeg" => {
            // JPEG has no alpha: put transparent parts on white.
            let mut rgb = Vec::with_capacity(img.data.len() / 4 * 3);
            for p in img.data.chunks_exact(4) {
                let a = p[3] as u32;
                for c in 0..3 {
                    rgb.push(((p[c] as u32 * a + 255 * (255 - a)) / 255) as u8);
                }
            }
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut w, 92)
                .encode(&rgb, img.width, img.height, image::ExtendedColorType::Rgb8)
                .map_err(|e| e.to_string())
        }
        "webp" => image::codecs::webp::WebPEncoder::new_lossless(&mut w)
            .encode(
                &img.data,
                img.width,
                img.height,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string()),
        _ => {
            use std::io::Write;
            w.write_all(&glint_core::encode::encode_png_best(img))
                .map_err(|e| e.to_string())
        }
    }
}

fn color(c: [u8; 4]) -> Color {
    Color::from_argb_u8(c[3], c[0], c[1], c[2])
}

/// Pushes the editor state to the window.
fn refresh(ui: &EditorWindow, app: &App) {
    let pm = app.r.render(&app.ed, true);
    let (w, h) = (pm.width(), pm.height());
    let img = if app.r.is_opaque() {
        SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(pm.data(), w, h)
    } else {
        let straight = glint_editor::render::to_image(&pm);
        SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&straight.data, w, h)
    };
    ui.set_canvas(slint::Image::from_rgba8(img));
    ui.set_img_w(w as i32);
    ui.set_img_h(h as i32);
    refresh_chrome(ui, app);
}

/// Everything except the canvas pixels.
fn refresh_chrome(ui: &EditorWindow, app: &App) {
    ui.set_tool(app.ed.tool.id().into());
    ui.set_color_index(app.ed.color as i32);
    ui.set_size_index(app.ed.size as i32);
    ui.set_brush(app.ed.brush());
    persist_style(&app.ed);
    ui.set_can_undo(app.ed.can_undo());
    ui.set_can_redo(app.ed.can_redo());
    ui.set_crop_pending(app.ed.crop_pending.is_some());
    // The crop frame, as parts (0..1) of the shown image; the window draws it over the image.
    match app.ed.crop_pending.filter(|_| app.ed.tool == Tool::Crop) {
        Some(c) => {
            let v = app.ed.view();
            ui.set_crop_frame(CropFrame {
                show: true,
                x0: (c.x0 - v.x0) / v.w(),
                y0: (c.y0 - v.y0) / v.h(),
                x1: (c.x1 - v.x0) / v.w(),
                y1: (c.y1 - v.y0) / v.h(),
                grid: app.ed.crop_dragging(),
            });
        }
        None => ui.set_crop_frame(CropFrame::default()),
    }
    // Only results of actions (copied, saved, errors); no tool hints (owner, 2026-11-12).
    ui.set_status(app.status.clone().into());
    let dot = if app.ed.dirty { " \u{2022}" } else { "" };
    ui.set_window_title(format!("{}{} - Glint", app.name, dot).into());
    match &app.ed.text_edit {
        Some(t) => {
            let v = app.ed.view();
            ui.set_text_x(t.at.x - v.x0);
            ui.set_text_y(t.at.y - v.y0);
            ui.set_text_size(text_size(t.style.width));
            ui.set_text_color(color(t.style.color));
            if !ui.get_text_editing() {
                ui.set_text_value(t.text.clone().into());
                ui.set_text_editing(true);
                ui.invoke_focus_text();
            }
        }
        None => {
            if ui.get_text_editing() {
                ui.set_text_editing(false);
                ui.invoke_focus_canvas();
            }
        }
    }
}

/// Asks before closing with unsaved, uncopied edits. Returns true when it is fine to close.
fn confirm_close(app: &mut App) -> bool {
    if !app.ed.dirty {
        return true;
    }
    let text: Vec<u16> =
        "Your edits are not saved or copied.\n\nCopy the image to the clipboard before closing?"
            .encode_utf16()
            .chain(Some(0))
            .collect();
    let answer = unsafe {
        MessageBoxW(
            hwnd(),
            PCWSTR(text.as_ptr()),
            windows::core::w!("Glint"),
            MB_YESNOCANCEL | MB_ICONWARNING,
        )
    };
    if answer == IDCANCEL {
        return false;
    }
    if answer == IDYES {
        return app.copy();
    }
    true
}

fn cursor_monitor() -> (glint_core::Rect, f32) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    unsafe {
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let mon = MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dx, mut dy) = (96, 96);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let r = info.rcWork;
        (
            glint_core::Rect::from_ltrb(r.left, r.top, r.right, r.bottom),
            dx as f32 / 96.0,
        )
    }
}

fn main() {
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args = parse_args();
    let mut file = args.file.clone();
    if args.open {
        file = pick_file();
        if file.is_none() {
            return;
        }
    }
    let image = match &file {
        Some(p) => load_image(p),
        None => match clipboard::get_image(None) {
            Ok(Some(img)) => Ok(img),
            Ok(None) if args.paste => Err("There is no image on the clipboard.".into()),
            _ => match pick_file() {
                Some(p) => {
                    file = Some(p.clone());
                    load_image(&p)
                }
                None => return,
            },
        },
    };
    let image = match image {
        Ok(i) => i,
        Err(e) => {
            message(&format!("Glint cannot open this image.\n\n{e}"));
            return;
        }
    };
    let name = args.name.clone().unwrap_or_else(|| {
        let stem = file
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string());
        format!(
            "{}.png",
            stem.unwrap_or_else(|| glint_core::naming::file_name(
                "",
                glint_core::naming::Stamp::now(),
                "png"
            )
            .trim_end_matches(".png")
            .into())
        )
    });
    let save_dir = args
        .save_dir
        .clone()
        .unwrap_or_else(glint_core::paths::default_save_dir);
    let (work, scale) = cursor_monitor();

    let ui = EditorWindow::new().expect("create window");
    let mut app = App::new(&image, name, save_dir, scale);
    if args.temp {
        app.temp_file = file.clone();
    }
    let app = Rc::new(RefCell::new(app));

    // Window size: the image at 100 % plus the toolbars, capped to 92 % of the work area.
    let chrome = ((50.0 + 24.0) * scale) as i32;
    let pad = (24.0 * scale) as i32;
    let w = (image.width as i32 + pad).clamp((980.0 * scale) as i32, work.w * 92 / 100);
    let h = (image.height as i32 + chrome).clamp((360.0 * scale) as i32, work.h * 92 / 100);
    ui.window()
        .set_size(slint::PhysicalSize::new(w as u32, h as u32));
    ui.window().set_position(slint::PhysicalPosition::new(
        work.x + (work.w - w) / 2,
        work.y + (work.h - h) / 2,
    ));

    let tools: Vec<ToolItem> = Tool::ALL
        .iter()
        .map(|t| ToolItem {
            id: t.id().into(),
            glyph: t.glyph().into(),
            tip: t.name().into(),
            key: t.key().to_string().into(),
        })
        .collect();
    ui.set_tools(ModelRc::new(VecModel::from(tools)));
    ui.set_palette(ModelRc::new(VecModel::from(
        PALETTE.iter().map(|c| color(*c)).collect::<Vec<_>>(),
    )));

    let weak = ui.as_weak();
    let with = move |f: &dyn Fn(&mut App, &EditorWindow) -> bool| {
        let Some(ui) = weak.upgrade() else { return };
        let rc = app_ref();
        let redraw = {
            let Ok(mut a) = rc.try_borrow_mut() else {
                return;
            };
            f(&mut a, &ui)
        };
        let a = rc.borrow();
        if redraw {
            refresh(&ui, &a);
        } else {
            refresh_chrome(&ui, &a);
        }
    };
    // The callbacks below share the app through this thread-local handle.
    APP.with(|cell| *cell.borrow_mut() = Some(app.clone()));
    fn app_ref() -> Rc<RefCell<App>> {
        APP.with(|c| c.borrow().clone().expect("app"))
    }

    {
        let with = with.clone();
        ui.on_tool_selected(move |id| {
            with(&|a, _| {
                if let Some(t) = Tool::from_id(&id) {
                    a.status.clear();
                    a.ed.set_tool(t);
                }
                true
            })
        });
    }
    {
        let with = with.clone();
        ui.on_color_selected(move |i| {
            with(&|a, _| {
                a.ed.set_color(i as usize);
                true
            })
        });
    }
    {
        let with = with.clone();
        ui.on_size_step(move |d| {
            with(&|a, _| {
                a.ed.bump_size(d);
                true
            })
        });
    }
    {
        let with = with.clone();
        let with_settle = with.clone();
        ui.on_pointer(move |kind, x, y, shift, ctrl, alt| {
            let m = Mods { shift, ctrl, alt };
            let kind = kind.to_string();
            with(&|a, ui| {
                match kind.as_str() {
                    "down" => {
                        // Grabbing the crop frame again restarts the pause before it crops.
                        SETTLE.with(|t| t.stop());
                        a.status.clear();
                        // Stroke widths follow the window's real scale (known once it is on screen).
                        a.ed.scale = ui.window().scale_factor();
                        if a.ed.text_edit.is_some() {
                            a.ed.commit_text(&ui.get_text_value());
                            ui.set_text_editing(false);
                        }
                        a.ed.pointer_down(x, y, m);
                        // The crop frame is drawn by the window; the picture does not change.
                        a.ed.tool != Tool::Crop
                    }
                    "up" => {
                        let crop = a.ed.tool == Tool::Crop;
                        a.ed.pointer_up(x, y, m);
                        if a.ed.crop_waiting() {
                            let with = with_settle.clone();
                            SETTLE.with(|t| {
                                t.start(TimerMode::SingleShot, CROP_PAUSE, move || {
                                    with(&|a, ui| settle_crop(a, ui));
                                })
                            });
                        }
                        !crop
                    }
                    _ => {
                        ui.set_cursor(a.ed.cursor(x, y).into());
                        if a.ed.is_dragging() {
                            // Dragging the crop frame only moves the window's overlay: no
                            // re-render and no new picture per mouse move (it lagged on big
                            // pictures). The crop itself happens after letting go.
                            let crop = a.ed.crop_dragging();
                            a.ed.pointer_move(x, y, m);
                            !crop
                        } else {
                            false
                        }
                    }
                }
            })
        });
    }
    {
        let with = with.clone();
        ui.on_double_click(move |x, y| {
            with(&|a, _| {
                a.ed.double_click(x, y);
                true
            })
        });
    }
    {
        let with = with.clone();
        ui.on_commit_text(move |t| {
            let t = t.to_string();
            with(&|a, ui| {
                a.ed.commit_text(&t);
                ui.set_text_editing(false);
                true
            })
        });
    }
    {
        let with = with.clone();
        ui.on_action(move |name| {
            let name = name.to_string();
            with(&|a, ui| run_action(a, ui, &name))
        });
    }
    {
        let with = with.clone();
        ui.on_key(move |text, ctrl, shift, alt| {
            let handled = std::cell::Cell::new(true);
            with(&|a, ui| {
                let (h, redraw) = on_key(a, ui, &text, ctrl, shift, alt);
                handled.set(h);
                redraw
            });
            handled.get()
        });
    }
    {
        let weak = ui.as_weak();
        ui.window().on_close_requested(move || {
            let app = app_ref();
            let ok = confirm_close(&mut app.borrow_mut());
            if ok {
                if let Some(ui) = weak.upgrade() {
                    let _ = ui.hide();
                }
                let _ = slint::quit_event_loop();
                CloseRequestResponse::HideWindow
            } else {
                CloseRequestResponse::KeepWindowShown
            }
        });
    }

    refresh(&ui, &app.borrow());
    ui.invoke_focus_canvas();
    let _ = ui.show();
    // Match stroke widths to the real window scale (the monitor guess can differ).
    app.borrow_mut().ed.scale = ui.window().scale_factor();
    let _ = slint::run_event_loop_until_quit();
    let temp = app.borrow().temp_file.clone();
    if let Some(t) = temp {
        let _ = std::fs::remove_file(t);
    }
}

thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
    /// Crops a moment after the crop frame is let go (iOS style).
    static SETTLE: slint::Timer = slint::Timer::default();
}

/// How long a changed crop frame waits, so the handles can be grabbed again first.
const CROP_PAUSE: Duration = Duration::from_millis(1000);
/// The zoom from the frame to the full cropped image.
const CROP_ANIM: Duration = Duration::from_millis(280);

/// Crops to the frame and zooms the kept part up to fill the canvas. Undo brings it back.
fn settle_crop(a: &mut App, ui: &EditorWindow) -> bool {
    let (Some(c), v) = (a.ed.crop_pending, a.ed.view()) else {
        return false;
    };
    if !a.ed.crop_waiting() {
        return false;
    }
    // Where the frame is now, as parts of the shown image: the cropped image starts there.
    ui.invoke_begin_settle(
        (c.x0 - v.x0) / v.w(),
        (c.y0 - v.y0) / v.h(),
        c.w() / v.w(),
        c.h() / v.h(),
    );
    let done = a.ed.apply_crop();
    let weak = ui.as_weak();
    // Next frame: animate from the frame to the normal fitted place.
    slint::Timer::single_shot(Duration::from_millis(16), move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_settle_anim(true);
            ui.set_settling(false);
            let weak = ui.as_weak();
            slint::Timer::single_shot(CROP_ANIM + Duration::from_millis(40), move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_settle_anim(false);
                }
            });
        }
    });
    done
}

/// Toolbar and shortcut actions. Returns true when the canvas must be redrawn.
fn run_action(a: &mut App, ui: &EditorWindow, name: &str) -> bool {
    a.status.clear();
    match name {
        "undo" => a.ed.undo(),
        "redo" => a.ed.redo(),
        "copy" => {
            a.copy();
        }
        "save" => a.save(),
        "close" => {
            if confirm_close(a) {
                let _ = ui.hide();
                let _ = slint::quit_event_loop();
            }
            return false;
        }
        "copy-close" => {
            if a.copy() {
                let _ = ui.hide();
                let _ = slint::quit_event_loop();
            }
            return false;
        }
        "paste" => match clipboard::get_image(hwnd()) {
            Ok(Some(img)) if a.ed.shapes.is_empty() => {
                a.ed = editor_for(&img, a.ed.scale, a.ed.color, a.ed.size);
                a.r = Renderer::new(&img);
                a.status = "Pasted the clipboard image".into();
            }
            Ok(Some(_)) => {
                a.status = "Paste replaces the image only while there are no annotations".into()
            }
            _ => a.status = "There is no image on the clipboard".into(),
        },
        "open" => {
            if let Some(p) = pick_file() {
                let exe = std::env::current_exe().unwrap_or_else(|_| "glint-editor.exe".into());
                let _ = std::process::Command::new(exe).arg("--file").arg(p).spawn();
            }
        }
        _ => {}
    }
    true
}

/// Tests only: F12 writes what the window shows to GLINT_TEST_DIR/editor-window.png.
fn snapshot(ui: &EditorWindow) {
    let Some(dir) = std::env::var_os("GLINT_TEST_DIR") else {
        return;
    };
    if let Ok(buf) = ui.window().take_snapshot() {
        let img = Image {
            width: buf.width(),
            height: buf.height(),
            data: buf.as_bytes().to_vec(),
        };
        let _ = std::fs::write(
            Path::new(&dir).join("editor-window.png"),
            glint_core::encode::encode_png_fast(&img),
        );
    }
}

fn is(text: &str, k: Key) -> bool {
    SharedString::from(k).as_str() == text
}

/// Keyboard shortcuts. Returns (handled, redraw).
fn on_key(
    a: &mut App,
    ui: &EditorWindow,
    text: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
) -> (bool, bool) {
    let _ = alt;
    if is(text, Key::Escape) {
        if a.ed.text_edit.is_some() {
            a.ed.cancel_text();
            ui.set_text_editing(false);
            return (true, true);
        }
        if a.ed.cancel_crop() {
            return (true, true);
        }
        if a.ed.sel.take().is_some() {
            return (true, true);
        }
        return (true, run_action(a, ui, "close"));
    }
    if is(text, Key::F12) {
        snapshot(ui);
        return (true, false);
    }
    // Tests only: F11 opens the arrow tool's tooltip (a hidden desktop has no real cursor, so
    // Windows ends a posted hover at once).
    if is(text, Key::F11) && std::env::var_os("GLINT_TEST_DIR").is_some() {
        ui.invoke_show_tip("Arrow".into(), "A".into(), 61.0);
        return (true, false);
    }
    if a.ed.text_edit.is_some() {
        return (false, false); // the text box handles typing
    }
    if is(text, Key::Return) {
        if a.ed.crop_waiting() {
            SETTLE.with(|t| t.stop());
            return (true, settle_crop(a, ui));
        }
        return (true, run_action(a, ui, "copy-close"));
    }
    if is(text, Key::Delete) || is(text, Key::Backspace) {
        return (true, a.ed.delete_selected());
    }
    let step = if shift { 10.0 } else { 1.0 };
    for (k, dx, dy) in [
        (Key::LeftArrow, -step, 0.0),
        (Key::RightArrow, step, 0.0),
        (Key::UpArrow, 0.0, -step),
        (Key::DownArrow, 0.0, step),
    ] {
        if is(text, k) {
            if let Some(i) = a.ed.sel {
                a.ed.nudge(i, dx, dy);
                return (true, true);
            }
            return (true, false);
        }
    }
    let c = text.chars().next().unwrap_or('\0').to_ascii_lowercase();
    if ctrl {
        let action = match (c, shift) {
            ('z', false) => "undo",
            ('z', true) | ('y', _) => "redo",
            ('c', _) => "copy",
            ('s', _) => "save",
            ('v', _) => "paste",
            ('o', _) => "open",
            ('w', _) => "close",
            _ => return (false, false),
        };
        return (true, run_action(a, ui, action));
    }
    match c {
        '1'..='6' => {
            a.ed.set_color(c as usize - '1' as usize);
            (true, true)
        }
        '[' => {
            a.ed.bump_size(-1);
            (true, true)
        }
        ']' => {
            a.ed.bump_size(1);
            (true, true)
        }
        _ => match Tool::from_key(c) {
            Some(t) => {
                a.status.clear();
                a.ed.set_tool(t);
                (true, true)
            }
            None => (false, false),
        },
    }
}
