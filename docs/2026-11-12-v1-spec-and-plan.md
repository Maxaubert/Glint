# Glint v1: spec and implementation plan (2026-11-12)

Research behind this: `C:\Users\Admin\Documents\Claude\research\glint\2026-11-12-brainstorm-and-stack.md`.
Decisions by Max on 2026-11-12: v1 is **core only**; editor UI is **Slint**.

## Changes after approval (Max, 2026-11-12)
- One default hotkey, **Ctrl+Alt+S**, opens the overlay; pressing it again closes it. The spec's
  Ctrl+Shift+S/W/F/R set was dropped because global hotkeys stole Save As, close-window, find and reload.
- In the overlay: **right-click** snips the monitor under the cursor, **middle-click** repeats the last area.
  Optional `fullscreen` and `repeat` hotkeys exist in config.toml, empty by default.
- **Window capture is removed** (no window mode, no window hotkey, a click without a drag clears the selection).
- HDR: GDI already gives correct colours for desktop content on this PC, so the DXGI tone-mapped path
  is opt-in (`hdr_capture = true`), see `crates/glint-capture/src/lib.rs`.
- The rest of 1.3 below that mentions window mode or the old hotkeys is superseded by this list.

## 1. Spec

### 1.1 Goal
A Windows 11 snipping tool that idles in the tray with very low RAM, captures instantly,
copies to the clipboard by default, and opens a fast editor only when asked.

### 1.2 Process model
| Process | Lifetime | Built with | RAM target |
|---|---|---|---|
| `glint.exe` (daemon) | Always running, starts with Windows (opt-in) | Pure Win32 via `windows` crate, GDI/Direct2D painting. No UI toolkit, no GPU context at idle. | < 10 MB private working set at idle |
| `glint-editor.exe` | Spawned when the preview is clicked, exits on close | Slint (software renderer first) + tiny-skia for annotations | No hard target; frees all on close |

Hand-off: the daemon writes the capture as PNG to `%LOCALAPPDATA%\Glint\tmp\<id>.png` and starts
the editor with that path. The editor deletes the temp file on exit. Nothing else touches disk.

### 1.3 Capture
- **Hotkeys** (rebindable in a config file in v1, no settings UI yet):
  region `Ctrl+Shift+S`, window `Ctrl+Shift+W`, fullscreen `Ctrl+Shift+F`, repeat last region `Ctrl+Shift+R`.
  Optional Print Screen takeover: Glint sets `PrintScreenKeyForSnippingEnabled=0` only after the user agrees, then registers `VK_SNAPSHOT`.
  Win+Shift+S is reserved by Windows and is not used.
- **Freeze first:** on hotkey, grab every monitor at once (DXGI Desktop Duplication, BitBlt fallback), then show the overlay over the frozen frames. The crop comes from the frozen frames. No yellow border.
- **Region overlay:** one topmost borderless window per monitor. Dims the frozen image, crosshair cursor,
  drag to select, live `W x H` readout, magnifier loupe with pixel grid and hex color, arrow keys nudge 1 px
  (Shift+arrow resizes, Ctrl = 10 px), edge and corner handles stay live after the drag, Enter or double-click confirms, Esc cancels.
  In the overlay: `W` switches to window mode, `F` to fullscreen.
- **Window mode:** hover highlights the window under the cursor (`WindowFromPoint` + `DWMWA_EXTENDED_FRAME_BOUNDS`, no invisible shadow), click captures it. v1 crops it from the frozen frame (so it matches what was visible). Occluded-window capture via WGC is post-v1.
- **Fullscreen:** the monitor under the cursor. Shift = all monitors stitched.
- **DPI:** per-monitor DPI aware v2 manifest; all math in physical pixels; mixed-DPI selections are stitched without resampling.
- **HDR:** if a monitor gives FP16, divide by its SDR white level and tone-map to 8-bit sRGB. If this proves hard, v1 may fall back to BitBlt on HDR monitors and log it.
- **Cursor:** not included in v1.

### 1.4 After capture
- Copy to the clipboard at once: registered `PNG`, `CF_DIBV5`, `CF_DIB`. Not saved.
- **Preview toast** at the bottom-right of the monitor where the capture happened: thumbnail, slides in, does not take focus, auto-hides after 6 s (paused on hover). Rapid captures stack (max 3).
- Hover shows buttons: **Edit**, **Save**, **Close**. Clicking the thumbnail = Edit.
- **Drag-out:** dragging the thumbnail drops a real PNG file into Explorer, Discord, Slack and so on (OLE `DoDragDrop` with a temp file).
- Quick Save from the toast uses the default folder (Pictures\Glint) and name `Glint_<app>_<yyyy-MM-dd_HH-mm-ss>.png`.

### 1.5 Editor (Slint)
- Window sized to the image (fits the screen), dark Windows 11 look, follows system theme.
- **Tools and keys:** Select `V`, Arrow `A`, Rectangle `R`, Ellipse `O`, Highlighter `H`, Pen `P`, Text `T`,
  Step number `N`, Blur `B`, Pixelate `X`, Crop `C`. Digits 1–6 pick a color from a fixed palette; `[` `]` change width.
- **Arrow:** straight by default; drag the midpoint handle to curve; tapered style; Shift snaps to 15°.
- **Ellipse/Rectangle:** outline by default; Shift = circle/square; Alt = from center.
- **Highlighter:** multiply blend, Shift = straight line.
- **Step numbers:** auto-increment, renumber on delete.
- **Blur/Pixelate:** baked into pixels on export (not reversible).
- All annotations stay editable objects (move, resize, recolor, delete) until export. Unlimited undo/redo (`Ctrl+Z`, `Ctrl+Y`).
- Each tool remembers its last color and width.
- **Output:** `Ctrl+C` copies the flattened image, `Ctrl+S` saves PNG (save dialog, remembers folder), `Ctrl+Shift+S` save as JPEG/WebP, `Enter` = copy and close, `Esc` = close (asks only if there are unsaved changes that were not copied).
- The editor can also open from `glint-editor.exe <file>` and paste with `Ctrl+V` (cheap, falls out of the design).

### 1.6 Tray and config
- Tray icon menu: Region, Window, Fullscreen, Open image…, Open config, Start with Windows (toggle), Quit.
- Config: `%APPDATA%\Glint\config.toml` (hotkeys, save folder, toast timeout, Print Screen takeover). Reloaded on change.
- Single instance (named mutex). A second launch triggers a region capture (so a shortcut works as a launcher).

### 1.7 Out of scope for v1
OCR, pin to screen, color picker mode, delay timer, scrolling capture, recording, history, auto-redact,
beautify, shape snapping, upload, AI features, settings UI, installer auto-update. All listed in the research doc for later.

### 1.8 Quality bars
- Hotkey to overlay visible: < 150 ms on this PC.
- Daemon idle private working set: < 10 MB, measured.
- No crash loses a capture: if anything after the clipboard write fails, the clipboard still holds the image.
- Tests never take the screen: E2E runs on a hidden desktop (reuse Prism's `tools/e2e/hidden.mjs` + `HiddenDesktop.cs` pattern).

## 2. Implementation plan

Cargo workspace, one shared `CARGO_TARGET_DIR` for any worktrees.

```
crates/
  glint-core/     geometry (rects, DPI, monitor layout), image buffer, PNG/DIB encode, clipboard, config, paths
  glint-capture/  DXGI + BitBlt capture, HDR tone map, window hit-testing
  glint/          daemon: tray, hotkeys, overlay, toast, drag-out, spawn editor
  glint-editor/   Slint UI, annotation model, tiny-skia renderer, undo, export
tools/e2e/        hidden-desktop runner
```

Tasks, in order (each ends with tests passing; one PR per group):

1. **Workspace + CI.** Workspace, `rust-toolchain`, clippy/fmt, GitHub Actions on `windows-latest` (build, test, clippy). README stub.
2. **glint-core.** Rect/selection math with unit tests (nudge, resize handles, multi-monitor negative coords, mixed DPI stitching). PNG encode (`png` crate), DIB/DIBV5 build. Clipboard write of the three formats (`clipboard-win` or raw Win32) with a round-trip test. Config load with defaults (`serde` + `toml`). Paths.
3. **glint-capture.** Monitor enumeration, DXGI Desktop Duplication one-shot per output, BitBlt fallback, HDR FP16 → sRGB tone map (unit-tested on synthetic FP16 data). Window list + extended frame bounds.
4. **Daemon skeleton.** Message-only window, single instance, tray (`tray-icon` + `muda`), `RegisterHotKey`, config reload. Measure idle RAM here and record it.
5. **Region overlay.** Per-monitor layered windows, frozen frame, dim, crosshair, drag, handles, readout, loupe, keyboard nudge, W/F mode switch, Enter/Esc. Direct2D painting. Repeat-last-region.
6. **Window + fullscreen modes.** Hover highlight, click capture, Shift for all monitors.
7. **Preview toast.** Slide-in, no-activate, timer, stack, hover buttons, quick save with smart name, drag-out via OLE.
8. **Editor shell (Slint).** Open from path, fit to screen, toolbar, key bindings, theme, Ctrl+V/open file.
9. **Annotation model + renderer.** Object list, tiny-skia render to a pixel buffer shown in Slint, hit-testing, select/move/resize, undo/redo stack. Snapshot tests per tool (render → compare PNG).
10. **Tools.** Arrow (curve, taper, snap), rect, ellipse, highlighter, pen, text, steps, blur, pixelate, crop. Per-tool remembered style.
11. **Export.** Flatten, copy, save PNG/JPEG/WebP, unsaved-changes guard, temp file cleanup.
12. **Print Screen takeover + start with Windows.** Consent prompt in tray menu, registry value, Run key.
13. **E2E on hidden desktop.** Hotkey → overlay → select → clipboard has PNG of expected size; preview → editor → draw arrow → save → file exists. Plus perf checks (hotkey latency, idle RAM).
14. **Packaging.** Release build, `cargo-packager` (MSI/NSIS) or Velopack. Release flow per the `shipping` skill. README with screenshots.

Open risks to check early (spikes inside tasks 3–5): DXGI on this PC's GPU and HDR TV setup; whether
the Slint software renderer is fast enough for a 4K canvas (fallback: femtovg renderer); overlay latency.
