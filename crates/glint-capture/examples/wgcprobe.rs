//! Does Windows.Graphics.Capture see the screen BEFORE Wind zooms it? (GDI and DXGI do not.)
//! Zoom in with Wind, run this, keep still for 5 seconds. Writes wgc.png (half size) to the folder
//! given as the first argument.
use windows::core::Interface;
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

fn main() -> windows::core::Result<()> {
    glint_capture::make_dpi_aware();
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    std::fs::create_dir_all(&dir).unwrap();
    println!("Zoom in with Wind now. Capturing in 5 seconds...");
    std::thread::sleep(std::time::Duration::from_secs(5));
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
        let mut device = None;
        let mut ctx = None;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            Default::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut ctx),
        )?;
        let device = device.unwrap();
        let ctx = ctx.unwrap();
        let dxgi: IDXGIDevice = device.cast()?;
        let d3d: IDirect3DDevice = CreateDirect3D11DeviceFromDXGIDevice(&dxgi)?.cast()?;
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let item: GraphicsCaptureItem = interop.CreateForMonitor(mon)?;
        let size = item.Size()?;
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            size,
        )?;
        let session = pool.CreateCaptureSession(&item)?;
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetIsBorderRequired(false);
        let t = std::time::Instant::now();
        session.StartCapture()?;
        let frame = loop {
            if let Ok(f) = pool.TryGetNextFrame() {
                break f;
            }
            if t.elapsed().as_secs() > 3 {
                println!("no frame in 3 s");
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        println!("first frame after {} ms", t.elapsed().as_millis());
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        let tex: ID3D11Texture2D = access.GetInterface()?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        tex.GetDesc(&mut desc);
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        device.CreateTexture2D(&desc, None, Some(&mut staging))?;
        let staging = staging.unwrap();
        ctx.CopyResource(&staging, &tex);
        let mut map = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
        let (w, h) = (desc.Width, desc.Height);
        let src = std::slice::from_raw_parts(map.pData as *const u8, (map.RowPitch * h) as usize);
        let img = glint_core::Image::from_bgra(w, h, src, map.RowPitch as usize, true);
        ctx.Unmap(&staging, 0);
        let _ = session.Close();
        let _ = pool.Close();
        let t = img.thumbnail(w / 2, h / 2);
        std::fs::write(dir.join("wgc.png"), glint_core::encode::encode_png_fast(&t)).unwrap();
        println!("wrote {}", dir.join("wgc.png").display());
    }
    println!("Done. You can zoom out.");
    Ok(())
}
