//! The markers over the big map of the game itself.
//!
//! While the game is in front, a thread takes pictures of its window (of the screen: the
//! game is not read).  When the big map of the game is open in them, it finds which part of
//! the map is shown (`map_sync`) and follows it as the map is moved and zoomed.  What it
//! found is given to the program, which draws the markers where they are on the screen, in
//! a window of its own that lies over the game, is empty but for the markers, and lets the
//! mouse through.
//!
//! Only the surface of the world is followed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::map_sync::{self, DiskGround, Shot, View};
use crate::map_tiles::MapInfo;

/// What the thread found: the view, and where the window of the game is on the screen (x, y,
/// width, height of what is inside it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Synced {
    pub view: View,
    pub rect: [i32; 4],
    pub score: f32,
}

/// What the thread is doing, for the settings to tell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Off,
    /// The game is not in front.
    NoGame,
    /// The game is in front, its big map is not open.
    MapClosed,
    /// The big map is open, and the markers are not asked for.
    MapOpen,
    /// The big map is open and the place is being looked for (or the map is being moved).
    Looking,
    Following,
}

#[derive(Default)]
struct Shared {
    /// The game is looked at (for whether its big map is open).
    watch: AtomicBool,
    /// And the place of its map is looked for.
    align: AtomicBool,
    /// Where the player is on the full size map of the world, if they are on it.
    player: Mutex<Option<[f32; 2]>>,
    /// Where the tiles of the world are, and what the map is.
    map: Mutex<Option<(PathBuf, MapInfo)>>,
    synced: Mutex<Option<Synced>>,
    status: Mutex<Status>,
    /// Where the window of the game is on the screen, while it is in front.
    rect: Mutex<Option<[i32; 4]>>,
    /// Counts what was found, to see it change.
    version: AtomicU64,
    started: AtomicBool,
    /// The way the camera looks is wanted (read from the minimap while the big map is not
    /// open).
    want_camera: AtomicBool,
    /// The way the camera looks (as the game tells the way a character faces), and when it was
    /// read.
    camera: Mutex<Option<(f32, Instant)>>,
}

pub use window::MarksWindow;

pub struct GameMap {
    shared: Arc<Shared>,
    window: Option<window::MarksWindow>,
}

impl GameMap {
    pub fn new() -> Self {
        Self { shared: Arc::default(), window: None }
    }

    /// Call every frame: whether the game is to be looked at, whether the place of its map
    /// is to be found, where the player is, and the map of the world.
    pub fn update(&mut self, ctx: &egui::Context, watch: bool, align: bool, player: Option<[f32; 2]>, map: Option<(PathBuf, MapInfo)>) {
        self.shared.watch.store(watch || align, Ordering::Relaxed);
        self.shared.align.store(align, Ordering::Relaxed);
        if let Ok(mut place) = self.shared.player.lock() {
            *place = player;
        }
        if let Ok(mut known) = self.shared.map.lock()
            && known.as_ref().map(|(dir, info)| (dir, &info.version)) != map.as_ref().map(|(dir, info)| (dir, &info.version))
        {
            *known = map;
        }
        if (watch || align) && !self.shared.started.swap(true, Ordering::Relaxed) {
            let shared = self.shared.clone();
            let ctx = ctx.clone();
            crate::spawn_named("game-map-watch", move || watch_game(shared, ctx));
        }
    }

    pub fn synced(&self) -> Option<Synced> {
        self.shared.synced.lock().ok().and_then(|synced| *synced)
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().map(|status| *status).unwrap_or_default()
    }

    /// The big map of the game is open (and the game in front).
    pub fn map_open(&self) -> bool {
        matches!(self.status(), Status::MapOpen | Status::Looking | Status::Following)
    }

    /// Where the window of the game is on the screen, while it is in front.
    pub fn game_rect(&self) -> Option<[i32; 4]> {
        self.shared.rect.lock().ok().and_then(|rect| *rect)
    }

    pub fn version(&self) -> u64 {
        self.shared.version.load(Ordering::Relaxed)
    }

    /// Asks for the way the camera looks to be read from the minimap (or not), and says where
    /// the minimap is.
    pub fn want_camera(&self, wanted: bool) {
        self.shared.want_camera.store(wanted, Ordering::Relaxed);
    }

    /// The way the camera looks, in the way the game tells where a character faces (0 along z,
    /// 90 along x), if it was read a moment ago.
    pub fn camera_bearing(&self) -> Option<f32> {
        let camera = self.shared.camera.lock().ok()?;
        camera.filter(|(_, at)| at.elapsed() < Duration::from_millis(1500)).map(|(bearing, _)| bearing)
    }

    /// Shows a picture (see `map_render::render_marks`) over the game, at a place of the
    /// screen as big as the picture.
    pub fn show(&mut self, rect: [i32; 4], picture: &[u8]) {
        self.window.get_or_insert_with(window::MarksWindow::new).show(rect, picture);
    }

    pub fn hide(&mut self) {
        if let Some(window) = &self.window {
            window.hide();
        }
    }
}

fn set(shared: &Shared, ctx: &egui::Context, synced: Option<Synced>, status: Status) {
    let mut changed = false;
    if let Ok(mut now) = shared.synced.lock()
        && *now != synced
    {
        *now = synced;
        changed = true;
    }
    if let Ok(mut now) = shared.status.lock()
        && *now != status
    {
        *now = status;
        changed = true;
    }
    if changed {
        shared.version.fetch_add(1, Ordering::Relaxed);
        ctx.request_repaint();
    }
}

/// The thread: looks at the game while this is on.
fn watch_game(shared: Arc<Shared>, ctx: egui::Context) {
    let mut ground: Option<(String, DiskGround)> = None;
    // The view of the last picture it was found in.
    let mut last: Option<View> = None;
    // How many pictures in a row the view was the same: the markers are shown only on a
    // view that stays, not on one that is on its way.
    let mut steady = 0u32;
    // How many pictures in a row the view was not found, and the map not open.
    let mut lost = 0u32;
    let mut closed = 0u32;
    // The map was seen to open just now: it has the player in its middle.
    let mut just_opened = false;
    let mut tries = 0u32;
    let mut retried = Instant::now();
    // The small picture of the last screen the place was found on, to see whether the screen
    // changed since: if it did not, the place is the same and nothing needs to be worked out.
    let mut last_small: Option<map_sync::Gray> = None;
    let mut capturer = capture::Capturer::default();
    // The camera, read from the minimap while the big map is not open.
    let mut follower = crate::camera_dir::Follower::default();
    // How many readings in a row were made from a part of the window only: now and then all of
    // it is looked at, to see that the big map is not open.
    let mut fast_cycles = 0u32;
    // The size of the window of the game, and when the camera was last said to the program.
    let mut client: Option<(usize, usize)> = None;
    let mut told_at = Instant::now();
    let mut told: Option<f32> = None;
    // The last picture of the game, kept for when the window did not change.
    let mut kept: Option<capture::Frame> = None;
    loop {
        if !shared.watch.load(Ordering::Relaxed) {
            last = None;
            if let Ok(mut rect) = shared.rect.lock() {
                *rect = None;
            }
            set(&shared, &ctx, None, Status::Off);
            std::thread::sleep(Duration::from_millis(400));
            continue;
        }
        // The camera, quickly: a part of the window with the minimap in it, many times a second,
        // while the big map is known to be closed.
        if shared.want_camera.load(Ordering::Relaxed)
            && closed >= 4
            && fast_cycles < 12
            && let Some((_, window_height)) = client
        {
            let h = window_height as f32;
            let region = [(0.01 * h) as usize, (0.01 * h) as usize, (0.36 * h) as usize, (0.26 * h) as usize];
            match capturer.region(region) {
                Some(capture::Grab::Same) => {
                    std::thread::sleep(Duration::from_millis(4));
                    continue;
                }
                Some(capture::Grab::New(part)) => {
                    let picture = crate::camera_dir::Picture { pixels: &part.pixels, width: part.width, height: part.height, step: 4, rgb: [2, 1, 0] };
                    let reading = crate::camera_dir::read_auto(&picture, [0, 0, part.width, part.height], crate::camera_dir::MINIMAP_RADIUS * h);
                    if reading.is_some() {
                        fast_cycles += 1;
                        let bearing = follower.take(reading, Instant::now());
                        if let Ok(mut camera) = shared.camera.lock() {
                            *camera = bearing.map(|bearing| (bearing, Instant::now()));
                        }
                        // The program is told of a turn at once (it draws when told).
                        let turned = match (bearing, told) {
                            (Some(now), Some(before)) => ((now - before + 540.).rem_euclid(360.) - 180.).abs() >= 1.5,
                            (None, None) => false,
                            _ => true,
                        };
                        if turned || told_at.elapsed() >= Duration::from_millis(150) {
                            told = bearing;
                            told_at = Instant::now();
                            ctx.request_repaint();
                        }
                        // About fifty times a second: the camera turns slower than that, and
                        // every reading costs a little.
                        std::thread::sleep(Duration::from_millis(18));
                        continue;
                    }
                    // No arrow of the player there: the minimap is gone (the map was opened,
                    // a menu) and all of the window is looked at.
                    fast_cycles = u32::MAX;
                }
                None => fast_cycles = u32::MAX,
            }
        }
        fast_cycles = 0;
        let grab = capturer.game();
        let frame = match grab {
            Some(capture::Grab::New(frame)) => {
                client = Some((frame.width, frame.height));
                kept = Some(frame);
                kept.as_ref()
            }
            // Nothing changed in the window: what was worked out stays as it is.
            Some(capture::Grab::Same) if kept.is_some() && (last.is_some() || closed >= 4) => {
                std::thread::sleep(Duration::from_millis(30));
                continue;
            }
            Some(capture::Grab::Same) => kept.as_ref(),
            None => None,
        };
        let Some(frame) = frame else {
            // Another window is in front for now: the view is kept for when the game is
            // back, its map may still be open as it was.
            kept = None;
            steady = 0;
            if let Ok(mut rect) = shared.rect.lock() {
                *rect = None;
            }
            set(&shared, &ctx, None, Status::NoGame);
            std::thread::sleep(Duration::from_millis(250));
            continue;
        };
        if let Ok(mut rect) = shared.rect.lock() {
            *rect = Some(frame.rect);
        }
        if !map_sync::map_open(&frame.pixels, frame.width, frame.height, 4, [2, 1, 0]) {
            // The camera: the way the lighter wedge of the minimap points.
            let wanted = shared.want_camera.load(Ordering::Relaxed);
            if wanted {
                let height = frame.height as f32;
                let picture = crate::camera_dir::Picture { pixels: &frame.pixels, width: frame.width, height: frame.height, step: 4, rgb: [2, 1, 0] };
                // The minimap is in the top left corner, wherever exactly.
                let search = [(0.01 * height) as usize, (0.01 * height) as usize, (0.36 * height) as usize, (0.26 * height) as usize];
                let reading = crate::camera_dir::read_auto(&picture, search, crate::camera_dir::MINIMAP_RADIUS * height);
                let bearing = follower.take(reading, Instant::now());
                if let Ok(mut camera) = shared.camera.lock() {
                    *camera = bearing.map(|bearing| (bearing, Instant::now()));
                }
            }
            // One picture without the map is not the map closed (the game coming to the
            // front, something passing over it).
            closed += 1;
            steady = 0;
            if closed >= 4 {
                last = None;
                lost = 0;
                tries = 0;
                just_opened = false;
                set(&shared, &ctx, None, Status::MapClosed);
            } else {
                let status = *shared.status.lock().unwrap_or_else(|e| e.into_inner());
                set(&shared, &ctx, None, if status == Status::Following { Status::Looking } else { status });
            }
            std::thread::sleep(Duration::from_millis(if wanted { 45 } else { 100 }));
            continue;
        }
        if closed >= 4 {
            just_opened = true;
        }
        closed = 0;
        if !shared.align.load(Ordering::Relaxed) {
            last = None;
            set(&shared, &ctx, None, Status::MapOpen);
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        // The tiles of the world, as the program has them now.
        let known = shared.map.lock().ok().and_then(|map| map.clone());
        let Some((dir, info)) = known else {
            set(&shared, &ctx, None, Status::Looking);
            std::thread::sleep(Duration::from_millis(300));
            continue;
        };
        if ground.as_ref().is_none_or(|(version, _)| *version != info.version) {
            ground = Some((info.version.clone(), DiskGround::new(dir, &info)));
        }
        let Some((_, ground)) = ground.as_ref() else {
            continue;
        };
        ground.trim();
        // The tiles that were not there may have come.
        if retried.elapsed() >= Duration::from_secs(5) {
            retried = Instant::now();
            ground.retry_missing();
        }
        // Nothing moved on the screen (the map stands, the markers are shown): the place is as
        // it was, which is told without looking for it, and the next look is a little later.
        let small = Shot::small_of(&frame.pixels, frame.width, frame.height, 4, [2, 1, 0]);
        if let (Some(view), Some(before)) = (last, last_small.as_ref())
            && steady >= 2
            && map_sync::difference(before, &small) < 0.6
        {
            std::thread::sleep(Duration::from_millis(160));
            continue;
        }
        let shot = Shot::new(&frame.pixels, frame.width, frame.height, 4, [2, 1, 0]);
        let player = shared.player.lock().ok().and_then(|player| *player);
        let found = match last {
            Some(view) => map_sync::track(&shot, ground, view).or_else(|| {
                // Lost: where it was, wider.
                if lost >= 2 { map_sync::recover(&shot, ground, view) } else { None }
            }),
            None => player.and_then(|player| {
                tries += 1;
                // Just opened, it has the player in its middle; if it is not found so (it
                // was moved already, or it was open before it was looked at), anywhere
                // around the player, which takes longer.
                if just_opened && tries <= 3 {
                    map_sync::acquire(&shot, ground, player)
                } else {
                    set(&shared, &ctx, None, Status::Looking);
                    map_sync::acquire_wide(&shot, ground, player)
                }
            }),
        };
        match found {
            Some(mut found) => {
                lost = 0;
                tries = 0;
                just_opened = false;
                // Not a different view for less than a pixel: the markers would tremble.
                let mut same = false;
                if let Some(was) = last {
                    let scale = was.zoom * frame.height as f32;
                    let moved = (found.view.center[0] - was.center[0]).hypot(found.view.center[1] - was.center[1]) * scale;
                    if moved < 1.5 && (found.view.zoom / was.zoom - 1.).abs() < 0.002 {
                        found.view = was;
                        same = true;
                    }
                }
                steady = if same { steady + 1 } else { 0 };
                last = Some(found.view);
                last_small = Some(small);
                if steady >= 1 {
                    set(&shared, &ctx, Some(Synced { view: found.view, rect: frame.rect, score: found.score }), Status::Following);
                } else {
                    // On its way (the map is being moved or zoomed), or found only once.
                    set(&shared, &ctx, None, Status::Looking);
                }
            }
            None => {
                lost += 1;
                steady = 0;
                // A moment of not finding it (the map opening, something over it) is not the
                // end; after some it is looked for anew.
                if lost >= 6 {
                    last = None;
                }
                set(&shared, &ctx, None, Status::Looking);
                std::thread::sleep(Duration::from_millis(60));
            }
        }
    }
}

mod capture {
    use std::time::{Duration, Instant};

    /// A picture of what is inside the window of the game: blue, green, red and one more
    /// byte for each place, row by row from the top.
    pub struct Frame {
        pub pixels: Vec<u8>,
        pub width: usize,
        pub height: usize,
        /// Where it is on the screen.
        pub rect: [i32; 4],
    }

    /// What a look at the game gave.
    pub enum Grab {
        /// A picture.
        New(Frame),
        /// The window did not change since the last picture.
        Same,
    }

    /// How the pictures are taken: with Windows Graphics Capture where it works (the picture
    /// is made by the graphics card, of the window alone, and only copied when wanted: a few
    /// thousandths of a second), and by copying the screen with GDI where it does not (some
    /// seconds are tried again later).
    #[derive(Default)]
    pub struct Capturer {
        #[cfg(windows)]
        wgc: Option<crate::window_capture::WindowCapture>,
        /// Not before this is Windows Graphics Capture tried (again).
        #[cfg(windows)]
        wgc_after: Option<Instant>,
        /// Pictures in a row that were all black (a window that is not drawn into the
        /// capture, as some full screen modes): after some, GDI is used.
        #[cfg(windows)]
        black: u32,
        /// Which way the last picture was taken, for telling.
        pub way: &'static str,
    }

    impl Capturer {
        /// The window of the game, if the game is in front.
        pub fn game(&mut self) -> Option<Grab> {
            if crate::overlay::what_is_in_front() != crate::hotkey::FRONT_GAME {
                // Another window is in front: the capture is let go (it is made again, in a
                // moment, when the game is back).
                #[cfg(windows)]
                {
                    self.wgc = None;
                }
                return None;
            }
            #[cfg(windows)]
            {
                if let Some(grab) = self.windows_capture() {
                    return Some(grab);
                }
                self.way = "GDI";
                gdi()
            }
            #[cfg(not(windows))]
            {
                None
            }
        }

        /// Only a part of the window of the game (x, y, width, height from its top left), where
        /// the capture with Windows Graphics Capture is on: it is much less work than all of it.
        pub fn region(&mut self, region: [usize; 4]) -> Option<Grab> {
            #[cfg(windows)]
            {
                if crate::overlay::what_is_in_front() != crate::hotkey::FRONT_GAME {
                    self.wgc = None;
                    return None;
                }
                let capture = self.wgc.as_mut()?;
                match capture.grab_region(region) {
                    Ok(Some(picture)) => Some(Grab::New(Frame {
                        pixels: picture.pixels,
                        width: picture.width,
                        height: picture.height,
                        rect: [picture.origin[0], picture.origin[1], picture.width as i32, picture.height as i32],
                    })),
                    Ok(None) => Some(Grab::Same),
                    Err(e) => {
                        tracing::warn!("Windows Graphics Capture stopped: {e:#}");
                        self.wgc = None;
                        self.wgc_after = Some(Instant::now() + Duration::from_secs(20));
                        None
                    }
                }
            }
            #[cfg(not(windows))]
            {
                let _ = region;
                None
            }
        }

        #[cfg(windows)]
        fn windows_capture(&mut self) -> Option<Grab> {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

            if self.wgc_after.is_some_and(|after| Instant::now() < after) {
                return None;
            }
            let hwnd = unsafe { GetForegroundWindow() };
            if hwnd.0.is_null() {
                return None;
            }
            if self.wgc.as_ref().is_some_and(|capture| capture.hwnd() != hwnd.0 as isize) {
                self.wgc = None;
            }
            if self.wgc.is_none() {
                match crate::window_capture::WindowCapture::new(hwnd) {
                    Ok(capture) => {
                        tracing::info!("the window of the game is captured with Windows Graphics Capture");
                        self.wgc = Some(capture);
                        self.black = 0;
                    }
                    Err(e) => {
                        tracing::warn!("Windows Graphics Capture is not used: {e:#}");
                        self.wgc_after = Some(Instant::now() + Duration::from_secs(60));
                        return None;
                    }
                }
            }
            let picture = match self.wgc.as_mut()?.grab() {
                Ok(Some(picture)) => picture,
                Ok(None) => {
                    self.way = "Windows Graphics Capture";
                    return Some(Grab::Same);
                }
                Err(e) => {
                    tracing::warn!("Windows Graphics Capture stopped: {e:#}");
                    self.wgc = None;
                    self.wgc_after = Some(Instant::now() + Duration::from_secs(20));
                    return None;
                }
            };
            let (width, height) = (picture.width, picture.height);
            if width < 640 || height < 360 || width > 8192 || height > 8192 {
                return None;
            }
            // A picture of nothing but black is of a window that is not captured.
            let dark = (0..64).all(|n| {
                let at = ((n * 7919) % (width * height)) * 4;
                picture.pixels[at] < 6 && picture.pixels[at + 1] < 6 && picture.pixels[at + 2] < 6
            });
            self.black = if dark { self.black + 1 } else { 0 };
            if self.black >= 30 {
                tracing::warn!("Windows Graphics Capture gives only black: the screen is copied instead");
                self.wgc = None;
                self.wgc_after = Some(Instant::now() + Duration::from_secs(120));
                return None;
            }
            self.way = "Windows Graphics Capture";
            Some(Grab::New(Frame {
                pixels: picture.pixels,
                width,
                height,
                rect: [picture.origin[0], picture.origin[1], width as i32, height as i32],
            }))
        }
    }

    /// The window of the game by copying the screen.  What the program itself draws over the
    /// game is not in the picture, but windows of others over it are.
    #[cfg(windows)]
    fn gdi() -> Option<Grab> {
        use windows::Win32::Foundation::{POINT, RECT};
        use windows::Win32::Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
            DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY, SelectObject,
        };
        use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetForegroundWindow};

        unsafe {
            let window = GetForegroundWindow();
            let mut client = RECT::default();
            GetClientRect(window, &mut client).ok()?;
            let (width, height) = (client.right - client.left, client.bottom - client.top);
            if width < 640 || height < 360 || width > 8192 || height > 8192 {
                return None;
            }
            let mut origin = POINT { x: 0, y: 0 };
            if !ClientToScreen(window, &mut origin).as_bool() {
                return None;
            }
            let screen = GetDC(None);
            if screen.is_invalid() {
                return None;
            }
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
            let bitmap = CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0);
            let result = match bitmap {
                Ok(bitmap) if !bits.is_null() => {
                    let old = SelectObject(memory, bitmap.into());
                    // Not the windows that are laid over (ours): they are left out without
                    // the flag that takes them.
                    let copied = BitBlt(memory, 0, 0, width, height, Some(screen), origin.x, origin.y, SRCCOPY).is_ok();
                    let frame = copied.then(|| Frame {
                        pixels: std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize).to_vec(),
                        width: width as usize,
                        height: height as usize,
                        rect: [origin.x, origin.y, width, height],
                    });
                    SelectObject(memory, old);
                    let _ = DeleteObject(bitmap.into());
                    frame.map(Grab::New)
                }
                _ => None,
            };
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            result
        }
    }
}

#[cfg(windows)]
mod window {
    use std::ffi::c_void;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

    use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC, CreateDIBSection,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, SelectObject,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GWL_EXSTYLE, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, GetWindowRect,
        HTCAPTION, HWND_TOPMOST, MA_NOACTIVATE, MSG, RegisterClassExW, SW_HIDE, SW_SHOWNA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SetWindowLongPtrW, SetWindowPos, ShowWindowAsync, TranslateMessage, ULW_ALPHA, UpdateLayeredWindow, WM_MOUSEACTIVATE,
        WM_NCHITTEST, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };
    use windows::core::w;

    /// A window over the game that shows a picture, empty where the picture is, and lets the
    /// mouse through.
    pub struct MarksWindow {
        hwnd: Arc<AtomicIsize>,
        shown: AtomicBool,
        movable: AtomicBool,
    }

    unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        unsafe {
            // While it can be moved, all of it is a place to take hold of it by.
            if message == WM_NCHITTEST && GetWindowLongPtrW(hwnd, GWLP_USERDATA) == 1 {
                return LRESULT(HTCAPTION as isize);
            }
            if message == WM_MOUSEACTIVATE {
                return LRESULT(MA_NOACTIVATE as isize);
            }
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    }

    impl MarksWindow {
        pub fn new() -> Self {
            let hwnd = Arc::new(AtomicIsize::new(0));
            let made = hwnd.clone();
            crate::spawn_named("game-map-marks-window", move || unsafe {
                let instance = GetModuleHandleW(None).unwrap_or_default();
                let class = w!("AaruGameMapMarks");
                let window_class = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(procedure),
                    hInstance: instance.into(),
                    lpszClassName: class,
                    ..Default::default()
                };
                RegisterClassExW(&window_class);
                let window = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                    class,
                    w!("Aaru markers"),
                    WS_POPUP,
                    0,
                    0,
                    16,
                    16,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                );
                let Ok(window) = window else {
                    tracing::error!("unable to make the window of the markers over the game's map");
                    return;
                };
                made.store(window.0 as isize, Ordering::Relaxed);
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            });
            Self { hwnd, shown: AtomicBool::new(false), movable: AtomicBool::new(false) }
        }

        fn window(&self) -> Option<HWND> {
            let value = self.hwnd.load(Ordering::Relaxed);
            (value != 0).then(|| HWND(value as *mut c_void))
        }

        /// Lets the window be taken hold of and moved with the mouse (it takes the mouse then),
        /// or not (the mouse goes through it).
        pub fn set_movable(&self, movable: bool) {
            if self.movable.swap(movable, Ordering::Relaxed) == movable {
                return;
            }
            let Some(hwnd) = self.window() else {
                self.movable.store(!movable, Ordering::Relaxed);
                return;
            };
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, isize::from(movable));
                let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                let through = WS_EX_TRANSPARENT.0 as isize;
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, if movable { style & !through } else { style | through });
            }
        }

        /// Where the window is on the screen now.
        pub fn position(&self) -> Option<[i32; 2]> {
            let hwnd = self.window()?;
            let mut rect = windows::Win32::Foundation::RECT::default();
            unsafe { GetWindowRect(hwnd, &mut rect).ok()? };
            Some([rect.left, rect.top])
        }

        pub fn hide(&self) {
            if self.shown.swap(false, Ordering::Relaxed)
                && let Some(hwnd) = self.window()
            {
                unsafe {
                    let _ = ShowWindowAsync(hwnd, SW_HIDE);
                }
            }
        }

        /// The picture: blue, green, red and the share that is seen (the three taken by it),
        /// row by row from the top, as big as the place it is shown at (x, y, width, height).
        pub fn show(&self, rect: [i32; 4], picture: &[u8]) {
            let Some(hwnd) = self.window() else {
                return;
            };
            let (width, height) = (rect[2].max(1) as usize, rect[3].max(1) as usize);
            if picture.len() < width * height * 4 {
                return;
            }
            unsafe {
                let memory = CreateCompatibleDC(None);
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: width as i32,
                        biHeight: -(height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut bits: *mut c_void = std::ptr::null_mut();
                let Ok(bitmap) = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                    let _ = DeleteDC(memory);
                    return;
                };
                if bits.is_null() {
                    let _ = DeleteObject(bitmap.into());
                    let _ = DeleteDC(memory);
                    return;
                }
                std::ptr::copy_nonoverlapping(picture.as_ptr(), bits as *mut u8, width * height * 4);
                let previous = SelectObject(memory, bitmap.into());
                let position = POINT { x: rect[0], y: rect[1] };
                let size = SIZE { cx: width as i32, cy: height as i32 };
                let source = POINT { x: 0, y: 0 };
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                let _ = UpdateLayeredWindow(
                    hwnd,
                    None,
                    Some(&position),
                    Some(&size),
                    Some(memory),
                    Some(&source),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                );
                SelectObject(memory, previous);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(memory);
                if !self.shown.swap(true, Ordering::Relaxed) {
                    let _ = ShowWindowAsync(hwnd, SW_SHOWNA);
                    let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                }
            }
        }
    }
}

#[cfg(not(windows))]
mod window {
    pub struct MarksWindow;

    impl MarksWindow {
        pub fn new() -> Self {
            Self
        }

        pub fn hide(&self) {}

        pub fn set_movable(&self, _movable: bool) {}

        pub fn position(&self) -> Option<[i32; 2]> {
            None
        }

        pub fn show(&self, _rect: [i32; 4], _picture: &[u8]) {}
    }
}
