# Glint

Windows 11 snipping tool in Rust. Spec, plan and decisions: `docs/2026-11-12-v1-spec-and-plan.md`.
Research (feature ideas, stack, capture APIs): `C:\Users\Admin\Documents\Claude\research\glint\2026-11-12-brainstorm-and-stack.md`.

## Layout
- `crates/glint-core`: geometry, Image, PNG/DIB codecs, clipboard, config.toml, naming, icon pixels.
- `crates/glint-capture`: GDI capture (default), DXGI FP16 HDR tone map (opt-in `hdr_capture`), monitors.
- `crates/glint`: tray daemon, plain Win32/GDI on purpose (idle RAM ~1.9 MB). Overlay, preview card.
- `crates/glint-editor`: Slint 1.18 (software renderer) + tiny-skia; model/editor/render are UI-free and unit-tested.

## Rules that are easy to break
- Never add a UI toolkit, GPU context or webview to `glint.exe`: the idle RAM target is < 10 MB (e2e checks it).
- Tests must never touch the user's clipboard or screen: `GLINT_TEST_DIR` makes daemon and editor write files
  instead; e2e runs on a hidden desktop with `GLINT_TEST_SYNTHETIC` (GDI cannot copy a hidden desktop).
- Processes that post mouse messages in tests must be per-monitor DPI aware, or Windows rescales the coordinates.
- No global hotkeys that apps use (Ctrl+Shift+S/W/F/R were rejected). Default is only Ctrl+Alt+S.
- No window capture (owner decision, 2026-11-12).

## Build, test, release
- Typecheck/lint: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings`
- Unit: `cargo test --workspace`
- E2E (headless): `cargo build --release && pwsh tools/e2e/run.ps1 -Release`    Run when: crates/glint/**, crates/glint-editor/**, crates/glint-capture/**
- Build / package: `cargo build --release; makensis /DVERSION=<version> installer\glint.nsi`   Artifact: `Glint-Setup-x64-<version>.exe`
- Known failures to tolerate: none
- Version source: `Cargo.toml` `[workspace.package] version`   Release: release.yml on push to main
- Install locally after merge: `dist\Glint-Setup-x64-<version>.exe /S`   Confirm version: `(Get-Item "$env:LOCALAPPDATA\Programs\Glint\glint.exe").VersionInfo.ProductVersion`
- Deploy: installer via GitHub Releases
- Signing: unsigned
