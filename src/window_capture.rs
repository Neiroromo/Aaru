//! Taking a picture of the window of the game with Windows Graphics Capture.
//!
//! The older way (copying the screen with GDI, see `game_map::capture`) copies the whole
//! rectangle of the window out of the screen, through the processor, every time, and sees
//! whatever is over the window.  Windows Graphics Capture has the window's own picture made by
//! the graphics card: only the part wanted is copied to the processor, and only when asked,
//! and windows over it are not in it.  It needs Windows 10 version 1903 or newer; where it is
//! not there, or does not give a picture, the caller falls back to GDI.
//!
//! Nothing of the game is read: this is the same picture the screen shows.

#![cfg(windows)]

use anyhow::{Context, Result, anyhow};
use windows::Graphics::Capture::{Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND, POINT, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::Interface;

/// A picture of the inside (the client area) of a window: blue, green, red and one more byte
/// for each place, row by row from the top.
pub struct Picture {
    pub pixels: Vec<u8>,
    pub width: usize,
    pub height: usize,
    /// Where the inside of the window is on the screen: x, y.
    pub origin: [i32; 2],
}

/// The window a capture is of, and what it needs to give pictures of it.
pub struct WindowCapture {
    hwnd: isize,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    winrt_device: IDirect3DDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    size: SizeInt32,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    /// The same for a part of the window.
    region_staging: Option<(ID3D11Texture2D, u32, u32)>,
}

impl Drop for WindowCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// Whether this kind of capture can be used on this system.
pub fn supported() -> bool {
    GraphicsCaptureSession::IsSupported().unwrap_or(false)
}

impl WindowCapture {
    pub fn new(hwnd: HWND) -> Result<Self> {
        if !supported() {
            return Err(anyhow!("Windows Graphics Capture is not supported here"));
        }
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .context("no graphics device")?;
        }
        let device = device.context("no graphics device")?;
        let context = context.context("no graphics context")?;
        let dxgi: IDXGIDevice = device.cast()?;
        let winrt_device: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;

        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
        let size = item.Size()?;
        // Two buffers: the newest picture and the one that is being drawn.
        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, size)?;
        let session = pool.CreateCaptureSession(&item)?;
        // The yellow border and the mouse pointer are not wanted in the picture (the border
        // can only be turned off where the system allows it).
        let _ = session.SetIsBorderRequired(false);
        let _ = session.SetIsCursorCaptureEnabled(false);
        session.StartCapture()?;
        Ok(Self { hwnd: hwnd.0 as isize, device, context, winrt_device, pool, session, size, staging: None, region_staging: None })
    }

    pub fn hwnd(&self) -> isize {
        self.hwnd
    }

    /// The newest picture of the inside of the window, if there is one that was not given
    /// before (the window did not change if there is not).
    /// The newest picture the window made, if there is one that was not taken before: the
    /// frame (it is kept as long as the texture is used), the texture, and its size.
    fn newest(&mut self) -> Result<Option<(Direct3D11CaptureFrame, ID3D11Texture2D, D3D11_TEXTURE2D_DESC)>> {
        // The newest of what is there; the older ones are let go.
        let mut newest = None;
        while let Ok(frame) = self.pool.TryGetNextFrame() {
            newest = Some(frame);
        }
        let Some(frame) = newest else {
            return Ok(None);
        };
        let content = frame.ContentSize()?;
        if content.Width != self.size.Width || content.Height != self.size.Height {
            // The window was made bigger or smaller: the pictures are made at the new size
            // from the next one.
            self.size = content;
            self.pool.Recreate(&self.winrt_device, DirectXPixelFormat::B8G8R8A8UIntNormalized, 2, content)?;
            self.staging = None;
            self.region_staging = None;
        }
        let surface = frame.Surface()?;
        let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
        let texture: ID3D11Texture2D = unsafe { access.GetInterface()? };
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        Ok(Some((frame, texture, desc)))
    }

    /// A texture the processor can read, of this size, made like `like`.
    fn staging_texture(&self, like: &D3D11_TEXTURE2D_DESC, width: u32, height: u32) -> Result<ID3D11Texture2D> {
        let mut desc = *like;
        desc.Width = width;
        desc.Height = height;
        desc.Usage = D3D11_USAGE_STAGING;
        desc.BindFlags = 0;
        desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
        desc.MiscFlags = 0;
        let mut staging = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut staging))? };
        staging.context("no staging texture")
    }

    /// The newest picture of the inside of the window, if there is one that was not given
    /// before (the window did not change if there is not).
    pub fn grab(&mut self) -> Result<Option<Picture>> {
        let Some((_frame, texture, desc)) = self.newest()? else {
            return Ok(None);
        };
        let (width, height) = (desc.Width, desc.Height);

        // The inside of the window in the picture: the picture is of the window with its frame
        // (when it has one), the game wants what is inside.
        let (crop_x, crop_y, crop_w, crop_h, origin) = self.inside(width, height);

        if self.staging.as_ref().is_none_or(|(_, w, h)| *w != width || *h != height) {
            self.staging = Some((self.staging_texture(&desc, width, height)?, width, height));
        }
        let (staging, _, _) = self.staging.as_ref().context("no staging texture")?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        let mut pixels = vec![0u8; crop_w * crop_h * 4];
        unsafe {
            self.context.CopyResource(staging, &texture);
            self.context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let pitch = mapped.RowPitch as usize;
            let source = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * height as usize);
            for row in 0..crop_h {
                let from = (crop_y + row) * pitch + crop_x * 4;
                pixels[row * crop_w * 4..(row + 1) * crop_w * 4].copy_from_slice(&source[from..from + crop_w * 4]);
            }
            self.context.Unmap(staging, 0);
        }
        Ok(Some(Picture { pixels, width: crop_w, height: crop_h, origin }))
    }

    /// As `grab`, but only a part of the inside of the window (x, y, width, height from its top
    /// left): the graphics card copies just that part, so it is a tenth of the work or less.
    pub fn grab_region(&mut self, region: [usize; 4]) -> Result<Option<Picture>> {
        let Some((_frame, texture, desc)) = self.newest()? else {
            return Ok(None);
        };
        let (width, height) = (desc.Width, desc.Height);
        let (crop_x, crop_y, crop_w, crop_h, origin) = self.inside(width, height);
        let left = region[0].min(crop_w.saturating_sub(1));
        let top = region[1].min(crop_h.saturating_sub(1));
        let part_w = region[2].min(crop_w - left).max(1);
        let part_h = region[3].min(crop_h - top).max(1);
        if self.region_staging.as_ref().is_none_or(|(_, w, h)| *w != part_w as u32 || *h != part_h as u32) {
            self.region_staging = Some((self.staging_texture(&desc, part_w as u32, part_h as u32)?, part_w as u32, part_h as u32));
        }
        let (staging, _, _) = self.region_staging.as_ref().context("no staging texture")?;
        let source = D3D11_BOX {
            left: (crop_x + left) as u32,
            top: (crop_y + top) as u32,
            front: 0,
            right: (crop_x + left + part_w) as u32,
            bottom: (crop_y + top + part_h) as u32,
            back: 1,
        };
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        let mut pixels = vec![0u8; part_w * part_h * 4];
        unsafe {
            self.context.CopySubresourceRegion(staging, 0, 0, 0, 0, &texture, 0, Some(&source));
            self.context.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let pitch = mapped.RowPitch as usize;
            let bytes = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * part_h);
            for row in 0..part_h {
                pixels[row * part_w * 4..(row + 1) * part_w * 4].copy_from_slice(&bytes[row * pitch..row * pitch + part_w * 4]);
            }
            self.context.Unmap(staging, 0);
        }
        Ok(Some(Picture { pixels, width: part_w, height: part_h, origin: [origin[0] + left as i32, origin[1] + top as i32] }))
    }

    /// Where the inside of the window is in a picture of the window `width` by `height`: the
    /// part (x, y, width, height) and where its corner is on the screen.
    fn inside(&self, width: u32, height: u32) -> (usize, usize, usize, usize, [i32; 2]) {
        let hwnd = HWND(self.hwnd as *mut std::ffi::c_void);
        let mut client = RECT::default();
        let mut corner = POINT { x: 0, y: 0 };
        let mut frame = RECT::default();
        unsafe {
            if GetClientRect(hwnd, &mut client).is_err() || !ClientToScreen(hwnd, &mut corner).as_bool() {
                return (0, 0, width as usize, height as usize, [0, 0]);
            }
            if DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut frame as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32).is_err() {
                frame = RECT { left: corner.x, top: corner.y, right: corner.x + client.right, bottom: corner.y + client.bottom };
            }
        }
        // The picture begins at the corner of the frame of the window.
        let x = (corner.x - frame.left).max(0) as usize;
        let y = (corner.y - frame.top).max(0) as usize;
        let w = (client.right - client.left).max(1) as usize;
        let h = (client.bottom - client.top).max(1) as usize;
        let w = w.min((width as usize).saturating_sub(x)).max(1);
        let h = h.min((height as usize).saturating_sub(y)).max(1);
        (x.min(width as usize - 1), y.min(height as usize - 1), w, h, [corner.x, corner.y])
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC,
        ReleaseDC, SRCCOPY, SelectObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    use windows::core::{PCWSTR, w};

    use super::*;

    /// With the game running: the two ways of taking its picture, timed, and what each gives
    /// (`IRMINSUL_CAPTURE_OUT` is a folder the pictures are written to).
    #[test]
    #[ignore]
    fn the_two_ways_of_taking_the_picture_are_timed() {
        let hwnd = unsafe { FindWindowW(w!("UnityWndClass"), PCWSTR::null()) }.expect("the game is not running");
        let mut capture = WindowCapture::new(hwnd).expect("the capture can not be started");
        // Frames come as the window changes: take them for a while.
        let started = Instant::now();
        let (mut frames, mut empty, mut total) = (0u32, 0u32, Duration::ZERO);
        let mut last = None;
        while started.elapsed() < Duration::from_secs(4) {
            let one = Instant::now();
            match capture.grab().expect("a picture") {
                Some(picture) => {
                    total += one.elapsed();
                    frames += 1;
                    last = Some(picture);
                }
                None => empty += 1,
            }
            std::thread::sleep(Duration::from_millis(8));
        }
        let picture = last.expect("no picture came");
        println!(
            "WGC: {frames} pictures in 4 s (so {:.0} a second the window changes), {:?} to take one (copy and read), {empty} asks with nothing new; picture {}x{} origin {:?}",
            frames as f32 / 4.,
            total / frames.max(1),
            picture.width,
            picture.height,
            picture.origin
        );
        println!("WGC: the map is open: {}", crate::map_sync::map_open(&picture.pixels, picture.width, picture.height, 4, [2, 1, 0]));

        // The same window by GDI, from the screen.
        let (width, height, origin) = (picture.width as i32, picture.height as i32, picture.origin);
        let mut gdi_total = Duration::ZERO;
        let mut gdi_pixels = Vec::new();
        for _ in 0..20 {
            let one = Instant::now();
            unsafe {
                let screen = GetDC(None);
                let memory = CreateCompatibleDC(Some(screen));
                let mut info = BITMAPINFO::default();
                info.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                };
                let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
                let bitmap = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0).expect("bitmap");
                let old = SelectObject(memory, bitmap.into());
                BitBlt(memory, 0, 0, width, height, Some(screen), origin[0], origin[1], SRCCOPY).expect("blit");
                gdi_pixels = std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize).to_vec();
                SelectObject(memory, old);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory);
                ReleaseDC(None, screen);
            }
            gdi_total += one.elapsed();
        }
        println!("GDI: {:?} to take one", gdi_total / 20);
        println!("GDI: the map is open: {}", crate::map_sync::map_open(&gdi_pixels, width as usize, height as usize, 4, [2, 1, 0]));
        if let Some(dir) = std::env::var_os("IRMINSUL_CAPTURE_OUT") {
            let dir = std::path::PathBuf::from(dir);
            let write = |name: &str, pixels: &[u8]| {
                let mut out = image::RgbImage::new(width as u32, height as u32);
                for (at, pixel) in out.pixels_mut().enumerate() {
                    pixel.0 = [pixels[at * 4 + 2], pixels[at * 4 + 1], pixels[at * 4]];
                }
                out.save(dir.join(name)).expect("written");
            };
            write("wgc.png", &picture.pixels);
            write("gdi.png", &gdi_pixels);
        }
    }
}

#[cfg(test)]
mod camera_live {
    use std::time::{Duration, Instant};

    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    use windows::core::{PCWSTR, w};

    use super::*;

    /// With the game running and the minimap in view (the big map closed): the direction of
    /// the camera read from it for a few seconds, to be compared with where it is turned.
    #[test]
    #[ignore]
    fn the_camera_is_read_from_the_live_game() {
        let hwnd = unsafe { FindWindowW(w!("UnityWndClass"), PCWSTR::null()) }.expect("the game is not running");
        let mut capture = WindowCapture::new(hwnd).expect("the capture can not be started");
        let settings = crate::minimap::Settings::default();
        let started = Instant::now();
        let mut follower = crate::camera_dir::Follower::default();
        let (mut shown, mut sure, mut total) = (0u32, 0u32, Duration::ZERO);
        let mut last_print = Instant::now() - Duration::from_secs(1);
        while started.elapsed() < Duration::from_secs(8) {
            if let Some(picture) = capture.grab().expect("a picture") {
                let height = picture.height as f32;
                let map_open = crate::map_sync::map_open(&picture.pixels, picture.width, picture.height, 4, [2, 1, 0]);
                let one = Instant::now();
                let frame = crate::camera_dir::Picture { pixels: &picture.pixels, width: picture.width, height: picture.height, step: 4, rgb: [2, 1, 0] };
                let search = [(0.01 * height) as usize, (0.01 * height) as usize, (0.36 * height) as usize, (0.26 * height) as usize];
                let reading = crate::camera_dir::read_auto(&frame, search, crate::camera_dir::MINIMAP_RADIUS * height);
                total += one.elapsed();
                shown += 1;
                sure += u32::from(reading.is_some_and(|reading| reading.is_sure()));
                let smooth = follower.take(reading, Instant::now());
                if last_print.elapsed() > Duration::from_millis(700) {
                    last_print = Instant::now();
                    println!(
                        "big map open: {map_open}; reading {:?}; steady bearing {:?}",
                        reading.map(|r| (r.screen_degrees.round(), r.strength.round(), (r.margin * 100.).round() / 100., r.center.map(|c| c.round()))),
                        smooth.map(f32::round)
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        println!("{shown} readings, {sure} sure, {:?} to read one", total / shown.max(1));
        // The part of the picture with the minimap, to look at.
        if let (Some(dir), Some(picture)) = (std::env::var_os("IRMINSUL_CAPTURE_OUT"), capture.grab().ok().flatten().or_else(|| None)) {
            let height = picture.height as f32;
            let (cx, cy, r) = (settings.x * height, settings.y * height, settings.radius * height);
            let (left, top, side) = ((cx - r * 1.5).max(0.) as u32, (cy - r * 1.5).max(0.) as u32, (r * 3.) as u32);
            let mut out = image::RgbImage::new(side, side);
            for y in 0..side {
                for x in 0..side {
                    let at = (((top + y) as usize) * picture.width + (left + x) as usize) * 4;
                    out.put_pixel(x, y, image::Rgb([picture.pixels[at + 2], picture.pixels[at + 1], picture.pixels[at]]));
                }
            }
            out.save(std::path::PathBuf::from(dir).join("minimap_live.png")).expect("written");
        }
    }
}
