//! HDR monitors: DXGI Desktop Duplication in FP16 scRGB, tone-mapped to 8-bit sRGB.
//!
//! scRGB is linear with 1.0 = 80 nits. Windows shows SDR white at the user's "SDR content
//! brightness", reported as `SDRWhiteLevel` where 1000 = 80 nits. Dividing by that level puts
//! SDR white at 1.0, so normal desktop content looks the same as on an SDR screen.

use crate::monitors::{rect, wide_to_string};
use glint_core::{Image, Rect};
use std::ffi::c_void;
use windows::core::{s, w, Interface, GUID, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_FORMAT_B8G8R8A8_UNORM,
    DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput6, IDXGIResource,
    DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

// dxgi.dll and d3d11.dll are loaded only when HDR capture runs. Importing them at startup made
// overlay tools such as RivaTuner (RTSS) treat the idle daemon as a Direct3D app and attach to it,
// which cost about 55 MB of private memory (measured 2026-11-12).

pub(crate) fn proc_addr(
    dll: PCWSTR,
    name: PCSTR,
) -> Result<unsafe extern "system" fn() -> isize, String> {
    unsafe {
        let h = LoadLibraryW(dll).map_err(|e| format!("LoadLibrary: {e}"))?;
        GetProcAddress(h, name).ok_or_else(|| "GetProcAddress failed".to_string())
    }
}

fn create_factory() -> Result<IDXGIFactory1, String> {
    type F = unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;
    let f: F = unsafe { std::mem::transmute(proc_addr(w!("dxgi.dll"), s!("CreateDXGIFactory1"))?) };
    let mut raw = std::ptr::null_mut();
    unsafe { f(&IDXGIFactory1::IID, &mut raw) }
        .ok()
        .map_err(|e| e.to_string())?;
    Ok(unsafe { IDXGIFactory1::from_raw(raw) })
}

fn create_device(
    adapter: &IDXGIAdapter,
    device: &mut Option<ID3D11Device>,
    ctx: &mut Option<ID3D11DeviceContext>,
) -> Result<(), String> {
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
    let f: F = unsafe { std::mem::transmute(proc_addr(w!("d3d11.dll"), s!("D3D11CreateDevice"))?) };
    let (mut dev, mut c) = (std::ptr::null_mut(), std::ptr::null_mut());
    unsafe {
        f(
            adapter.as_raw(),
            D3D_DRIVER_TYPE_UNKNOWN,
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
        *device = Some(ID3D11Device::from_raw(dev));
        *ctx = Some(ID3D11DeviceContext::from_raw(c));
    }
    Ok(())
}

/// An output that is in HDR mode right now.
pub struct HdrOutput {
    adapter: IDXGIAdapter1,
    output: IDXGIOutput6,
    pub rect: Rect,
    pub device: String,
}

/// Lists HDR outputs. Cheap: makes a DXGI factory, no D3D device.
pub fn hdr_outputs() -> Result<Vec<HdrOutput>, String> {
    let mut out = Vec::new();
    unsafe {
        let factory = create_factory()?;
        let mut ai = 0;
        while let Ok(adapter) = factory.EnumAdapters1(ai) {
            ai += 1;
            let mut oi = 0;
            while let Ok(o) = adapter.EnumOutputs(oi) {
                oi += 1;
                let Ok(o6) = o.cast::<IDXGIOutput6>() else {
                    continue;
                };
                let Ok(desc) = o6.GetDesc1() else { continue };
                if desc.AttachedToDesktop.as_bool()
                    && desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020
                {
                    let rotated = desc.Rotation != DXGI_MODE_ROTATION_IDENTITY
                        && desc.Rotation != DXGI_MODE_ROTATION_UNSPECIFIED;
                    if rotated {
                        continue; // rare; the GDI copy is kept
                    }
                    out.push(HdrOutput {
                        adapter: adapter.clone(),
                        output: o6,
                        rect: rect(&desc.DesktopCoordinates),
                        device: wide_to_string(&desc.DeviceName),
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Grabs one HDR output and tone-maps it to sRGB.
pub fn capture_output(o: &HdrOutput) -> Result<Image, String> {
    let white = sdr_white_level(&o.device).unwrap_or(1000);
    let t0 = std::time::Instant::now();
    let trace = |what: &str| {
        if std::env::var_os("GLINT_TRACE").is_some() {
            eprintln!("hdr {what}: {:?}", t0.elapsed());
        }
    };
    unsafe {
        let mut device: Option<ID3D11Device> = None;
        let mut ctx: Option<ID3D11DeviceContext> = None;
        create_device(
            &o.adapter
                .cast::<IDXGIAdapter>()
                .map_err(|e| e.to_string())?,
            &mut device,
            &mut ctx,
        )?;
        let (device, ctx) = (device.ok_or("no device")?, ctx.ok_or("no context")?);
        trace("device");
        let dup = o
            .output
            .DuplicateOutput1(
                &device,
                0,
                &[DXGI_FORMAT_R16G16B16A16_FLOAT, DXGI_FORMAT_B8G8R8A8_UNORM],
            )
            .map_err(|e| format!("DuplicateOutput1: {e}"))?;
        trace("duplicate");
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res: Option<IDXGIResource> = None;
        // The first frame after creating a duplication holds the current desktop image.
        let mut tries = 0;
        loop {
            dup.AcquireNextFrame(500, &mut info, &mut res)
                .map_err(|e| format!("AcquireNextFrame: {e}"))?;
            if info.LastPresentTime != 0 || tries >= 3 {
                break;
            }
            res = None;
            let _ = dup.ReleaseFrame();
            tries += 1;
        }
        trace("acquire");
        let tex: ID3D11Texture2D = res.ok_or("no frame")?.cast().map_err(|e| e.to_string())?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        tex.GetDesc(&mut desc);
        let mut sdesc = desc;
        sdesc.Usage = D3D11_USAGE_STAGING;
        sdesc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        sdesc.BindFlags = 0;
        sdesc.MiscFlags = 0;
        sdesc.MipLevels = 1;
        sdesc.ArraySize = 1;
        let mut staging: Option<ID3D11Texture2D> = None;
        device
            .CreateTexture2D(&sdesc, None, Some(&mut staging))
            .map_err(|e| e.to_string())?;
        let staging = staging.ok_or("no staging texture")?;
        ctx.CopyResource(&staging, &tex);
        let _ = dup.ReleaseFrame();
        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map))
            .map_err(|e| e.to_string())?;
        trace("map");
        let (w, h) = (desc.Width, desc.Height);
        let stride = map.RowPitch as usize;
        let src = std::slice::from_raw_parts(map.pData as *const u8, stride * h as usize);
        let img = if desc.Format == DXGI_FORMAT_R16G16B16A16_FLOAT {
            tone_map_fp16(w, h, src, stride, white)
        } else {
            Image::from_bgra(w, h, src, stride, true)
        };
        ctx.Unmap(&staging, 0);
        trace("tonemap");
        Ok(img)
    }
}

/// The SDR white level of a GDI device (`\\.\DISPLAY1`), in units where 1000 = 80 nits.
pub fn sdr_white_level(gdi_device: &str) -> Option<u32> {
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm)
            .ok()
            .ok()?;
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut np,
            paths.as_mut_ptr(),
            &mut nm,
            modes.as_mut_ptr(),
            None,
        )
        .ok()
        .ok()?;
        for p in &paths[..np as usize] {
            let mut name = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                    size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                    adapterId: p.sourceInfo.adapterId,
                    id: p.sourceInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut name.header) != 0
                || wide_to_string(&name.viewGdiDeviceName) != gdi_device
            {
                continue;
            }
            let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL,
                    size: std::mem::size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32,
                    adapterId: p.targetInfo.adapterId,
                    id: p.targetInfo.id,
                },
                ..Default::default()
            };
            if DisplayConfigGetDeviceInfo(&mut white.header) == 0 && white.SDRWhiteLevel > 0 {
                return Some(white.SDRWhiteLevel);
            }
        }
        None
    }
}

/// IEEE half to f32.
pub fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((h >> 10) & 0x1F) as i32;
    let man = (h & 0x3FF) as f32;
    match exp {
        0 => sign * man * 2f32.powi(-24),
        31 => {
            if man == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        _ => sign * (1.0 + man / 1024.0) * 2f32.powi(exp - 15),
    }
}

/// Maps a linear value (1.0 = SDR white) to 8-bit sRGB, rolling off highlights above 0.9.
pub fn tone_map_channel(x: f32) -> u8 {
    let x = if x.is_nan() { 0.0 } else { x.max(0.0) };
    const KNEE: f32 = 0.9;
    let y = if x <= KNEE {
        x
    } else {
        KNEE + (1.0 - KNEE) * (1.0 - (-(x - KNEE) / (1.0 - KNEE)).exp())
    };
    let s = if y <= 0.003_130_8 {
        12.92 * y
    } else {
        1.055 * y.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0).round().clamp(0.0, 255.0) as u8
}

/// FP16 scRGB rows to an sRGB image. `white` is the SDR white level (1000 = 80 nits).
/// Every half-float bit pattern maps to one output byte, so the work is one table lookup per channel.
pub fn tone_map_fp16(w: u32, h: u32, src: &[u8], stride: usize, white: u32) -> Image {
    let k = 1000.0 / white.max(1) as f32; // scRGB value -> multiple of SDR white
    let lut: Vec<u8> = (0..=u16::MAX)
        .map(|bits| tone_map_channel(half_to_f32(bits) * k))
        .collect();
    let mut img = Image::new(w, h);
    let row_out = w as usize * 4;
    for (y, out_row) in img.data.chunks_exact_mut(row_out).enumerate() {
        let row = &src[y * stride..y * stride + w as usize * 8];
        for (o, p) in out_row.chunks_exact_mut(4).zip(row.chunks_exact(8)) {
            o[0] = lut[u16::from_le_bytes([p[0], p[1]]) as usize];
            o[1] = lut[u16::from_le_bytes([p[2], p[3]]) as usize];
            o[2] = lut[u16::from_le_bytes([p[4], p[5]]) as usize];
            o[3] = 255;
        }
    }
    img
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves() {
        assert_eq!(half_to_f32(0x3C00), 1.0);
        assert_eq!(half_to_f32(0x4000), 2.0);
        assert_eq!(half_to_f32(0xBC00), -1.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(0), 0.0);
    }

    #[test]
    fn tone_curve() {
        assert_eq!(tone_map_channel(0.0), 0);
        assert_eq!(tone_map_channel(-1.0), 0);
        assert_eq!(tone_map_channel(0.5), 188); // sRGB of 0.5 linear
        assert!(tone_map_channel(1.0) >= 250);
        assert_eq!(tone_map_channel(100.0), 255);
        // Monotonic
        let mut last = 0;
        for i in 0..2000 {
            let v = tone_map_channel(i as f32 / 200.0);
            assert!(v >= last);
            last = v;
        }
    }

    #[test]
    fn sdr_white_maps_to_white() {
        // One pixel at scRGB 2.5 (200 nits) with an SDR white level of 2500 (200 nits).
        let two_and_half = 0x4100u16.to_le_bytes(); // 2.5
        let one = 0x3C00u16.to_le_bytes();
        let px = [two_and_half, two_and_half, two_and_half, one].concat();
        let img = tone_map_fp16(1, 1, &px, 8, 2500);
        assert!(img.pixel(0, 0)[0] >= 250, "{:?}", img.pixel(0, 0));
        // The same pixel with SDR white at 80 nits is a highlight and clips near white too.
        let img = tone_map_fp16(1, 1, &px, 8, 1000);
        assert!(img.pixel(0, 0)[0] >= 254);
    }
}
