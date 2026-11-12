//! Windows.Graphics.Capture of one monitor. Unlike GDI and DXGI Desktop Duplication, it returns
//! the desktop as DWM composes it BEFORE a fullscreen magnifier's transform: under Wind's zoom it
//! still sees the whole real screen (measured 2026-11-12 with the `wgcprobe` example, Wind at 3-5x).
//! The Windows Snipping Tool gets its unzoomed picture the same way, with no visible blink.
//!
//! d3d11.dll is loaded only here, at capture time, like hdr.rs (static imports made RTSS attach to
//! the idle daemon). Each call builds and drops its own device, so nothing stays resident.

use crate::hdr::proc_addr;
use glint_core::{Image, Rect};
use std::ffi::c_void;
use windows::core::{IInspectable, Interface, HRESULT};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, POINT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

fn device() -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
    type F = unsafe extern "system" fn(
        *mut c_void,
        D3D_DRIVER_TYPE,
        HMODULE,
        u32,
        *const c_void,
        u32,
        u32,
        *mut *mut c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT;
    let f: F = unsafe {
        std::mem::transmute(proc_addr(
            windows::core::w!("d3d11.dll"),
            windows::core::s!("D3D11CreateDevice"),
        )?)
    };
    let (mut dev, mut c) = (std::ptr::null_mut(), std::ptr::null_mut());
    unsafe {
        f(
            std::ptr::null_mut(),
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT.0,
            std::ptr::null(),
            0,
            D3D11_SDK_VERSION,
            &mut dev,
            std::ptr::null_mut(),
            &mut c,
        )
    }
    .ok()
    .map_err(|e| format!("D3D11CreateDevice: {e}"))?;
    unsafe {
        Ok((
            ID3D11Device::from_raw(dev),
            ID3D11DeviceContext::from_raw(c),
        ))
    }
}

/// The WinRT wrapper of a D3D11 device, also loaded from d3d11.dll at run time.
fn winrt_device(d: &ID3D11Device) -> Result<IDirect3DDevice, String> {
    type F = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;
    let f: F = unsafe {
        std::mem::transmute(proc_addr(
            windows::core::w!("d3d11.dll"),
            windows::core::s!("CreateDirect3D11DeviceFromDXGIDevice"),
        )?)
    };
    let dxgi: IDXGIDevice = d.cast().map_err(|e| e.to_string())?;
    let mut raw = std::ptr::null_mut();
    unsafe { f(dxgi.as_raw(), &mut raw) }
        .ok()
        .map_err(|e| format!("CreateDirect3D11DeviceFromDXGIDevice: {e}"))?;
    let insp = unsafe { IInspectable::from_raw(raw) };
    insp.cast().map_err(|e| e.to_string())
}

/// Captures the monitor that holds `rect`'s centre, without the cursor. Physical pixels.
pub fn capture_monitor(rect: &Rect) -> Result<Image, String> {
    let e = |e: windows::core::Error| e.to_string();
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let (dev, ctx) = device()?;
        let d3d = winrt_device(&dev)?;
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(e)?;
        let mon = MonitorFromPoint(
            POINT {
                x: rect.x + rect.w / 2,
                y: rect.y + rect.h / 2,
            },
            MONITOR_DEFAULTTONEAREST,
        );
        let item: GraphicsCaptureItem = interop.CreateForMonitor(mon).map_err(e)?;
        let size = item.Size().map_err(e)?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            size,
        )
        .map_err(e)?;
        let session = pool.CreateCaptureSession(&item).map_err(e)?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        // No yellow capture border (allowed for a one-frame capture on Windows 11).
        let _ = session.SetIsBorderRequired(false);
        session.StartCapture().map_err(e)?;
        let t = std::time::Instant::now();
        let frame = loop {
            if let Ok(f) = pool.TryGetNextFrame() {
                break f;
            }
            if t.elapsed().as_millis() > 1000 {
                let _ = session.Close();
                let _ = pool.Close();
                return Err("no frame within 1 s".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface().map_err(e)?.cast().map_err(e)?;
        let tex: ID3D11Texture2D = access.GetInterface().map_err(e)?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        tex.GetDesc(&mut desc);
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        dev.CreateTexture2D(&desc, None, Some(&mut staging))
            .map_err(e)?;
        let staging = staging.ok_or("no staging texture")?;
        ctx.CopyResource(&staging, &tex);
        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map))
            .map_err(e)?;
        let (w, h) = (desc.Width, desc.Height);
        let src = std::slice::from_raw_parts(map.pData as *const u8, (map.RowPitch * h) as usize);
        let img = Image::from_bgra(w, h, src, map.RowPitch as usize, true);
        ctx.Unmap(&staging, 0);
        let _ = frame.Close();
        let _ = session.Close();
        let _ = pool.Close();
        Ok(img)
    }
}
