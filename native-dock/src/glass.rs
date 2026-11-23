//! Frosted wallpaper, rendered from the wallpaper FILE, never from screen pixels.
use crate::model::wide;
use image::{imageops, DynamicImage, ImageDecoder, ImageReader};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::null_mut,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::SystemTime,
};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::Registry::*, System::Threading::*,
    UI::WindowsAndMessaging::*,
};

fn vertical_offset(viewport: f32, drawn: f32, style: u32, tile: bool) -> f32 {
    if tile {
        0.
    } else if (style == 10 || style == 22) && drawn > viewport {
        // Windows Fill/Span vertically crops 1/3 from the top and 2/3 from the bottom.
        (viewport - drawn) / 3.
    } else {
        (viewport - drawn) / 2.
    }
}

fn decode_wallpaper(path: &Path) -> Option<DynamicImage> {
    let mut reader = ImageReader::open(path).ok()?.with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut source = DynamicImage::from_decoder(decoder).ok()?;
    source.apply_orientation(orientation);
    Some(source)
}

struct OwnedBitmap(HBITMAP);
impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                DeleteObject(self.0);
            }
        }
    }
}

struct WallpaperSource {
    path: PathBuf,
    small: RefCell<Option<image::RgbaImage>>,
    preview: image::RgbaImage,
    sharp: Cell<HBITMAP>,
    width: i32,
    height: i32,
    natural: [f32; 2],
    accent: u32,
}
impl Drop for WallpaperSource {
    fn drop(&mut self) {
        unsafe {
            let sharp = self.sharp.get();
            if !sharp.is_null() {
                DeleteObject(sharp);
            }
        }
    }
}
impl WallpaperSource {
    fn make_preview(small: &image::RgbaImage) -> image::RgbaImage {
        if small.width() <= 1920 && small.height() <= 1920 {
            small.clone()
        } else {
            imageops::thumbnail(small, 1920, 1920)
        }
    }
    fn release_large(&self) {
        if self.width > 1280 {
            self.small.borrow_mut().take();
        }
    }
    fn has_full_preview(&self) -> bool {
        self.preview.width() as i32 == self.width && self.preview.height() as i32 == self.height
    }
    unsafe fn sharp_bitmap(&self) -> HBITMAP {
        let existing = self.sharp.get();
        if !existing.is_null() {
            return existing;
        }
        let guard = self.small.borrow_mut();
        let temp;
        let rgba = if let Some(small) = guard.as_ref() {
            small
        } else if self.has_full_preview() {
            &self.preview
        } else if let Some((decoded, _)) = Glass::decode_small(&self.path, Glass::wallpaper_cap()) {
            temp = decoded;
            &temp
        } else {
            return null_mut();
        };
        let mut sharp_bytes = rgba.as_raw().clone();
        drop(guard);
        for p in sharp_bytes.chunks_exact_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        if let Some(bitmap) = dib(self.width, self.height, &sharp_bytes) {
            self.sharp.set(bitmap);
            bitmap
        } else {
            null_mut()
        }
    }
}

type SourceKey = (PathBuf, Option<SystemTime>);
type BlurKey = (PathBuf, Option<SystemTime>, i32, i32);
type NativeLayerKey = (PathBuf, u64, usize, [i32; 4], u32, bool, u32, (i32, i32), [u32; 2]);

struct PendingWallpaper {
    path: PathBuf,
    modified: Option<SystemTime>,
    style: u32,
    tile: bool,
    blur: f32,
    dpi: f32,
    small: image::RgbaImage,
    natural: [f32; 2],
    accent: u32,
    blurred_bytes: Vec<u8>,
    blur_w: i32,
    blur_h: i32,
}

static PENDING_WALLPAPER: Mutex<Option<PendingWallpaper>> = Mutex::new(None);
static LOADING_KEY: Mutex<Option<SourceKey>> = Mutex::new(None);

thread_local! {
    static WALLPAPER_SOURCES: RefCell<HashMap<SourceKey, Rc<WallpaperSource>>> = RefCell::new(HashMap::new());
    static BLURRED_CACHE: RefCell<HashMap<BlurKey, (Rc<OwnedBitmap>, i32, i32)>> = RefCell::new(HashMap::new());
    static PATH_BUF: RefCell<Vec<u16>> = RefCell::new(vec![0u16; 32768]);
    static NATIVE_LAYERS: RefCell<Vec<(NativeLayerKey, crate::compositor::Buffer)>> = const { RefCell::new(Vec::new()) };
}
pub fn clear_cache() {
    NATIVE_LAYERS.with(|cache| cache.borrow_mut().clear());
    WALLPAPER_SOURCES.with(|cache| cache.borrow_mut().clear());
    BLURRED_CACHE.with(|cache| cache.borrow_mut().clear());
}

pub(crate) fn trim_working_set() {
    NATIVE_LAYERS.with(|cache| cache.borrow_mut().clear());
    unsafe {
        windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            usize::MAX,
            usize::MAX,
            0,
        );
    }
}

static LIQUID_GLASS: AtomicBool = AtomicBool::new(false);
static FROSTED_GLASS: AtomicBool = AtomicBool::new(true);
static GLOBAL_BLUR_TENTHS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(130);
static LIQUID_GLASS_REVISION: AtomicU64 = AtomicU64::new(0);

pub fn set_liquid_glass(enabled: bool) {
    if LIQUID_GLASS.swap(enabled, Ordering::Relaxed) != enabled {
        LIQUID_GLASS_REVISION.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn liquid_glass() -> bool {
    LIQUID_GLASS.load(Ordering::Relaxed)
}
pub fn set_frosted_glass(enabled: bool) {
    if FROSTED_GLASS.swap(enabled, Ordering::Relaxed) != enabled {
        LIQUID_GLASS_REVISION.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn frosted_glass() -> bool {
    FROSTED_GLASS.load(Ordering::Relaxed)
}
pub fn set_global_blur(blur: f32) {
    let tenths = (blur.clamp(0.0, 80.0) * 10.0).round() as i32;
    if GLOBAL_BLUR_TENTHS.swap(tenths, Ordering::Relaxed) != tenths {
        LIQUID_GLASS_REVISION.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn global_blur() -> f32 {
    GLOBAL_BLUR_TENTHS.load(Ordering::Relaxed) as f32 / 10.0
}

pub fn move_paint_due(hwnd: HWND) -> bool {
    paint_due(hwnd, true)
}

pub fn resize_paint_due(hwnd: HWND) -> bool {
    paint_due(hwnd, false)
}

thread_local! {
    static RESIZING_WINDOWS: RefCell<std::collections::HashSet<isize>> =
        RefCell::new(std::collections::HashSet::new());
}

pub fn set_resizing(hwnd: HWND, resizing: bool) {
    RESIZING_WINDOWS.with(|set| {
        let mut set = set.borrow_mut();
        if resizing {
            set.insert(hwnd as isize);
        } else {
            set.remove(&(hwnd as isize));
        }
    });
}

pub fn is_resizing(hwnd: HWND) -> bool {
    RESIZING_WINDOWS.with(|set| set.borrow().contains(&(hwnd as isize)))
}

fn paint_due(hwnd: HWND, is_move: bool) -> bool {
    use std::collections::HashMap;
    use std::time::Instant;
    thread_local! {
        static LAST_PAINT: RefCell<HashMap<(isize, bool), Instant>> = RefCell::new(HashMap::new());
    }
    let min_ms = if crate::motion::low_power() { 16 } else { 6 };
    let now = Instant::now();
    LAST_PAINT.with(|map| {
        let mut map = map.borrow_mut();
        let key = (hwnd as isize, is_move);
        if map
            .get(&key)
            .is_some_and(|prev| now.duration_since(*prev).as_millis() < min_ms)
        {
            return false;
        }
        map.insert(key, now);
        true
    })
}

pub struct Glass {
    blend_buffer: RefCell<Option<(HWND, crate::compositor::Buffer)>>,
    liquid_src: RefCell<Vec<u8>>,
    generation: u64,
    accent: Cell<u32>,
    bitmap: Option<Rc<OwnedBitmap>>,
    source: Option<Rc<WallpaperSource>>,
    blur_width: i32,
    blur_height: i32,
    width: i32,
    height: i32,
    natural: [f32; 2],
    path: PathBuf,
    modified: Option<SystemTime>,
    style: u32,
    tile: bool,
    low_power: bool,
    blur: f32,
    dpi: f32,
}
impl Glass {
    pub fn new(blur: f32, dpi: f32) -> Self {
        Self {
            blend_buffer: RefCell::new(None),
            liquid_src: RefCell::new(Vec::new()),
            generation: 0,
            accent: Cell::new(crate::accent::FALLBACK),
            bitmap: None,
            source: None,
            blur_width: 0,
            blur_height: 0,
            width: 0,
            height: 0,
            natural: [0.0; 2],
            path: PathBuf::new(),
            modified: None,
            style: 10,
            tile: false,
            low_power: crate::motion::low_power(),
            blur,
            dpi,
        }
    }
    pub fn wallpaper_accent(&self) -> u32 {
        crate::accent::vivid(self.accent.get())
    }
    /// Dominant wallpaper accent without vibrance boosting.
    pub fn raw_wallpaper_accent(&self) -> u32 {
        self.accent.get()
    }
    pub fn available(&self) -> bool {
        self.bitmap.is_some()
    }
    pub fn dpi(&self) -> f32 {
        self.dpi
    }
    pub fn blur(&self) -> f32 {
        self.blur
    }
    pub fn revision(&self) -> u64 {
        self.generation
            .wrapping_add(LIQUID_GLASS_REVISION.load(Ordering::Relaxed))
    }
    pub fn desktop_origin_size(&self) -> [i32; 4] {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if vw > 0 && vh > 0 {
                [vx, vy, vw, vh]
            } else {
                [
                    0,
                    0,
                    GetSystemMetrics(SM_CXSCREEN).max(1),
                    GetSystemMetrics(SM_CYSCREEN).max(1),
                ]
            }
        }
    }
    pub fn monitor_origin_size(&self, hwnd: HWND) -> [i32; 4] {
        unsafe {
            if self.style == 22 {
                return self.desktop_origin_size();
            }
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            ) != 0
            {
                let r = monitor.rcMonitor;
                [r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1)]
            } else {
                self.desktop_origin_size()
            }
        }
    }
    pub fn gpu_wallpaper_pair_for(
        &self,
        hwnd: HWND,
    ) -> Option<([i32; 4], Vec<u8>, Option<Vec<u8>>)> {
        unsafe {
            let area = self.monitor_origin_size(hwnd);
            let [vx, vy, vw, vh] = area;
            if vw <= 0 || vh <= 0 || (vw as i64) * (vh as i64) > 32_000_000 {
                return None;
            }
            let blurred_bmp = self.bitmap.as_ref()?.0;
            let mut buffer = crate::compositor::Buffer::plain(vw, vh, 1.0)?;
            let render_bmp =
                |buffer: &mut crate::compositor::Buffer, bmp: HBITMAP, src_size: (i32, i32)| {
                    let rr = RECT {
                        left: 0,
                        top: 0,
                        right: vw,
                        bottom: vh,
                    };
                    if self.style == 0 || self.style == 6 {
                        PatBlt(buffer.dc, 0, 0, vw, vh, BLACKNESS);
                    } else {
                        let brush = CreateSolidBrush(self.wallpaper_accent());
                        FillRect(buffer.dc, &rr, brush);
                        DeleteObject(brush);
                    }
                    let monitor_rect = RECT {
                        left: vx,
                        top: vy,
                        right: vx + vw,
                        bottom: vy + vh,
                    };
                    self.draw_layer(buffer.dc, hwnd, vw, vh, bmp, src_size, monitor_rect, false);
                    let pixels = buffer.pixels_mut();
                    for p in pixels.chunks_exact_mut(4) {
                        p[3] = 255;
                    }
                    pixels.to_vec()
                };
            let blurred = render_bmp(&mut buffer, blurred_bmp, self.blur_size());
            let sharp = self
                .source
                .as_ref()
                .map(|s| s.sharp_bitmap())
                .filter(|b| !b.is_null())
                .map(|bmp| render_bmp(&mut buffer, bmp, (self.width, self.height)));
            Some((area, blurred, sharp))
        }
    }
    pub fn gpu_wallpaper_bgra(&self) -> Option<(i32, i32, Vec<u8>)> {
        unsafe {
            let [vx, vy, vw, vh] = self.desktop_origin_size();
            if vw <= 0 || vh <= 0 || (vw as i64) * (vh as i64) > 32_000_000 {
                return None;
            }
            let sharp = self
                .source
                .as_ref()
                .map(|s| s.sharp_bitmap())
                .filter(|b| !b.is_null());
            let (bmp, src_size) = if let Some(sharp) = sharp {
                (sharp, (self.width, self.height))
            } else if let Some(blurred) = self.bitmap.as_ref() {
                (blurred.0, self.blur_size())
            } else {
                return None;
            };
            let mut buffer = crate::compositor::Buffer::plain(vw, vh, 1.0)?;
            let rr = RECT {
                left: 0,
                top: 0,
                right: vw,
                bottom: vh,
            };
            if self.style == 0 || self.style == 6 {
                PatBlt(buffer.dc, 0, 0, vw, vh, BLACKNESS);
            } else {
                let brush = CreateSolidBrush(self.wallpaper_accent());
                FillRect(buffer.dc, &rr, brush);
                DeleteObject(brush);
            }
            let desktop_rect = RECT {
                left: vx,
                top: vy,
                right: vx + vw,
                bottom: vy + vh,
            };
            self.draw_layer(
                buffer.dc,
                GetDesktopWindow(),
                vw,
                vh,
                bmp,
                src_size,
                desktop_rect,
                false,
            );
            let pixels = buffer.pixels_mut();
            for p in pixels.chunks_exact_mut(4) {
                p[3] = 255;
            }
            Some((vw, vh, pixels.to_vec()))
        }
    }
    /// Test-only file source. Does not read or change the user's desktop setup.
    #[cfg(test)]
    pub(crate) unsafe fn fixture_file(path: &Path, blur: f32, dpi: f32) -> Self {
        let mut glass = Self::new(blur, dpi);
        glass.path = path.into();
        glass.style = 2;
        let (bitmap, blur_w, blur_h, source) = glass.load().expect("fixture wallpaper");
        glass.bitmap = Some(bitmap);
        glass.blur_width = blur_w;
        glass.blur_height = blur_h;
        glass.width = source.width;
        glass.height = source.height;
        glass.natural = source.natural;
        glass.accent.set(source.accent);
        glass.source = Some(source);
        glass.generation = 1;
        glass
    }
    #[cfg(test)]
    pub(crate) unsafe fn fixture_tiled_file(path: &Path, blur: f32, dpi: f32) -> Self {
        let mut glass = Self::fixture_file(path, blur, dpi);
        glass.tile = true;
        glass.style = 0;
        glass
    }
    pub unsafe fn set_blur(&mut self, blur: f32, dpi: f32) {
        if (self.blur - blur).abs() < 0.01 && (self.dpi - dpi).abs() < 0.01 {
            return;
        }
        self.blur = blur;
        self.dpi = dpi;
        if let Some(source) = &self.source {
            if let Some((bitmap, bw, bh)) =
                Self::get_or_build_blurred(&self.path, self.modified, source, blur, dpi)
            {
                self.bitmap = Some(bitmap);
                self.blur_width = bw;
                self.blur_height = bh;
                self.generation += 1;
                return;
            }
        }
        self.modified = None;
        self.refresh();
    }
    unsafe fn query_wallpaper() -> Option<(PathBuf, Option<SystemTime>, u32, bool)> {
        if let Some(live_path) = resolve_live_wallpaper_file() {
            let modified = std::fs::metadata(&live_path).and_then(|m| m.modified()).ok();
            return Some((live_path, modified, 10, false));
        }
        let spi_path = PATH_BUF.with(|buf| {
            let mut name = buf.borrow_mut();
            if SystemParametersInfoW(
                SPI_GETDESKWALLPAPER,
                name.len() as u32,
                name.as_mut_ptr() as _,
                0,
            ) == 0
            {
                return None;
            }
            let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
            Some(PathBuf::from(String::from_utf16_lossy(&name[..len])))
        })?;
        let path = resolve_wallpaper_path(spi_path);
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let style = registry_number("WallpaperStyle", 10);
        let tile = registry_number("TileWallpaper", 0) != 0;
        Some((path, modified, style, tile))
    }
    /// Compute the decoded wallpaper dimension cap for a desktop size and power mode.
    fn wallpaper_cap_for_screen(w: i32, h: i32, low_power: bool) -> Option<(u32, u32)> {
        if low_power {
            // Keep low-power decoding lightweight compared to 4K while staying crisp up to Full HD.
            let max_dim = if w > 0 && h > 0 {
                (w.max(h) as u32).clamp(1280, 1920)
            } else {
                1920
            };
            return Some((max_dim, max_dim));
        }
        if w > 0 && h > 0 {
            Some(((w as u32).clamp(1920, 3840), (h as u32).clamp(1080, 2160)))
        } else {
            None
        }
    }
    unsafe fn wallpaper_cap() -> Option<(u32, u32)> {
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(GetSystemMetrics(SM_CXSCREEN));
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(GetSystemMetrics(SM_CYSCREEN));
        Self::wallpaper_cap_for_screen(w, h, crate::motion::low_power())
    }
    fn decode_small(path: &Path, cap: Option<(u32, u32)>) -> Option<(image::RgbaImage, [f32; 2])> {
        let decoded = decode_wallpaper(path)?;
        let natural = [decoded.width() as f32, decoded.height() as f32];
        let small = match cap {
            Some((cw, ch)) if decoded.width() > cw || decoded.height() > ch => {
                decoded.thumbnail(cw, ch).to_rgba8()
            }
            _ => decoded.to_rgba8(),
        };
        Some((small, natural))
    }
    /// Non-blocking startup refresh: decodes and blurs the wallpaper on a background
    /// thread and posts `message` to `notify` when ready.
    pub unsafe fn refresh_async(&mut self, notify: HWND, message: u32) -> bool {
        let Some((path, modified, style, tile)) = Self::query_wallpaper() else {
            return false;
        };
        if self.bitmap.is_some()
            && path == self.path
            && modified == self.modified
            && style == self.style
            && tile == self.tile
            && self.low_power == crate::motion::low_power()
        {
            return false;
        }
        let key: SourceKey = (path.clone(), modified);
        if WALLPAPER_SOURCES.with(|c| c.borrow().contains_key(&key)) {
            return self.refresh();
        }
        {
            let mut loading = LOADING_KEY.lock().unwrap_or_else(|e| e.into_inner());
            if loading.as_ref() == Some(&key) {
                return false;
            }
            *loading = Some(key);
        }
        self.path = path.clone();
        self.modified = modified;
        self.style = style;
        self.tile = tile;
        self.low_power = crate::motion::low_power();
        let blur = self.blur;
        let dpi = self.dpi;
        let cap = Self::wallpaper_cap();
        let notify_addr = notify as usize;
        std::thread::spawn(move || {
            let loaded = (|| {
                let (small, natural) = Self::decode_small(&path, cap)?;
                let palette = imageops::thumbnail(&small, 128, 128);
                let accent = crate::accent::dominant(palette.as_raw());
                let (blurred_bytes, blur_w, blur_h) =
                    Self::prepare_blurred_bytes(&small, blur, dpi, natural[0]);
                Some(PendingWallpaper {
                    path,
                    modified,
                    style,
                    tile,
                    blur,
                    dpi,
                    small,
                    natural,
                    accent,
                    blurred_bytes,
                    blur_w,
                    blur_h,
                })
            })();
            *LOADING_KEY.lock().unwrap_or_else(|e| e.into_inner()) = None;
            if let Some(pending) = loaded {
                *PENDING_WALLPAPER.lock().unwrap_or_else(|e| e.into_inner()) = Some(pending);
                PostMessageW(notify_addr as HWND, message, 0, 0);
            }
        });
        false
    }
    pub unsafe fn finish_async(&mut self) -> bool {
        let Some(pending) = PENDING_WALLPAPER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        else {
            return false;
        };
        let source_key: SourceKey = (pending.path.clone(), pending.modified);
        let w = pending.small.width() as i32;
        let h = pending.small.height() as i32;
        let preview = WallpaperSource::make_preview(&pending.small);
        let source = Rc::new(WallpaperSource {
            path: pending.path.clone(),
            small: RefCell::new(Some(pending.small)),
            preview,
            sharp: Cell::new(null_mut()),
            width: w,
            height: h,
            natural: pending.natural,
            accent: pending.accent,
        });
        WALLPAPER_SOURCES.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= 2 {
                cache.clear();
            }
            cache.insert(source_key, source.clone());
        });
        let Some(bitmap) = dib(pending.blur_w, pending.blur_h, &pending.blurred_bytes) else {
            return false;
        };
        let owned = Rc::new(OwnedBitmap(bitmap));
        let blur_key: BlurKey = (
            pending.path.clone(),
            pending.modified,
            (pending.blur * 100.).round() as i32,
            (pending.dpi * 100.).round() as i32,
        );
        BLURRED_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= 4 {
                cache.clear();
            }
            cache.insert(blur_key, (owned.clone(), pending.blur_w, pending.blur_h));
        });
        let (final_bitmap, bw, bh) = if (self.blur - pending.blur).abs() < 0.01
            && (self.dpi - pending.dpi).abs() < 0.01
        {
            (owned, pending.blur_w, pending.blur_h)
        } else if let Some(built) = Self::get_or_build_blurred(
            &pending.path,
            pending.modified,
            &source,
            self.blur,
            self.dpi,
        ) {
            built
        } else {
            (owned, pending.blur_w, pending.blur_h)
        };
        self.path = pending.path;
        self.modified = pending.modified;
        self.style = pending.style;
        self.tile = pending.tile;
        self.low_power = crate::motion::low_power();
        self.bitmap = Some(final_bitmap);
        self.blur_width = bw;
        self.blur_height = bh;
        self.width = source.width;
        self.height = source.height;
        self.natural = source.natural;
        self.accent.set(source.accent);
        source.release_large();
        if source.width > 1280 {
            windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                usize::MAX,
                usize::MAX,
                0,
            );
        }
        self.source = Some(source);
        self.generation += 1;
        true
    }
    /// Windows writes into our owned UTF-16 buffer. We only read the returned
    /// wallpaper file and two desktop appearance values; no registry writes.
    pub unsafe fn refresh(&mut self) -> bool {
        let Some((path, modified, style, tile)) = Self::query_wallpaper() else {
            return false;
        };
        if self.bitmap.is_some()
            && path == self.path
            && modified == self.modified
            && style == self.style
            && tile == self.tile
            && self.low_power == crate::motion::low_power()
        {
            return false;
        }
        let key: SourceKey = (path.clone(), modified);
        if LOADING_KEY
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            == Some(&key)
        {
            return false;
        }
        self.path = path;
        self.generation += 1;
        self.modified = modified;
        self.style = style;
        self.tile = tile;
        self.low_power = crate::motion::low_power();
        match self.load() {
            Some((bitmap, bw, bh, source)) => {
                self.bitmap = Some(bitmap);
                self.blur_width = bw;
                self.blur_height = bh;
                self.width = source.width;
                self.height = source.height;
                self.natural = source.natural;
                self.accent.set(source.accent);
                self.source = Some(source);
                true
            }
            None => {
                self.source = None;
                self.bitmap = None;
                self.blur_width = 0;
                self.blur_height = 0;
                true
            }
        }
    }
    unsafe fn get_or_build_blurred(
        path: &Path,
        modified: Option<SystemTime>,
        source: &WallpaperSource,
        blur: f32,
        dpi: f32,
    ) -> Option<(Rc<OwnedBitmap>, i32, i32)> {
        let key: BlurKey = (
            path.to_path_buf(),
            modified,
            (blur * 100.).round() as i32,
            (dpi * 100.).round() as i32,
        );
        if let Some(hit) = BLURRED_CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
            return Some(hit);
        }
        let (bitmap, bw, bh) = if let Some(small) = source.small.borrow().as_ref() {
            Self::build_blurred(small, blur, dpi, source.natural[0])?
        } else if blur > 0.05 || source.has_full_preview() {
            Self::build_blurred(&source.preview, blur, dpi, source.natural[0])?
        } else {
            let (decoded, _) = Self::decode_small(&source.path, Self::wallpaper_cap())?;
            Self::build_blurred(&decoded, blur, dpi, source.natural[0])?
        };
        let owned = Rc::new(OwnedBitmap(bitmap));
        BLURRED_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= 4 {
                cache.clear();
            }
            cache.insert(key, (owned.clone(), bw, bh));
        });
        Some((owned, bw, bh))
    }
    fn prepare_blurred_bytes(
        small: &image::RgbaImage,
        blur: f32,
        dpi: f32,
        natural_w: f32,
    ) -> (Vec<u8>, i32, i32) {
        let sigma = (blur * dpi * small.width() as f32 / natural_w.max(1.0)).clamp(0.0, 40.0);
        let mut blurred = if sigma <= 0.05 {
            small.clone()
        } else if sigma < 3.5 {
            imageops::blur(small, sigma)
        } else {
            imageops::fast_blur(small, sigma)
        };
        let w = blurred.width();
        let h = blurred.height();
        for pixel in blurred.pixels_mut() {
            let luminance =
                0.2126 * pixel[0] as f32 + 0.7152 * pixel[1] as f32 + 0.0722 * pixel[2] as f32;
            for channel in &mut pixel.0[..3] {
                let saturated = luminance + (*channel as f32 - luminance) * 1.28;
                *channel = (saturated * 0.92).clamp(0.0, 255.0) as u8;
            }
            pixel[3] = 255;
        }
        let mut bytes = blurred.into_raw();
        for p in bytes.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        (bytes, w as i32, h as i32)
    }
    unsafe fn build_blurred(
        small: &image::RgbaImage,
        blur: f32,
        dpi: f32,
        natural_w: f32,
    ) -> Option<(HBITMAP, i32, i32)> {
        let (bytes, w, h) = Self::prepare_blurred_bytes(small, blur, dpi, natural_w);
        let bitmap = dib(w, h, &bytes)?;
        Some((bitmap, w, h))
    }
    unsafe fn load(&self) -> Option<(Rc<OwnedBitmap>, i32, i32, Rc<WallpaperSource>)> {
        let source_key: SourceKey = (self.path.clone(), self.modified);
        let source = if let Some(cached) =
            WALLPAPER_SOURCES.with(|cache| cache.borrow().get(&source_key).cloned())
        {
            cached
        } else {
            let cap = Self::wallpaper_cap();
            let (small, natural) = Self::decode_small(&self.path, cap)?;
            let palette = imageops::thumbnail(&small, 128, 128);
            let accent = crate::accent::dominant(palette.as_raw());
            if std::env::var_os("LUMEN_TRACE_FRAMES").is_some() {
                eprintln!("native-wallpaper-palette colorref={:06x}", accent);
            }
            let w = small.width() as i32;
            let h = small.height() as i32;
            let preview = WallpaperSource::make_preview(&small);
            let source = Rc::new(WallpaperSource {
                path: self.path.clone(),
                small: RefCell::new(Some(small)),
                preview,
                sharp: Cell::new(null_mut()),
                width: w,
                height: h,
                natural,
                accent,
            });
            WALLPAPER_SOURCES.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.len() >= 2 {
                    cache.clear();
                }
                cache.insert(source_key, source.clone());
            });
            source
        };
        let (bitmap, blur_w, blur_h) =
            Self::get_or_build_blurred(&self.path, self.modified, &source, self.blur, self.dpi)?;
        source.release_large();
        Some((bitmap, blur_w, blur_h, source))
    }
    pub unsafe fn draw(&self, dc: HDC, hwnd: HWND, width: i32, height: i32) {
        self.draw_blended(dc, hwnd, width, height, 255);
    }
    fn blur_size(&self) -> (i32, i32) {
        if self.blur_width > 0 && self.blur_height > 0 {
            (self.blur_width, self.blur_height)
        } else {
            (self.width, self.height)
        }
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_sharp_blur_field(
        &self,
        dc: HDC,
        hwnd: HWND,
        width: i32,
        height: i32,
        window: RECT,
        moving: bool,
        low_moving: bool,
        fallback_amount: u8,
        frost_at: impl Fn(i32, i32) -> f32,
    ) -> bool {
        let Some(bitmap) = self.bitmap.as_ref().map(|b| b.0) else {
            return false;
        };
        if let Some(sharp) = self
            .source
            .as_ref()
            .map(|s| s.sharp_bitmap())
            .filter(|b| !b.is_null())
        {
            self.draw_layer(
                dc,
                hwnd,
                width,
                height,
                sharp,
                (self.width, self.height),
                window,
                low_moving,
            );
            let mut scratch = self.blend_buffer.borrow_mut();
            if scratch
                .as_ref()
                .is_none_or(|(owner, b)| *owner != hwnd || b.width != width || b.height != height)
            {
                *scratch = crate::compositor::Buffer::plain(width, height, 1.).map(|b| (hwnd, b));
            }
            if let Some((_, buffer)) = scratch.as_mut() {
                if self.style == 0 || self.style == 6 {
                    PatBlt(buffer.dc, 0, 0, width, height, BLACKNESS);
                }
                self.draw_layer(
                    buffer.dc,
                    hwnd,
                    width,
                    height,
                    bitmap,
                    self.blur_size(),
                    window,
                    moving,
                );
                let hbmp = GetCurrentObject(dc, OBJ_BITMAP as u32);
                let mut ds: DIBSECTION = zeroed();
                let is_dib = !hbmp.is_null()
                    && GetObjectW(hbmp, size_of::<DIBSECTION>() as i32, &mut ds as *mut _ as _)
                        >= size_of::<BITMAP>() as i32
                    && !ds.dsBm.bmBits.is_null()
                    && ds.dsBm.bmBitsPixel == 32
                    && ds.dsBm.bmWidth >= width
                    && ds.dsBm.bmHeight.abs() >= height;
                if is_dib {
                    GdiFlush();
                    let dst_stride = ds.dsBm.bmWidthBytes.max(width * 4) as usize;
                    let dst_px = std::slice::from_raw_parts_mut(
                        ds.dsBm.bmBits as *mut u8,
                        height as usize * dst_stride,
                    );
                    let blur_px = &buffer.pixels_mut()[..(width as usize * height as usize * 4)];
                    for y in 0..height as usize {
                        for x in 0..width as usize {
                            let a = (frost_at(x as i32, y as i32) * 255.0)
                                .round()
                                .clamp(0.0, 255.0) as u32;
                            let di = y * dst_stride + x * 4;
                            let bi = (y * width as usize + x) * 4;
                            if a == 255 {
                                dst_px[di..di + 3].copy_from_slice(&blur_px[bi..bi + 3]);
                            } else if a > 0 {
                                let inv = 255 - a;
                                for c in 0..3 {
                                    dst_px[di + c] = ((dst_px[di + c] as u32 * inv
                                        + blur_px[bi + c] as u32 * a
                                        + 127)
                                        / 255) as u8;
                                }
                            }
                        }
                    }
                } else {
                    AlphaBlend(
                        dc,
                        0,
                        0,
                        width,
                        height,
                        buffer.dc,
                        0,
                        0,
                        width,
                        height,
                        BLENDFUNCTION {
                            BlendOp: AC_SRC_OVER as u8,
                            BlendFlags: 0,
                            SourceConstantAlpha: fallback_amount,
                            AlphaFormat: 0,
                        },
                    );
                }
            }
            if crate::motion::low_power() && !moving {
                scratch.take();
            }
            true
        } else {
            self.draw_layer(
                dc,
                hwnd,
                width,
                height,
                bitmap,
                self.blur_size(),
                window,
                moving,
            );
            true
        }
    }
    pub unsafe fn draw_with_frost(
        &self,
        dc: HDC,
        hwnd: HWND,
        width: i32,
        height: i32,
        frost_at: impl Fn(i32, i32) -> f32,
    ) {
        if self.bitmap.is_none()
            || width <= 0
            || height <= 0
            || width as i64 * height as i64 > 32_000_000
        {
            return;
        }
        let window = crate::fusion::window_rect(hwnd);
        let moving = crate::fusion::is_moving(hwnd) || is_resizing(hwnd);
        let low_moving = moving && crate::motion::low_power();
        if self.style == 0 || self.style == 6 {
            PatBlt(dc, 0, 0, width, height, BLACKNESS);
        }
        self.draw_sharp_blur_field(
            dc,
            hwnd,
            width,
            height,
            window,
            moving,
            low_moving,
            255,
            frost_at,
        );
    }
    pub unsafe fn draw_blended(&self, dc: HDC, hwnd: HWND, width: i32, height: i32, amount: u8) {
        let Some(bitmap) = self.bitmap.as_ref().map(|b| b.0) else {
            return;
        };
        if width <= 0 || height <= 0 || width as i64 * height as i64 > 32_000_000 {
            return;
        }
        if !crate::fusion::is_surface(hwnd) {
            crate::fusion::frost_strength(hwnd, amount);
        }
        let frost_blend = if !crate::fusion::is_surface(hwnd) {
            crate::fusion::shared_frost(hwnd, width, height, amount)
        } else {
            None
        };
        let window = crate::fusion::window_rect(hwnd);
        let moving = crate::fusion::is_moving(hwnd) || is_resizing(hwnd);
        let low_moving = moving && crate::motion::low_power();
        let apply_liquid = liquid_glass() && !low_moving && !crate::fusion::is_surface(hwnd);
        if self.style == 0 || self.style == 6 {
            PatBlt(dc, 0, 0, width, height, BLACKNESS);
        }
        if let Some(ref fb) = frost_blend {
            self.draw_sharp_blur_field(
                dc,
                hwnd,
                width,
                height,
                window,
                moving,
                low_moving,
                amount,
                |x, y| fb.sample_local(x, y),
            );
            self.finish_liquid_glass(
                dc,
                hwnd,
                width,
                height,
                window,
                apply_liquid,
                amount,
                Some(fb),
            );
            return;
        }
        // The caller already caches the finished widget background. A second
        // full-monitor bitmap is unnecessary.
        if amount == 255 {
            let mut scratch = self.blend_buffer.borrow_mut();
            if scratch.as_ref().is_some_and(|(owner, _)| *owner == hwnd) {
                scratch.take();
            }
            self.draw_layer(
                dc,
                hwnd,
                width,
                height,
                bitmap,
                self.blur_size(),
                window,
                moving,
            );
            self.finish_liquid_glass(dc, hwnd, width, height, window, apply_liquid, amount, None);
            return;
        }
        if let Some(source) = &self.source {
            let sharp = source.sharp_bitmap();
            if !sharp.is_null() {
                self.draw_layer(
                    dc,
                    hwnd,
                    width,
                    height,
                    sharp,
                    (self.width, self.height),
                    window,
                    low_moving,
                );
            }
        }
        if amount == 0 {
            let mut scratch = self.blend_buffer.borrow_mut();
            if scratch.as_ref().is_some_and(|(owner, _)| *owner == hwnd) {
                scratch.take();
            }
            self.finish_liquid_glass(dc, hwnd, width, height, window, apply_liquid, amount, None);
            return;
        }
        let mut scratch = self.blend_buffer.borrow_mut();
        if scratch
            .as_ref()
            .is_none_or(|(owner, b)| *owner != hwnd || b.width != width || b.height != height)
        {
            *scratch = crate::compositor::Buffer::plain(width, height, 1.).map(|b| (hwnd, b));
        }
        if let Some((_, buffer)) = scratch.as_ref() {
            PatBlt(buffer.dc, 0, 0, width, height, BLACKNESS);
            self.draw_layer(
                buffer.dc,
                hwnd,
                width,
                height,
                bitmap,
                self.blur_size(),
                window,
                moving,
            );
            AlphaBlend(
                dc,
                0,
                0,
                width,
                height,
                buffer.dc,
                0,
                0,
                width,
                height,
                BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: amount,
                    AlphaFormat: 0,
                },
            );
            self.finish_liquid_glass(dc, hwnd, width, height, window, apply_liquid, amount, None);
        }
        if crate::motion::low_power() && !moving {
            scratch.take();
        }
    }
    pub unsafe fn sample_blurred_tile(
        &self,
        hwnd: HWND,
        window: RECT,
        tw: usize,
        th: usize,
    ) -> Option<Vec<[u8; 3]>> {
        if tw == 0 || th == 0 || tw > 512 || th > 512 {
            return None;
        }
        let bmp = self.bitmap.as_ref()?.0;
        let (buf, bw, bh, stride) = self.dib_slice(bmp)?;
        let (left_f, top_f, dw, dh, _) = self.wallpaper_placement(hwnd, window);
        let placed = SharpPlacement {
            buf,
            w: bw,
            h: bh,
            stride,
            left_f,
            top_f,
            dw,
            dh,
            tile: self.tile,
            style: self.style,
        };
        let win_w = (window.right - window.left).max(1) as f32;
        let win_h = (window.bottom - window.top).max(1) as f32;
        let mut tile = Vec::with_capacity(tw * th);
        for ty in 0..th {
            let ly = (ty as f32 + 0.5) / th as f32 * win_h;
            for tx in 0..tw {
                let lx = (tx as f32 + 0.5) / tw as f32 * win_w;
                let b = sample_placed_channel(&placed, lx, ly, 0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                let g = sample_placed_channel(&placed, lx, ly, 1)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                let r = sample_placed_channel(&placed, lx, ly, 2)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                tile.push([b, g, r]);
            }
        }
        Some(tile)
    }
    unsafe fn dib_slice<'a>(&self, bmp: HBITMAP) -> Option<(&'a [u8], usize, usize, usize)> {
        if bmp.is_null() {
            return None;
        }
        let mut ds: DIBSECTION = zeroed();
        if GetObjectW(bmp as _, size_of::<DIBSECTION>() as i32, &mut ds as *mut _ as _)
            < size_of::<BITMAP>() as i32
        {
            return None;
        }
        let w = ds.dsBm.bmWidth;
        let h = ds.dsBm.bmHeight;
        if ds.dsBm.bmBits.is_null() || ds.dsBm.bmBitsPixel != 32 || w <= 0 || h <= 0 {
            return None;
        }
        let stride = ds.dsBm.bmWidthBytes.max(w * 4) as usize;
        let len = stride * (h as usize);
        Some((
            std::slice::from_raw_parts(ds.dsBm.bmBits as *const u8, len),
            w as usize,
            h as usize,
            stride,
        ))
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn finish_liquid_glass(
        &self,
        dc: HDC,
        hwnd: HWND,
        width: i32,
        height: i32,
        window: RECT,
        apply_liquid: bool,
        amount: u8,
        frost_blend: Option<&crate::fusion::FrostBlend>,
    ) {
        if apply_liquid {
            self.apply_liquid_glass_with_source(
                dc,
                hwnd,
                width,
                height,
                window,
                amount,
                frost_blend,
            );
        }
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn apply_liquid_glass_with_source(
        &self,
        dc: HDC,
        hwnd: HWND,
        width: i32,
        height: i32,
        window: RECT,
        amount: u8,
        frost_blend: Option<&crate::fusion::FrostBlend>,
    ) {
        let (left_f, top_f, dw, dh, _) = self.wallpaper_placement(hwnd, window);
        let sharp = self
            .source
            .as_ref()
            .map(|s| s.sharp_bitmap())
            .filter(|b| !b.is_null())
            .and_then(|bmp| self.dib_slice(bmp))
            .map(|(buf, w, h, stride)| SharpPlacement {
                buf,
                w,
                h,
                stride,
                left_f,
                top_f,
                dw,
                dh,
                tile: self.tile,
                style: self.style,
            });
        let blur = self
            .bitmap
            .as_ref()
            .map(|b| b.0)
            .filter(|b| !b.is_null())
            .and_then(|bmp| self.dib_slice(bmp))
            .map(|(buf, w, h, stride)| SharpPlacement {
                buf,
                w,
                h,
                stride,
                left_f,
                top_f,
                dw,
                dh,
                tile: self.tile,
                style: self.style,
            });
        let sharp_input = match sharp {
            Some(ref placed) => SharpInput::Placed(placed),
            None => SharpInput::None,
        };
        self.apply_liquid_glass_impl(
            dc,
            sharp_input,
            blur.as_ref(),
            amount,
            width,
            height,
            Some((hwnd, window)),
            frost_blend,
        );
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn apply_bridge_refraction(
        &self,
        hwnd: HWND,
        window: RECT,
        w: i32,
        h: i32,
        dpi: f32,
        pixels: &mut [u8],
        samples: &[(i32, i32, f32, f32, f32, f32, f32, f32)],
    ) {
        self.apply_bridge_refraction_with_frost(hwnd, window, w, h, dpi, pixels, samples, |_, _| {
            1.0
        });
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn apply_bridge_refraction_with_frost(
        &self,
        hwnd: HWND,
        window: RECT,
        w: i32,
        h: i32,
        dpi: f32,
        pixels: &mut [u8],
        samples: &[(i32, i32, f32, f32, f32, f32, f32, f32)],
        frost_at: impl Fn(i32, i32) -> f32,
    ) {
        if samples.is_empty() || w <= 0 || h <= 0 {
            return;
        }
        let dpi = dpi.max(1.0);
        let blur = self
            .bitmap
            .as_ref()
            .map(|b| b.0)
            .and_then(|bmp| self.dib_slice(bmp));
        let Some((blur_buf, blur_w, blur_h, blur_stride)) = blur else {
            for &(x, y, _d, _nx, _ny, a, edge, spec) in samples {
                let i = ((y * w + x) * 4) as usize;
                for c in 0..3 {
                    let v = pixels[i + c] as f32;
                    let lit = (v + (255. - v) * edge + spec).min(255.);
                    pixels[i + c] = (lit * a) as u8;
                }
            }
            return;
        };
        let (left_f, top_f, dw, dh, _) = self.wallpaper_placement(hwnd, window);
        let blur_placed = SharpPlacement {
            buf: blur_buf,
            w: blur_w,
            h: blur_h,
            stride: blur_stride,
            left_f,
            top_f,
            dw,
            dh,
            tile: self.tile,
            style: self.style,
        };
        let sharp_placed = self
            .source
            .as_ref()
            .map(|s| s.sharp_bitmap())
            .and_then(|bmp| self.dib_slice(bmp))
            .map(|(buf, sw, sh, stride)| SharpPlacement {
                buf,
                w: sw,
                h: sh,
                stride,
                left_f,
                top_f,
                dw,
                dh,
                tile: self.tile,
                style: self.style,
            });
        let rim = (40.0 * dpi).clamp(12.0, (w.min(h) as f32) * 0.40);
        let bend = 12.5 * dpi;
        let ca_strength = 4.5 * dpi;
        let has_sharp = sharp_placed.is_some();
        for &(x, y, d, nx, ny, a, edge, spec) in samples {
            let u = (-d / rim).clamp(0.0, 1.0);
            let (edge_align, ca_factor, clarity) = liquid_lens_profile(u, has_sharp, rim, bend, ca_strength);
            let disp = -bend * edge_align;
            let ca = ca_factor * ca_strength;
            let fx = x as f32 + 0.5;
            let fy = y as f32 + 0.5;

            let fb = sample_placed_disp_2d(&blur_placed, fx, fy, nx, ny, disp, ca, dpi, 0);
            let fg = sample_placed_disp_2d(&blur_placed, fx, fy, nx, ny, disp, 0.0, dpi, 1);
            let fr = sample_placed_disp_2d(&blur_placed, fx, fy, nx, ny, disp, -ca, dpi, 2);

            let local_frost = frost_at(x, y).clamp(0.0, 1.0);
            let (fb, fg, fr) = if local_frost < 0.999 {
                if let Some(ref sp) = sharp_placed {
                    let sb0 = sample_placed_disp_2d(sp, fx, fy, nx, ny, disp, ca, dpi, 0);
                    let sg0 = sample_placed_disp_2d(sp, fx, fy, nx, ny, disp, 0.0, dpi, 1);
                    let sr0 = sample_placed_disp_2d(sp, fx, fy, nx, ny, disp, -ca, dpi, 2);
                    (
                        sb0 + (fb - sb0) * local_frost,
                        sg0 + (fg - sg0) * local_frost,
                        sr0 + (fr - sr0) * local_frost,
                    )
                } else {
                    (fb, fg, fr)
                }
            } else {
                (fb, fg, fr)
            };

            let (b, g, r) = if let Some(ref sp) = sharp_placed {
                if clarity > 0.001 {
                    let ca_s = ca * 0.45;
                    let sb = sample_placed_disp_2d(sp, fx, fy, nx, ny, disp, ca_s, dpi, 0);
                    let sg = sample_placed_disp_2d(sp, fx, fy, nx, ny, disp, 0.0, dpi, 1);
                    let sr = sample_placed_disp_2d(sp, fx, fy, nx, ny, disp, -ca_s, dpi, 2);
                    (
                        fb + (sb - fb) * clarity,
                        fg + (sg - fg) * clarity,
                        fr + (sr - fr) * clarity,
                    )
                } else {
                    (fb, fg, fr)
                }
            } else {
                (fb, fg, fr)
            };

            let i = ((y * w + x) * 4) as usize;
            let refr = [b, g, r];
            let weight = inner_rim_weight(u);
            for c in 0..3 {
                let backing = pixels[i + c] as f32;
                let value = backing + (refr[c] - backing) * weight;
                let lit = (value + (255.0 - value) * edge + spec).min(255.0);
                pixels[i + c] = (lit * a).round() as u8;
            }
        }
    }
    /// Applies static iOS-style edge refraction, chromatic aberration, and a
    /// subtle specular rim only across the perimeter band of a 32-bit DIB.
    #[cfg(test)]
    unsafe fn apply_liquid_glass(&self, dc: HDC, width: i32, height: i32) {
        self.apply_liquid_glass_impl(dc, SharpInput::None, None, 255, width, height, None, None);
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn apply_liquid_glass_impl(
        &self,
        dc: HDC,
        sharp_input: SharpInput<'_>,
        blur_placed: Option<&SharpPlacement<'_>>,
        amount: u8,
        width: i32,
        height: i32,
        fusion_owner: Option<(HWND, RECT)>,
        frost_blend: Option<&crate::fusion::FrostBlend>,
    ) {
        if width < 16 || height < 16 {
            return;
        }
        let hbmp = GetCurrentObject(dc, OBJ_BITMAP as u32);
        if hbmp.is_null() {
            return;
        }
        let mut ds: DIBSECTION = zeroed();
        if GetObjectW(hbmp, size_of::<DIBSECTION>() as i32, &mut ds as *mut _ as _)
            < size_of::<BITMAP>() as i32
        {
            return;
        }
        if ds.dsBm.bmBits.is_null()
            || ds.dsBm.bmBitsPixel != 32
            || ds.dsBm.bmWidth < width
            || ds.dsBm.bmHeight < height
        {
            return;
        }
        GdiFlush();
        let w = width as usize;
        let h = height as usize;
        let stride = ds.dsBm.bmWidthBytes.max(width * 4) as usize;
        let bits = ds.dsBm.bmBits as *mut u8;
        let mut src_guard = self.liquid_src.borrow_mut();
        let needed = w * h * 4;
        if src_guard.len() < needed {
            src_guard.resize(needed, 0);
        }
        let src = &mut src_guard[..needed];
        if stride == w * 4 {
            src.copy_from_slice(std::slice::from_raw_parts(bits, needed));
        } else {
            for y in 0..h {
                std::ptr::copy_nonoverlapping(
                    bits.add(y * stride),
                    src.as_mut_ptr().add(y * w * 4),
                    w * 4,
                );
            }
        }
        let dst_buf = std::slice::from_raw_parts_mut(bits, h * stride);
        let (fusion, pane_radius) = match fusion_owner {
            Some((hwnd, window)) => (
                crate::fusion::group(hwnd)
                    .filter(|panes| panes.len() > 1)
                    .map(|panes| (panes, [window.left as f32, window.top as f32])),
                crate::fusion::pane_radius(hwnd),
            ),
            None => (None, None),
        };
        apply_liquid_glass_pixels(
            dst_buf,
            stride,
            w,
            h,
            src,
            sharp_input,
            blur_placed,
            amount,
            self.dpi,
            true,
            pane_radius,
            fusion,
            frost_blend,
        );
        if crate::motion::low_power() {
            *src_guard = Vec::new();
        }
    }
}

#[derive(Clone, Copy)]
struct SharpPlacement<'a> {
    buf: &'a [u8],
    w: usize,
    h: usize,
    stride: usize,
    left_f: f32,
    top_f: f32,
    dw: f32,
    dh: f32,
    tile: bool,
    style: u32,
}

#[derive(Clone, Copy)]
enum SharpInput<'a> {
    None,
    #[cfg_attr(not(test), allow(dead_code))]
    Buffer {
        buf: &'a [u8],
        w: usize,
        h: usize,
        pad_f: f32,
    },
    Placed(&'a SharpPlacement<'a>),
}

/// Applies static Liquid Glass edge refraction, chromatic aberration, and a
/// specular rim to an RGBA pixel buffer using a padded sharp wallpaper source.
#[cfg_attr(not(test), allow(dead_code))]
pub fn apply_liquid_glass_rgba(
    hwnd: HWND,
    dst: &mut [u8],
    w: usize,
    h: usize,
    sharp: &[u8],
    sw: usize,
    sh: usize,
    pad: usize,
    dpi: f32,
) {
    if w < 16 || h < 16 || dst.len() < w * h * 4 || sharp.len() < sw * sh * 4 {
        return;
    }
    let (fusion, pane_radius) = if !hwnd.is_null() {
        (
            crate::fusion::group(hwnd)
                .filter(|panes| panes.len() > 1)
                .map(|panes| {
                    let window = unsafe { crate::fusion::window_rect(hwnd) };
                    (panes, [window.left as f32, window.top as f32])
                }),
            crate::fusion::pane_radius(hwnd),
        )
    } else {
        (None, None)
    };
    thread_local! {
        static RGBA_SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }
    RGBA_SCRATCH.with(|scratch| {
        let mut frosted = scratch.borrow_mut();
        frosted.clear();
        frosted.extend_from_slice(&dst[..w * h * 4]);
        apply_liquid_glass_pixels(
            dst,
            w * 4,
            w,
            h,
            &frosted,
            SharpInput::Buffer {
                buf: sharp,
                w: sw,
                h: sh,
                pad_f: pad as f32,
            },
            None,
            255,
            dpi,
            false,
            pane_radius,
            fusion,
            None,
        );
        if crate::motion::low_power() && frosted.capacity() > 1024 * 1024 {
            *frosted = Vec::new();
        }
    });
}

/// Smooth rounded-rectangle SDF (`d`, `nx`, `ny`) with C¹-continuous interior
/// corner normals when `rim > radius`.
#[inline]
pub fn rounded_rect_sdf(
    fx: f32,
    fy: f32,
    hw: f32,
    hh: f32,
    rx: f32,
    ry: f32,
    radius: f32,
) -> (f32, f32, f32) {
    let qx = (fx - hw).abs() - rx;
    let qy = (fy - hh).abs() - ry;
    let (d, nx_abs, ny_abs) = if qx > 0.0 && qy > 0.0 {
        let len = (qx * qx + qy * qy).sqrt();
        let inv = 1.0 / len.max(1e-5);
        (len - radius, qx * inv, qy * inv)
    } else if qx > 0.0 {
        (qx - radius, 1.0, 0.0)
    } else if qy > 0.0 {
        (qy - radius, 0.0, 1.0)
    } else {
        let sum = (qx + qy).min(-1e-5);
        let t = (qy / sum).clamp(0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        let inv = 1.0 / (s * s + (1.0 - s) * (1.0 - s)).sqrt().max(1e-5);
        let d_smooth = s * qx + (1.0 - s) * qy - radius;
        (d_smooth, s * inv, (1.0 - s) * inv)
    };
    let k = (radius * 0.5).max(1.0);
    let r_center = (qx * qx + qy * qy).sqrt();
    let ft = (r_center / k).clamp(0.0, 1.0);
    let fade = ft * ft * (3.0 - 2.0 * ft);
    let nx = (if fx < hw { -nx_abs } else { nx_abs }) * fade;
    let ny = (if fy < hh { -ny_abs } else { ny_abs }) * fade;
    (d, nx, ny)
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn sample_disp_2d(
    buf: &[u8],
    bw: usize,
    bh: usize,
    bx: f32,
    by: f32,
    nx: f32,
    ny: f32,
    disp: f32,
    signed_ca: f32,
    dpi: f32,
    ch: usize,
) -> f32 {
    sample_refracted_channel(bx, by, nx, ny, disp, signed_ca, dpi, [1., 1.], |x, y| {
        sample_channel(buf, bw, bh, x, y, ch)
    })
}

#[inline]
fn optical_fade(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn inner_rim_weight(u: f32) -> f32 {
    // Match the actual backing at the inner edge, including any difference
    // between GDI's scaled wallpaper and the bilinear refraction sampler.
    if u <= 0.75 { 1.0 } else { optical_fade((1.0 - u) * 4.0) }
}

#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn sample_refracted_channel(
    bx: f32, by: f32, nx: f32, ny: f32, disp: f32, signed_ca: f32, dpi: f32,
    pixel_scale: [f32; 2],
    sample: impl Fn(f32, f32) -> f32,
) -> f32 {
    let px = nx * pixel_scale[0];
    let py = ny * pixel_scale[1];
    let cx = bx + px * disp;
    let cy = by + py * disp;
    if signed_ca.abs() <= 0.05 {
        sample(cx, cy)
    } else {
        // Shrink the existing five-sample footprint continuously to a point.
        // This removes the filter threshold seam without extra texture taps.
        let transition = 0.75 * dpi.max(1.0);
        let footprint = if signed_ca.abs() >= 0.05 + transition {
            1.0
        } else {
            optical_fade((signed_ca.abs() - 0.05) / transition)
        };
        let soft_ca = signed_ca * footprint;
        let d0 = disp + soft_ca * 0.35;
        let d1 = disp + soft_ca;
        let d2 = disp + soft_ca * 1.65;
        let spread = (signed_ca.abs() * 0.45).max(0.75 * dpi) * footprint;
        let tx = -ny * spread * pixel_scale[0];
        let ty = nx * spread * pixel_scale[1];
        0.20 * sample(bx + px * d0, by + py * d0)
            + 0.30 * sample(bx + px * d1, by + py * d1)
            + 0.15 * sample(bx + px * d1 + tx, by + py * d1 + ty)
            + 0.15 * sample(bx + px * d1 - tx, by + py * d1 - ty)
            + 0.20 * sample(bx + px * d2, by + py * d2)
    }
}

/// Curved convex meniscus lens profile (`u = 0` outer rim -> `u = 1` flat interior).
/// Broad meniscus with a bounded gradient and a flat inner endpoint. The colour
/// split uses the same curve in a narrower rim with its gradient preserved.
#[inline]
fn liquid_lens_profile(u: f32, has_sharp: bool, rim: f32, bend: f32, ca_strength: f32) -> (f32, f32, f32) {
    let k = (1.0 - u).clamp(0.0, 1.0);
    let edge_align = k * k * (2.0 - k);
    let colour_k = (1.0 - u / 0.65).clamp(0.0, 1.0);
    // Scale amplitude with width: confine costly colour taps to the bevel
    // without steepening their coordinate gradient or weakening the main lens.
    let ca_factor = (0.45 * 0.65) * colour_k * colour_k * (2.0 - colour_k);
    let clarity = if has_sharp && u < 0.88 {
        let s = (1.0 - u / 0.88).clamp(0.0, 1.0);
        0.52 * s * s * s * (s * (s * 6.0 - 15.0) + 10.0)
    } else {
        0.0
    };
    // This curve's maximum slope is 4/3, versus 2.65 for the old bevel.
    // Keep full bending wherever possible instead of flattening every pane.
    // Reserve the chromatic budget for the filter footprint's <=2.2 gain
    // and its farthest 1.65 tap (0.45 * 2.2 * 1.65 < 1.65). Short pills
    // still receive the same no-fold bound, for every tap rather than its mean.
    let strain = (4.0 / 3.0) * (bend.abs() + 1.65 * ca_strength.abs());
    let strength = (0.65 * rim.max(0.) / strain.max(1e-5)).min(1.);
    (edge_align * strength, ca_factor * strength, clarity)
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn sample_channel_stride(
    src: &[u8],
    w: usize,
    h: usize,
    stride: usize,
    x: f32,
    y: f32,
    ch: usize,
) -> f32 {
    let cx = x.clamp(0.0, (w - 1) as f32);
    let cy = y.clamp(0.0, (h - 1) as f32);
    let x0 = cx as usize;
    let y0 = cy as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = cx - x0 as f32;
    let fy = cy - y0 as f32;
    let r0 = y0 * stride;
    let r1 = y1 * stride;
    let p00 = src[r0 + x0 * 4 + ch] as f32;
    let p10 = src[r0 + x1 * 4 + ch] as f32;
    let p01 = src[r1 + x0 * 4 + ch] as f32;
    let p11 = src[r1 + x1 * 4 + ch] as f32;
    let top = p00 + (p10 - p00) * fx;
    let bot = p01 + (p11 - p01) * fx;
    top + (bot - top) * fy
}

#[inline]
fn sample_bgr(src: &[u8], w: usize, h: usize, x: f32, y: f32) -> [f32; 3] {
    let cx = x.clamp(0.0, (w - 1) as f32);
    let cy = y.clamp(0.0, (h - 1) as f32);
    let x0 = cx as usize;
    let y0 = cy as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = cx - x0 as f32;
    let fy = cy - y0 as f32;
    let i00 = (y0 * w + x0) * 4;
    let i10 = (y0 * w + x1) * 4;
    let i01 = (y1 * w + x0) * 4;
    let i11 = (y1 * w + x1) * 4;
    std::array::from_fn(|ch| {
        let p00 = src[i00 + ch] as f32;
        let p10 = src[i10 + ch] as f32;
        let p01 = src[i01 + ch] as f32;
        let p11 = src[i11 + ch] as f32;
        let top = p00 + (p10 - p00) * fx;
        let bot = p01 + (p11 - p01) * fx;
        top + (bot - top) * fy
    })
}

#[inline]
fn sample_placed_channel(p: &SharpPlacement<'_>, lx: f32, ly: f32, ch: usize) -> f32 {
    let u = (lx - p.left_f) / p.dw.max(1.0);
    let v = (ly - p.top_f) / p.dh.max(1.0);
    let (nu, nv) = if p.tile {
        (u.rem_euclid(1.0), v.rem_euclid(1.0))
    } else if (p.style == 0 || p.style == 6) && (u < 0.0 || u >= 1.0 || v < 0.0 || v >= 1.0) {
        return 0.0;
    } else {
        (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
    };
    sample_channel_stride(
        p.buf,
        p.w,
        p.h,
        p.stride,
        nu * p.w as f32 - 0.5,
        nv * p.h as f32 - 0.5,
        ch,
    )
}

#[inline]
#[allow(clippy::too_many_arguments)]
fn sample_placed_disp_2d(
    p: &SharpPlacement<'_>,
    bx: f32,
    by: f32,
    nx: f32,
    ny: f32,
    disp: f32,
    signed_ca: f32,
    dpi: f32,
    ch: usize,
) -> f32 {
    if !p.tile && p.style != 0 && p.style != 6 {
        // Convert the lens into texture space once, rather than dividing and
        // clamping desktop coordinates again for each chromatic filter tap.
        let scale = [p.w as f32 / p.dw.max(1.), p.h as f32 / p.dh.max(1.)];
        let x = (bx - p.left_f) * scale[0] - 0.5;
        let y = (by - p.top_f) * scale[1] - 0.5;
        return sample_refracted_channel(x, y, nx, ny, disp, signed_ca, dpi, scale, |x, y| {
            sample_channel_stride(p.buf, p.w, p.h, p.stride, x, y, ch)
        });
    }
    sample_refracted_channel(bx, by, nx, ny, disp, signed_ca, dpi, [1., 1.], |x, y| {
        sample_placed_channel(p, x, y, ch)
    })
}

/// Returns `(disp, soft_ca, spec, weight)` for a Liquid Glass card/group rim pixel.
#[inline]
#[allow(clippy::too_many_arguments)]
pub(crate) fn card_rim_lens_params(
    d: f32,
    nx: f32,
    ny: f32,
    rim: f32,
    bend: f32,
    ca_strength: f32,
    dpi: f32,
) -> (f32, f32, f32, f32) {
    let u = (-d / rim).clamp(0.0, 1.0);
    let (edge_align, ca_factor, _) = liquid_lens_profile(u, false, rim, bend, ca_strength);
    let disp = -bend * edge_align;
    let ca = ca_factor * ca_strength;
    let soft_ca = if ca.abs() <= 0.05 {
        0.0
    } else {
        let transition = 0.75 * dpi.max(1.0);
        let footprint = if ca.abs() >= 0.05 + transition {
            1.0
        } else {
            optical_fade((ca.abs() - 0.05) / transition)
        };
        ca * footprint
    };
    let spec = if u < 0.32 {
        let edge = (1.0 - u / 0.32).clamp(0.0, 1.0);
        let light_dir =
            (-nx * 0.55 - ny * 0.83).max(0.0) + (nx * 0.4 + ny * 0.6).max(0.0) * 0.35;
        edge * edge * (12.0 + 26.0 * light_dir)
    } else {
        0.0
    };
    let weight = inner_rim_weight(u);
    (disp, soft_ca, spec, weight)
}

/// Evaluates a single Liquid Glass rim pixel (`[B, G, R]`) from an opaque BGRA
/// backing buffer for sub-cards, pills, and control plates.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn refract_bgra_card_pixel(
    source: &[u8],
    sw: usize,
    sh: usize,
    x: f32,
    y: f32,
    d: f32,
    nx: f32,
    ny: f32,
    rim: f32,
    bend: f32,
    ca_strength: f32,
    dpi: f32,
) -> [u8; 3] {
    if sw == 0 || sh == 0 || source.len() < sw * sh * 4 {
        return [0; 3];
    }
    let u = (-d / rim).clamp(0.0, 1.0);
    let (edge_align, ca_factor, _) = liquid_lens_profile(u, false, rim, bend, ca_strength);
    let disp = -bend * edge_align;
    let ca = ca_factor * ca_strength;

    let [b, g, r] = if ca.abs() <= 0.05 {
        sample_bgr(source, sw, sh, x + nx * disp, y + ny * disp)
    } else {
        [
            sample_disp_2d(source, sw, sh, x, y, nx, ny, disp, ca, dpi, 0),
            sample_disp_2d(source, sw, sh, x, y, nx, ny, disp, 0.0, dpi, 1),
            sample_disp_2d(source, sw, sh, x, y, nx, ny, disp, -ca, dpi, 2),
        ]
    };

    let spec = if u < 0.32 {
        let edge = (1.0 - u / 0.32).clamp(0.0, 1.0);
        let light_dir =
            (-nx * 0.55 - ny * 0.83).max(0.0) + (nx * 0.4 + ny * 0.6).max(0.0) * 0.35;
        edge * edge * (12.0 + 26.0 * light_dir)
    } else {
        0.0
    };
    let weight = inner_rim_weight(u);
    let backing = if weight < 1.0 {
        if x == (x as i32) as f32 && y == (y as i32) as f32 {
            let base = (y.clamp(0.0, (sh - 1) as f32) as usize * sw
                + x.clamp(0.0, (sw - 1) as f32) as usize)
                * 4;
            [
                source[base] as f32,
                source[base + 1] as f32,
                source[base + 2] as f32,
            ]
        } else {
            sample_bgr(source, sw, sh, x, y)
        }
    } else {
        [0.0; 3]
    };
    std::array::from_fn(|ch| {
        let refracted = ([b, g, r][ch] + spec).min(255.0);
        let value = if weight < 1.0 {
            backing[ch] + (refracted - backing[ch]) * weight
        } else {
            refracted
        };
        value.clamp(0.0, 255.0) as u8
    })
}

/// Applies Liquid Glass edge refraction, 2D-blurred chromatic aberration, and a
/// specular rim to a premultiplied BGRA card buffer (preserving its alpha mask).
pub fn apply_liquid_glass_premul_bgra(
    buf: &mut [u8],
    w: usize,
    h: usize,
    radius: f32,
    dpi: f32,
) {
    if w < 16 || h < 16 || buf.len() < w * h * 4 {
        return;
    }
    let dpi = dpi.max(1.0);
    let min_dim = w.min(h) as f32;
    let radius = radius.min(min_dim * 0.5);
    let rim = (20.0 * dpi).clamp(8.0, min_dim * 0.32);
    let bend = 10.0 * dpi;
    let ca_strength = 3.5 * dpi;
    let hw = w as f32 * 0.5;
    let hh = h as f32 * 0.5;
    let rx = (hw - radius).max(0.0);
    let ry = (hh - radius).max(0.0);
    let rim_x = rim.ceil() as usize;
    let corner_y = radius.max(rim);
    thread_local! {
        static CARD_SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }
    CARD_SCRATCH.with(|scratch| {
        let mut src = scratch.borrow_mut();
        src.clear();
        src.extend_from_slice(&buf[..w * h * 4]);

        for y in 0..h {
            let fy = y as f32 + 0.5;
            let in_straight_band = fy >= corner_y && fy <= h as f32 - corner_y;
            let mut x = 0usize;
            while x < w {
                if in_straight_band && x == rim_x && w > rim_x * 2 {
                    x = w - rim_x;
                }
                let fx = x as f32 + 0.5;
                let (d, nx, ny) = rounded_rect_sdf(fx, fy, hw, hh, rx, ry, radius);
                if d > -rim && d <= 0.5 {
                    let i = (y * w + x) * 4;
                    let a = src[i + 3];
                    if a > 0 {
                        let u = (-d / rim).clamp(0.0, 1.0);
                        let (edge_align, ca_factor, _) = liquid_lens_profile(u, false, rim, bend, ca_strength);
                        let disp = -bend * edge_align;
                        let ca = ca_factor * ca_strength;

                        let sa_b = sample_disp_2d(&src, w, h, x as f32, y as f32, nx, ny, disp, ca, dpi, 3).max(1.0);
                        let sa_g = sample_disp_2d(&src, w, h, x as f32, y as f32, nx, ny, disp, 0.0, dpi, 3).max(1.0);
                        let sa_r = sample_disp_2d(&src, w, h, x as f32, y as f32, nx, ny, disp, -ca, dpi, 3).max(1.0);

                        let a_norm = a as f32 / 255.0;
                        let b = sample_disp_2d(&src, w, h, x as f32, y as f32, nx, ny, disp, ca, dpi, 0) * (255.0 / sa_b);
                        let g = sample_disp_2d(&src, w, h, x as f32, y as f32, nx, ny, disp, 0.0, dpi, 1) * (255.0 / sa_g);
                        let r = sample_disp_2d(&src, w, h, x as f32, y as f32, nx, ny, disp, -ca, dpi, 2) * (255.0 / sa_r);

                        let spec = if u < 0.32 {
                            let edge = (1.0 - u / 0.32).clamp(0.0, 1.0);
                            let light_dir =
                                (-nx * 0.55 - ny * 0.83).max(0.0) + (nx * 0.4 + ny * 0.6).max(0.0) * 0.35;
                            edge * edge * (12.0 + 26.0 * light_dir)
                        } else {
                            0.0
                        };

                        let weight = inner_rim_weight(u);
                        for (ch, refracted) in [b, g, r].into_iter().enumerate() {
                            let value = (refracted + spec).min(255.0) * a_norm;
                            let backing = src[i + ch] as f32;
                            buf[i + ch] = (backing + (value - backing) * weight)
                                .round().clamp(0.0, a as f32) as u8;
                        }
                    }
                }
                x += 1;
            }
        }
        if crate::motion::low_power() && src.capacity() > 1024 * 1024 {
            *src = Vec::new();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn apply_liquid_glass_pixels(
    dst_buf: &mut [u8],
    dst_stride: usize,
    w: usize,
    h: usize,
    frosted: &[u8],
    sharp_input: SharpInput<'_>,
    blur_placed: Option<&SharpPlacement<'_>>,
    amount: u8,
    dpi: f32,
    bgra: bool,
    pane_radius: Option<f32>,
    fusion: Option<(Vec<crate::fusion::geometry::Pane>, [f32; 2])>,
    frost_blend: Option<&crate::fusion::FrostBlend>,
) {
    let dpi = dpi.max(1.0);
    let min_dim = w.min(h) as f32;
    let inset = if min_dim >= 24.0 * dpi { 4.0 * dpi } else { 0.0 };
    let pane_w = (w as f32 - 2.0 * inset).max(1.0);
    let pane_h = (h as f32 - 2.0 * inset).max(1.0);
    let pane_min = pane_w.min(pane_h);
    let radius = (pane_radius.unwrap_or(32.0) * dpi).min(pane_min * 0.5);
    let rim = (40.0 * dpi).clamp(10.0, pane_min * 0.40);
    let bend = 12.5 * dpi;
    let ca_strength = 4.5 * dpi;
    let hw = pane_w * 0.5;
    let hh = pane_h * 0.5;
    let rx = (hw - radius).max(0.0);
    let ry = (hh - radius).max(0.0);
    let rim_x = (inset + rim).ceil() as usize;
    let corner_y = inset + radius.max(rim);
    let (ch_r, ch_g, ch_b) = if bgra { (2, 1, 0) } else { (0, 1, 2) };
    let has_sharp = !matches!(sharp_input, SharpInput::None);
    let sharp_placed = match sharp_input {
        SharpInput::Placed(p) => Some(p),
        _ => None,
    };
    let blur_ratio = f32::from(amount) / 255.0;

    // Workers only read wallpaper/snapshot bytes and own disjoint output rows.
    // All HDC work and GdiFlush happen on the UI thread before this CPU pass.
    let render_rows = |rows: Vec<(usize, &mut [u8])>| {
        let apply_pixel = |dst_row: &mut [u8], x: usize, y: usize, d: f32, nx: f32, ny: f32| {
            if d <= -rim || d > 0.5 {
                return;
            }
            let u = (-d / rim).clamp(0.0, 1.0);
            let (edge_align, ca_factor, clarity) = liquid_lens_profile(u, has_sharp, rim, bend, ca_strength);
            let disp = -bend * edge_align;
            let ca = ca_factor * ca_strength;

            let local_blur = frost_blend.map_or(blur_ratio, |fb| fb.sample_local(x as i32, y as i32));
            let (fb, fg, fr) = if let Some(bp) =
                blur_placed.filter(|_| local_blur >= 0.999 || sharp_placed.is_some())
            {
                let fx = x as f32 + 0.5;
                let fy = y as f32 + 0.5;
                let bb = sample_placed_disp_2d(bp, fx, fy, nx, ny, disp, ca, dpi, ch_b);
                let bg = sample_placed_disp_2d(bp, fx, fy, nx, ny, disp, 0.0, dpi, ch_g);
                let br = sample_placed_disp_2d(bp, fx, fy, nx, ny, disp, -ca, dpi, ch_r);
                if local_blur < 0.999 {
                    if let Some(sp) = sharp_placed {
                        let sb = sample_placed_disp_2d(sp, fx, fy, nx, ny, disp, ca, dpi, ch_b);
                        let sg = sample_placed_disp_2d(sp, fx, fy, nx, ny, disp, 0.0, dpi, ch_g);
                        let sr = sample_placed_disp_2d(sp, fx, fy, nx, ny, disp, -ca, dpi, ch_r);
                        (
                            sb + (bb - sb) * local_blur,
                            sg + (bg - sg) * local_blur,
                            sr + (br - sr) * local_blur,
                        )
                    } else {
                        (bb, bg, br)
                    }
                } else {
                    (bb, bg, br)
                }
            } else {
                (
                    sample_disp_2d(frosted, w, h, x as f32, y as f32, nx, ny, disp, ca, dpi, ch_b),
                    sample_disp_2d(frosted, w, h, x as f32, y as f32, nx, ny, disp, 0.0, dpi, ch_g),
                    sample_disp_2d(frosted, w, h, x as f32, y as f32, nx, ny, disp, -ca, dpi, ch_r),
                )
            };

            let (b, g, r) = if clarity > 0.001 {
                let ca_s = ca * 0.45;
                match sharp_input {
                    SharpInput::None => (fb, fg, fr),
                    SharpInput::Buffer { buf, w: sw, h: sh, pad_f } => {
                        let sx = x as f32 + pad_f;
                        let sy = y as f32 + pad_f;
                        let sb = sample_disp_2d(buf, sw, sh, sx, sy, nx, ny, disp, ca_s, dpi, ch_b);
                        let sg = sample_disp_2d(buf, sw, sh, sx, sy, nx, ny, disp, 0.0, dpi, ch_g);
                        let sr = sample_disp_2d(buf, sw, sh, sx, sy, nx, ny, disp, -ca_s, dpi, ch_r);
                        (
                            fb + (sb - fb) * clarity,
                            fg + (sg - fg) * clarity,
                            fr + (sr - fr) * clarity,
                        )
                    }
                    SharpInput::Placed(p) => {
                        let fx = x as f32 + 0.5;
                        let fy = y as f32 + 0.5;
                        let sb = sample_placed_disp_2d(p, fx, fy, nx, ny, disp, ca_s, dpi, ch_b);
                        let sg = sample_placed_disp_2d(p, fx, fy, nx, ny, disp, 0.0, dpi, ch_g);
                        let sr = sample_placed_disp_2d(p, fx, fy, nx, ny, disp, -ca_s, dpi, ch_r);
                        (
                            fb + (sb - fb) * clarity,
                            fg + (sg - fg) * clarity,
                            fr + (sr - fr) * clarity,
                        )
                    }
                }
            } else {
                (fb, fg, fr)
            };

            let i = x * 4;
            let weight = inner_rim_weight(u);
            if weight >= 1.0 {
                dst_row[i + ch_b] = b.clamp(0.0, 255.0) as u8;
                dst_row[i + ch_g] = g.clamp(0.0, 255.0) as u8;
                dst_row[i + ch_r] = r.clamp(0.0, 255.0) as u8;
            } else {
                let source_i = (y * w + x) * 4;
                for (ch, refracted) in [(ch_b, b), (ch_g, g), (ch_r, r)] {
                    let backing = frosted[source_i + ch] as f32;
                    dst_row[i + ch] = (backing + (refracted - backing) * weight)
                        .clamp(0.0, 255.0) as u8;
                }
            }
        };

        if let Some((panes, origin)) = &fusion {
            let skip_margin = rim / dpi + 1.0;
            crate::fusion::geometry::with_outline(&panes, |outline| {
                for (y, dst_row) in rows {
                    let gy = origin[1] + y as f32 + 0.5;
                    for x in 0..w {
                        let gx = origin[0] + x as f32 + 0.5;
                        if crate::fusion::geometry::covered_core_within(&panes, gx, gy, skip_margin) {
                            continue;
                        }
                        let d = outline.distance([gx, gy]);
                        if d > -rim && d <= 0.5 {
                            let dx = outline.distance([gx + 1.0, gy]) - outline.distance([gx - 1.0, gy]);
                            let dy = outline.distance([gx, gy + 1.0]) - outline.distance([gx, gy - 1.0]);
                            let len = (dx * dx + dy * dy).sqrt().max(1e-5);
                            apply_pixel(dst_row, x, y, d, dx / len, dy / len);
                        }
                    }
                }
            });
            return;
        }

        for (y, dst_row) in rows {
            let fy = y as f32 + 0.5;
            let py = fy - inset;
            let in_straight_band = fy >= corner_y && fy <= h as f32 - corner_y;

            let mut x = 0usize;
            while x < w {
                if in_straight_band && x == rim_x && w > rim_x * 2 {
                    x = w - rim_x;
                }
                let fx = x as f32 + 0.5;
                let px = fx - inset;
                let (d, nx, ny) = rounded_rect_sdf(px, py, hw, hh, rx, ry, radius);
                if d > -rim && d <= 0.5 {
                    apply_pixel(dst_row, x, y, d, nx, ny);
                }
                x += 1;
            }
        }
    };

    with_pixel_rows_parallel(dst_buf, dst_stride, 0..h, w * h, 196_608, render_rows);
}

struct PoolState {
    generation: u64,
    active_workers: usize,
    remaining: usize,
    batches_ptr: usize,
    fn_ptr: unsafe fn(*const (), *mut ()),
    ctx_ptr: usize,
    panicked: Option<Box<dyn std::any::Any + Send>>,
}

struct RowPool {
    dispatch_lock: std::sync::Mutex<()>,
    inner: std::sync::Arc<RowPoolInner>,
    worker_count: usize,
}

struct RowPoolInner {
    state: std::sync::Mutex<PoolState>,
    work_cv: std::sync::Condvar,
    done_cv: std::sync::Condvar,
}

unsafe fn noop_batch(_ctx: *const (), _batch: *mut ()) {}

unsafe fn invoke_batch<F: Fn(Vec<(usize, &mut [u8])>) + Sync>(ctx: *const (), batch: *mut ()) {
    let f = &*(ctx as *const F);
    let rows = std::mem::take(&mut *(batch as *mut Vec<(usize, &mut [u8])>));
    f(rows);
}

impl RowPool {
    fn new(max_extra_workers: usize) -> Self {
        let inner = std::sync::Arc::new(RowPoolInner {
            state: std::sync::Mutex::new(PoolState {
                generation: 0,
                active_workers: 0,
                remaining: 0,
                batches_ptr: 0,
                fn_ptr: noop_batch,
                ctx_ptr: 0,
                panicked: None,
            }),
            work_cv: std::sync::Condvar::new(),
            done_cv: std::sync::Condvar::new(),
        });
        let mut spawned = 0usize;
        for worker_idx in 0..max_extra_workers {
            let shared = std::sync::Arc::clone(&inner);
            let res = std::thread::Builder::new()
                .name(format!("lumen-glass-{worker_idx}"))
                .spawn(move || {
                    let mut seen_gen = 0u64;
                    loop {
                        let (job_gen, batch_ptr, fn_ptr, ctx_ptr) = {
                            let mut st = shared.state.lock().unwrap_or_else(|e| e.into_inner());
                            while st.generation == seen_gen || worker_idx >= st.active_workers {
                                st = shared.work_cv.wait(st).unwrap_or_else(|e| e.into_inner());
                            }
                            seen_gen = st.generation;
                            let batch_ptr = unsafe {
                                (st.batches_ptr as *mut Vec<(usize, &'static mut [u8])>)
                                    .add(worker_idx) as *mut ()
                            };
                            (st.generation, batch_ptr, st.fn_ptr, st.ctx_ptr)
                        };
                        struct DoneGuard<'a> {
                            inner: &'a RowPoolInner,
                            job_gen: u64,
                            panic_payload: Option<Box<dyn std::any::Any + Send>>,
                        }
                        impl Drop for DoneGuard<'_> {
                            fn drop(&mut self) {
                                let mut st =
                                    self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                                if st.generation == self.job_gen {
                                    if let Some(payload) = self.panic_payload.take() {
                                        if st.panicked.is_none() {
                                            st.panicked = Some(payload);
                                        }
                                    }
                                    st.remaining = st.remaining.saturating_sub(1);
                                    if st.remaining == 0 {
                                        self.inner.done_cv.notify_all();
                                    }
                                }
                            }
                        }
                        let mut guard = DoneGuard {
                            inner: &shared,
                            job_gen,
                            panic_payload: None,
                        };
                        if let Err(payload) =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
                                (fn_ptr)(ctx_ptr as *const (), batch_ptr);
                            }))
                        {
                            guard.panic_payload = Some(payload);
                        }
                    }
                });
            if res.is_ok() {
                spawned += 1;
            } else {
                break;
            }
        }
        Self {
            dispatch_lock: std::sync::Mutex::new(()),
            inner,
            worker_count: spawned,
        }
    }
}

/// CPU-only optical passes. Workers receive disjoint row slices; callers finish
/// all GDI work before entering and don't present until the scoped jobs join.
pub(crate) fn with_pixel_rows_parallel<F>(
    pixels: &mut [u8],
    stride: usize,
    rows: std::ops::Range<usize>,
    work_pixels: usize,
    threshold: usize,
    render_rows: F,
) where
    F: Fn(Vec<(usize, &mut [u8])>) + Sync,
{
    static WORKERS: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    static POOL: std::sync::OnceLock<RowPool> = std::sync::OnceLock::new();
    let max_workers = *WORKERS.get_or_init(|| {
        std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, 8))
    });
    let workers = if work_pixels < threshold || crate::motion::low_power() {
        1
    } else {
        max_workers.min(if work_pixels < 196_608 { 4 } else { 8 })
    };
    if workers <= 1 {
        let mut batch = Vec::with_capacity(rows.len());
        for (y, row) in pixels.chunks_exact_mut(stride).enumerate().take(rows.end).skip(rows.start) {
            batch.push((y, row));
        }
        render_rows(batch);
        return;
    }

    let pool = POOL.get_or_init(|| RowPool::new(max_workers - 1));
    let workers = workers.min(pool.worker_count + 1);
    if workers <= 1 {
        let mut batch = Vec::with_capacity(rows.len());
        for (y, row) in pixels.chunks_exact_mut(stride).enumerate().take(rows.end).skip(rows.start) {
            batch.push((y, row));
        }
        render_rows(batch);
        return;
    }

    let mut batches: Vec<Vec<(usize, &mut [u8])>> = (0..workers)
        .map(|_| Vec::with_capacity(rows.len().div_ceil(workers)))
        .collect();
    for (y, row) in pixels.chunks_exact_mut(stride).enumerate().take(rows.end).skip(rows.start) {
        // Interleave rows so top/bottom rims don't overload two workers while
        // other workers only get the much cheaper flat middle of the card.
        batches[y % workers].push((y, row));
    }
    let ui_rows = batches.pop().unwrap();
    let active_workers = workers - 1;

    let _dispatch = pool.dispatch_lock.lock().unwrap_or_else(|e| e.into_inner());
    {
        let mut st = pool.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        st.generation = st.generation.wrapping_add(1);
        st.active_workers = active_workers;
        st.remaining = active_workers;
        st.batches_ptr = batches.as_mut_ptr() as usize;
        st.fn_ptr = invoke_batch::<F>;
        st.ctx_ptr = (&render_rows as *const F) as usize;
        st.panicked = None;
    }
    pool.inner.work_cv.notify_all();

    struct WaitGuard<'a> {
        inner: &'a RowPoolInner,
    }
    impl Drop for WaitGuard<'_> {
        fn drop(&mut self) {
            let panicked = {
                let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                while st.remaining > 0 {
                    st = self.inner.done_cv.wait(st).unwrap_or_else(|e| e.into_inner());
                }
                st.active_workers = 0;
                st.batches_ptr = 0;
                st.fn_ptr = noop_batch;
                st.ctx_ptr = 0;
                st.panicked.take()
            };
            if let Some(payload) = panicked {
                if !std::thread::panicking() {
                    std::panic::resume_unwind(payload);
                }
            }
        }
    }
    let _wait = WaitGuard { inner: &pool.inner };
    render_rows(ui_rows);
}
impl Glass {
    unsafe fn wallpaper_placement(&self, hwnd: HWND, window: RECT) -> (f32, f32, f32, f32, f32) {
        let mut monitor: MONITORINFO = zeroed();
        monitor.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        );
        let area = if self.style == 22 {
            RECT {
                left: GetSystemMetrics(SM_XVIRTUALSCREEN),
                top: GetSystemMetrics(SM_YVIRTUALSCREEN),
                right: GetSystemMetrics(SM_XVIRTUALSCREEN) + GetSystemMetrics(SM_CXVIRTUALSCREEN),
                bottom: GetSystemMetrics(SM_YVIRTUALSCREEN) + GetSystemMetrics(SM_CYVIRTUALSCREEN),
            }
        } else {
            monitor.rcMonitor
        };
        let aw = (area.right - area.left) as f32;
        let ah = (area.bottom - area.top) as f32;
        let (dw, dh) = if self.style == 2 {
            (aw, ah)
        } else if self.style == 0 || self.tile {
            (self.natural[0], self.natural[1])
        } else {
            let ratio = if self.style == 6 {
                (aw / self.natural[0]).min(ah / self.natural[1])
            } else {
                (aw / self.natural[0]).max(ah / self.natural[1])
            };
            (self.natural[0] * ratio, self.natural[1] * ratio)
        };
        let shift = if liquid_glass() { 6.0 * self.dpi.max(1.0) } else { 0.0 };
        let left_f = area.left as f32
            + if self.tile { 0.0 } else { (aw - dw) / 2.0 }
            - window.left as f32
            + shift;
        let top_f = area.top as f32
            + vertical_offset(ah, dh, self.style, self.tile)
            - window.top as f32
            + shift;
        (left_f, top_f, dw, dh, shift)
    }
    unsafe fn draw_layer(
        &self,
        dc: HDC,
        hwnd: HWND,
        width: i32,
        height: i32,
        bitmap: HBITMAP,
        src_size: (i32, i32),
        window: RECT,
        fast: bool,
    ) {
        let area = self.monitor_origin_size(hwnd);
        let [ax, ay, aw, ah] = area;
        let bytes = aw.max(0) as usize * ah.max(0) as usize * 4;
        // Cache the shared wallpaper at monitor resolution, not a frozen
        // widget frame. Every paint crops the CURRENT desktop coordinates and
        // applies the current lens afterwards. Straddling/off-screen windows
        // retain the original placement path.
        if bitmap.is_null() || aw <= 0 || ah <= 0 || bytes > 32 * 1024 * 1024
            || window.left < ax || window.top < ay
            || window.left as i64 + width as i64 > ax as i64 + aw as i64
            || window.top as i64 + height as i64 > ay as i64 + ah as i64
        {
            self.draw_layer_uncached(dc, hwnd, width, height, bitmap, src_size, window, fast);
            return;
        }
        let key = (
            self.path.clone(), self.revision(), bitmap as usize, area, self.style,
            self.tile, self.dpi.to_bits(), src_size, self.natural.map(f32::to_bits),
        );
        NATIVE_LAYERS.with(|cache| {
            let mut cache = cache.borrow_mut();
            let index = if let Some(index) = cache.iter().position(|(k, _)| *k == key) {
                index
            } else {
                let Some(layer) = crate::compositor::Buffer::plain(aw, ah, 1.) else {
                    self.draw_layer_uncached(dc, hwnd, width, height, bitmap, src_size, window, fast);
                    return;
                };
                PatBlt(layer.dc, 0, 0, aw, ah, BLACKNESS);
                self.draw_layer_uncached(
                    layer.dc, hwnd, aw, ah, bitmap, src_size,
                    RECT { left: ax, top: ay, right: ax + aw, bottom: ay + ah }, false,
                );
                GdiFlush();
                while !cache.is_empty() && (cache.len() >= 6
                    || cache.iter().map(|(_, b)| b.width as usize * b.height as usize * 4).sum::<usize>() + bytes > 64 * 1024 * 1024)
                {
                    cache.remove(0);
                }
                cache.push((key, layer));
                cache.len() - 1
            };
            let layer = cache.remove(index);
            BitBlt(dc, 0, 0, width, height, layer.1.dc, window.left - ax, window.top - ay, SRCCOPY);
            cache.push(layer);
        });
    }

    unsafe fn draw_layer_uncached(
        &self,
        dc: HDC,
        hwnd: HWND,
        width: i32,
        height: i32,
        bitmap: HBITMAP,
        src_size: (i32, i32),
        window: RECT,
        fast: bool,
    ) {
        if bitmap.is_null() {
            return;
        }
        let (left_f, top_f, dw, dh, shift) = self.wallpaper_placement(hwnd, window);
        let left = left_f.round() as i32;
        let top = top_f.round() as i32;
        let source = CreateCompatibleDC(dc);
        let previous = SelectObject(source, bitmap);
        SetStretchBltMode(dc, if fast { COLORONCOLOR } else { HALFTONE });
        if !fast {
            SetBrushOrgEx(dc, 0, 0, null_mut());
        }
        if self.tile {
            let step_x = (dw.round() as i32).max(1);
            let step_y = (dh.round() as i32).max(1);
            for y in ((top.rem_euclid(step_y) - step_y)..height).step_by(step_y as usize) {
                for x in ((left.rem_euclid(step_x) - step_x)..width).step_by(step_x as usize) {
                    StretchBlt(
                        dc,
                        x,
                        y,
                        step_x,
                        step_y,
                        source,
                        0,
                        0,
                        src_size.0,
                        src_size.1,
                        SRCCOPY,
                    );
                }
            }
        } else {
            let right = (left_f + dw).round() as i32;
            let bottom = (top_f + dh).round() as i32;
            if shift > 0.0 && self.style != 6 && self.style != 0 {
                if left > 0 {
                    StretchBlt(
                        dc,
                        0,
                        0,
                        left.min(width),
                        height,
                        source,
                        0,
                        0,
                        1,
                        src_size.1,
                        SRCCOPY,
                    );
                }
                if top > 0 {
                    StretchBlt(
                        dc,
                        0,
                        0,
                        width,
                        top.min(height),
                        source,
                        0,
                        0,
                        src_size.0,
                        1,
                        SRCCOPY,
                    );
                }
            }
            StretchBlt(
                dc,
                left,
                top,
                (right - left).max(1),
                (bottom - top).max(1),
                source,
                0,
                0,
                src_size.0,
                src_size.1,
                SRCCOPY,
            );
        }
        SelectObject(source, previous);
        DeleteDC(source);
    }
}
unsafe fn registry_number(name: &str, default: u32) -> u32 {
    let mut buffer = [0u16; 32];
    let mut bytes = 64;
    if RegGetValueW(
        HKEY_CURRENT_USER,
        wide("Control Panel\\Desktop").as_ptr(),
        wide(name).as_ptr(),
        RRF_RT_REG_SZ,
        null_mut(),
        buffer.as_mut_ptr() as _,
        &mut bytes,
    ) != 0
    {
        return default;
    }
    let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
        .parse()
        .unwrap_or(default)
}

fn parse_transcoded_cache_path(raw: &[u8]) -> Option<PathBuf> {
    if raw.len() < 26 {
        return None;
    }
    let words: Vec<u16> = raw[24..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&w| w != 0)
        .collect();
    if words.is_empty() {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&words)))
}

unsafe fn resolve_wallpaper_path(spi_path: PathBuf) -> PathBuf {
    let mut buffer = vec![0u8; 2048];
    let mut bytes = buffer.len() as u32;
    if RegGetValueW(
        HKEY_CURRENT_USER,
        wide("Control Panel\\Desktop").as_ptr(),
        wide("TranscodedImageCache").as_ptr(),
        RRF_RT_REG_BINARY,
        null_mut(),
        buffer.as_mut_ptr() as _,
        &mut bytes,
    ) == 0
    {
        if let Some(cached_source) = parse_transcoded_cache_path(&buffer[..bytes as usize]) {
            if cached_source
                .to_string_lossy()
                .eq_ignore_ascii_case(&spi_path.to_string_lossy())
            {
                if let Some(appdata) = std::env::var_os("APPDATA") {
                    let transcoded = PathBuf::from(appdata)
                        .join("Microsoft\\Windows\\Themes\\TranscodedWallpaper");
                    if transcoded.is_file() {
                        return transcoded;
                    }
                }
            }
        }
    }
    spi_path
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "bmp"
            )
        })
        && path.is_file()
}

unsafe fn window_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
    String::from_utf16_lossy(&buf[..len])
}

unsafe fn find_live_child_pid(parent: HWND, depth: u8) -> Option<u32> {
    if parent.is_null() || depth > 3 {
        return None;
    }
    let mut child = null_mut();
    for _ in 0..64 {
        child = FindWindowExW(parent, child, std::ptr::null(), std::ptr::null());
        if child.is_null() {
            break;
        }
        if IsWindowVisible(child) == 0 {
            continue;
        }
        let class = window_class_name(child);
        if matches!(
            class.as_str(),
            "SHELLDLL_DefView" | "FolderView" | "SysListView32"
        ) {
            continue;
        }
        if class == "WorkerW" {
            if let Some(pid) = find_live_child_pid(child, depth + 1) {
                return Some(pid);
            }
            continue;
        }
        let mut rect: RECT = zeroed();
        if GetWindowRect(child, &mut rect) != 0
            && rect.right - rect.left >= 200
            && rect.bottom - rect.top >= 200
        {
            let mut pid = 0u32;
            GetWindowThreadProcessId(child, &mut pid);
            if pid != 0 {
                return Some(pid);
            }
        }
    }
    None
}

unsafe fn find_live_wallpaper_pid() -> Option<u32> {
    let progman = FindWindowW(wide("Progman").as_ptr(), std::ptr::null());
    if let Some(pid) = find_live_child_pid(progman, 0) {
        return Some(pid);
    }
    let worker_class = wide("WorkerW");
    let mut worker = null_mut();
    for _ in 0..64 {
        worker = FindWindowExW(null_mut(), worker, worker_class.as_ptr(), std::ptr::null());
        if worker.is_null() {
            break;
        }
        if IsWindowVisible(worker) == 0 {
            continue;
        }
        if let Some(pid) = find_live_child_pid(worker, 0) {
            return Some(pid);
        }
    }
    None
}

unsafe fn process_image_path(pid: u32) -> Option<PathBuf> {
    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if handle.is_null() {
        return None;
    }
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) != 0;
    CloseHandle(handle);
    if !ok || len == 0 {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

fn resolve_project_json_image(project_path: &Path) -> Option<PathBuf> {
    let dir = project_path.parent()?;
    let raw = std::fs::read_to_string(project_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    for key in ["file", "preview", "Thumbnail", "Preview", "FileName"] {
        if let Some(rel) = json.get(key).and_then(|v| v.as_str()) {
            let candidate = Path::new(rel);
            let full = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                dir.join(candidate)
            };
            if is_supported_image(&full) {
                return Some(full);
            }
        }
    }
    for fallback in ["preview.jpg", "preview.png", "preview.bmp"] {
        let full = dir.join(fallback);
        if full.is_file() {
            return Some(full);
        }
    }
    None
}

fn resolve_wallpaper_engine_image(exe_dir: &Path) -> Option<PathBuf> {
    let config_path = exe_dir.join("config.json");
    let raw = std::fs::read_to_string(config_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let root = json.as_object()?;
    for profile in root.values() {
        let Some(selected) = profile
            .get("general")
            .and_then(|g| g.get("wallpaperconfig"))
            .and_then(|w| w.get("selectedwallpapers"))
            .or_else(|| profile.get("selectedwallpapers"))
            .and_then(|s| s.as_object())
        else {
            continue;
        };
        let entry = selected
            .get("Monitor0")
            .or_else(|| selected.values().next())?;
        let Some(file_str) = entry.get("file").and_then(|v| v.as_str()) else {
            continue;
        };
        let file_path = PathBuf::from(file_str);
        if is_supported_image(&file_path) {
            return Some(file_path);
        }
        if file_path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("project.json"))
        {
            if let Some(img) = resolve_project_json_image(&file_path) {
                return Some(img);
            }
        }
    }
    None
}

fn resolve_lively_image() -> Option<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    let layout_path =
        PathBuf::from(local_app_data).join("Lively Wallpaper\\SaveData\\wpdata\\WallpaperLayout.json");
    let raw = std::fs::read_to_string(layout_path).ok()?;
    let layouts: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let first = layouts.as_array()?.first()?;
    if let Some(info_dir) = first.get("LivelyInfoPath").and_then(|v| v.as_str()) {
        let info_path = Path::new(info_dir).join("LivelyInfo.json");
        if let Some(img) = resolve_project_json_image(&info_path) {
            return Some(img);
        }
    }
    if let Some(file_str) = first.get("FilePath").and_then(|v| v.as_str()) {
        let path = PathBuf::from(file_str);
        if is_supported_image(&path) {
            return Some(path);
        }
    }
    None
}

unsafe fn resolve_live_wallpaper_file() -> Option<PathBuf> {
    use std::time::{Duration, Instant};
    thread_local! {
        static LIVE_WP_CACHE: RefCell<Option<(Instant, Option<PathBuf>)>> = const { RefCell::new(None) };
    }
    let ttl = if crate::motion::low_power() {
        Duration::from_secs(10)
    } else {
        Duration::from_secs(5)
    };
    if let Some(cached) = LIVE_WP_CACHE.with(|c| {
        c.borrow()
            .as_ref()
            .filter(|(t, _)| t.elapsed() < ttl)
            .map(|(_, p)| p.clone())
    }) {
        return cached;
    }
    let resolved = (|| {
        let pid = find_live_wallpaper_pid()?;
        let exe_path = process_image_path(pid)?;
        let exe_name = exe_path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if exe_name.contains("wallpaper32") || exe_name.contains("wallpaper64") {
            if let Some(dir) = exe_path.parent() {
                if let Some(img) = resolve_wallpaper_engine_image(dir) {
                    return Some(img);
                }
            }
        }
        if exe_name.contains("lively") || exe_name.contains("cefsharp") {
            if let Some(img) = resolve_lively_image() {
                return Some(img);
            }
        }
        if let Some(dir) = exe_path.parent() {
            if let Some(img) = resolve_wallpaper_engine_image(dir) {
                return Some(img);
            }
        }
        None
    })();
    LIVE_WP_CACHE.with(|c| *c.borrow_mut() = Some((Instant::now(), resolved.clone())));
    resolved
}
#[inline]
fn sample_channel(src: &[u8], w: usize, h: usize, x: f32, y: f32, ch: usize) -> f32 {
    let cx = x.clamp(0.0, (w - 1) as f32);
    let cy = y.clamp(0.0, (h - 1) as f32);
    let x0 = cx as usize;
    let y0 = cy as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = cx - x0 as f32;
    let fy = cy - y0 as f32;
    let p00 = src[(y0 * w + x0) * 4 + ch] as f32;
    let p10 = src[(y0 * w + x1) * 4 + ch] as f32;
    let p01 = src[(y1 * w + x0) * 4 + ch] as f32;
    let p11 = src[(y1 * w + x1) * 4 + ch] as f32;
    let top = p00 + (p10 - p00) * fx;
    let bot = p01 + (p11 - p01) * fx;
    top + (bot - top) * fy
}

pub unsafe fn dib(width: i32, height: i32, bytes: &[u8]) -> Option<HBITMAP> {
    if width <= 0 || height <= 0 || bytes.len() != width as usize * height as usize * 4 {
        return None;
    }
    let mut info: BITMAPINFO = zeroed();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    let mut bits = null_mut();
    let bitmap = CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bitmap.is_null() || bits.is_null() {
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        return None;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), bits as *mut u8, bytes.len());
    Some(bitmap)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn liquid_meniscus_retains_outer_bend_and_strong_card_refraction() {
        for dpi in [1., 1.25, 1.75, 2.] {
            let (outer, colour, _) = liquid_lens_profile(0., false, 40. * dpi, 12.5 * dpi, 4.5 * dpi);
            assert!(outer >= 0.97, "full-size glass must retain almost all original edge displacement");
            assert!(colour >= 0.28, "the glass edge must retain visible colour separation");
            let (card, _, _) = liquid_lens_profile(0., false, 20. * dpi, 10. * dpi, 3.5 * dpi);
            assert!(card >= 0.60, "nested glass must not become visually flat");
        }
    }
    #[test]
    fn liquid_rim_never_folds_or_stretches_a_source_row_into_a_stripe() {
        for dpi in [1., 1.25, 1.75, 2.] {
            for (rim, bend, ca_strength) in [(40. * dpi, 12.5 * dpi, 4.5 * dpi),
                (20. * dpi, 10. * dpi, 3.5 * dpi), (9. * dpi, 10. * dpi, 3.5 * dpi)] {
                for channel_sign in [-1., 0., 1.] {
                    let mapped = |depth: f32| {
                        let (edge, ca, _) = liquid_lens_profile(depth / rim, false, rim, bend, ca_strength);
                        let taps = RefCell::new(Vec::new());
                        let mean = sample_refracted_channel(64., 64. + depth, 0., -1.,
                            -bend * edge, channel_sign * ca_strength * ca, dpi, [1., 1.], |_, y| {
                                taps.borrow_mut().push(y);
                                y
                            });
                        (mean, taps.into_inner())
                    };
                    let step = rim / 2048.;
                    for i in 0..2048 {
                        let depth = i as f32 * step;
                        let (a, taps_a) = mapped(depth);
                        let (b, taps_b) = mapped(depth + step);
                        let slope = (b - a) / step;
                        assert!(slope >= 0.35, "source row collapses/folds: dpi={dpi} rim={rim} channel={channel_sign} depth={depth} slope={slope}");
                        if taps_a.len() == taps_b.len() {
                            for (a, b) in taps_a.into_iter().zip(taps_b) {
                                let slope = (b - a) / step;
                                assert!(slope >= 0.35, "individual optical tap stretches: dpi={dpi} rim={rim} channel={channel_sign} depth={depth} slope={slope}");
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn parallel_refraction_matches_serial_pixels_including_fusion_and_row_padding() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) { crate::motion::set_low_power(self.0); }
        }
        let _reset = Reset(crate::motion::low_power());
        let (w, h) = (640usize, 400usize);
        let stride = w * 4 + 16;
        let source: Vec<u8> = (0..w * h)
            .flat_map(|i| [((i * 37) % 256) as u8, ((i * 61) % 256) as u8, ((i * 17) % 256) as u8, 173])
            .collect();
        let mut initial = vec![29u8; stride * h];
        for y in 0..h {
            initial[y * stride..y * stride + w * 4]
                .copy_from_slice(&source[y * w * 4..(y + 1) * w * 4]);
        }
        let pane = |id, rect| crate::fusion::geometry::Pane {
            id, rect, radius: 34., dpi: 1.25, topmost: false,
        };
        for fusion in [None, Some((vec![
            pane(1, [4., 4., 340., 396.]),
            pane(2, [340., 40., 636., 396.]),
        ], [0., 0.]))] {
            let mut serial = initial.clone();
            let mut parallel = initial.clone();
            crate::motion::set_low_power(true);
            apply_liquid_glass_pixels(
                &mut serial, stride, w, h, &source, SharpInput::None, None,
                255, 1.25, true, Some(34.), fusion.clone(), None,
            );
            crate::motion::set_low_power(false);
            apply_liquid_glass_pixels(
                &mut parallel, stride, w, h, &source, SharpInput::None, None,
                255, 1.25, true, Some(34.), fusion, None,
            );
            assert!(serial == parallel, "parallel optics must produce identical pixels, alpha and padding");
        }
    }

    #[test]
    fn texture_space_refraction_matches_desktop_sampling_with_nonuniform_scale() {
        let (w, h, stride) = (64usize, 40usize, 272usize);
        let mut pixels = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                for c in 0..4 {
                    pixels[y * stride + x * 4 + c] = ((x * 37 + y * 61 + c * 17) % 256) as u8;
                }
            }
        }
        for (dw, dh) in [(193.5, 71.25), (31.75, 83.5)] {
            for style in [0, 2, 6, 10, 22] {
                for tile in [false, true] {
                    let p = SharpPlacement {
                        buf: &pixels, w, h, stride, left_f: -13.25, top_f: 7.5,
                        dw, dh, tile, style,
                    };
                    for (bx, by) in [(-30., -10.), (0.25, 9.75), (21., 35.), (80., 50.), (200., 100.)] {
                        for (nx, ny) in [(0., 1.), (1., 0.), (0.6, -0.8)] {
                            for ca in [-5., -0.15, 0., 0.15, 5.] {
                                for ch in 0..3 {
                                    let expected = sample_refracted_channel(
                                        bx, by, nx, ny, -7.25, ca, 1.75, [1., 1.],
                                        |x, y| sample_placed_channel(&p, x, y, ch),
                                    );
                                    let actual = sample_placed_disp_2d(&p, bx, by, nx, ny, -7.25, ca, 1.75, ch);
                                    assert!((actual - expected).abs() < 0.02,
                                        "mapping changed: {actual} vs {expected}, style={style}, tile={tile}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "Read-only wallpaper preview of the actual search and group glass paths"]
    fn preview_textured_nested_glass() {
        let _ui = crate::native_ui_test_lock();
        let _runtime = crate::render::Runtime::start().unwrap();
        let path = PathBuf::from(std::env::var_os("LUMEN_EDGE_WALLPAPER").expect("wallpaper path"));
        let out = PathBuf::from(std::env::var_os("LUMEN_VERIFY_DIR").expect("output folder"));
        std::fs::create_dir_all(&out).unwrap();
        let prev = liquid_glass();
        set_liquid_glass(true);
        unsafe {
            let dpi = 1.75;
            let (w, h) = (926, 1040);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                crate::model::wide("STATIC").as_ptr(),
                crate::model::wide("Lumen nested glass preview").as_ptr(),
                WS_POPUP, 100, 100, w, h,
                null_mut(), null_mut(), null_mut(), std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            let glass = Glass::fixture_file(&path, 0., dpi);
            let mut backing = crate::compositor::Buffer::new(w, h, dpi, 34.).unwrap();
            glass.draw_blended(backing.dc, hwnd, w, h, 255);
            backing.background_alpha(255);
            let frame = crate::compositor::Buffer::new(w, h, dpi, 34.).unwrap();
            frame.copy_from(backing.dc);
            let search = [16., 14., w as f32 / dpi - 16., 60.];
            let group = [16., 80., 322., 166.];
            backing.restore_glass_card(frame.dc, search, 23.);
            backing.restore_glass_card(frame.dc, group, 20.);
            {
                let _batch = crate::render::AlphaBatch::new(frame.dc, dpi);
                let c = crate::render::Canvas::new(frame.dc, dpi);
                c.search_surface(search, 23., 0x808080, false);
                c.group_surface_live(group, 20., false);
                c.text("Search apps...", [45., 26., 260., 52.], 14., false, 0xeeeeee, 0);
                for (label, x, color) in [("Pr", 38., 0x990000), ("Ai", 137., 0x000033), ("Ps", 236., 0x332200)] {
                    c.round([x, 90., x + 44., 134.], 9., color, 255, false, false);
                    c.text(label, [x, 96., x + 44., 129.], 23., true, 0xeeddaa, 1);
                }
            }
            crate::render::flush(frame.dc);
            let mut pixels = frame.snapshot();
            for p in pixels.chunks_exact_mut(4) { p.swap(0, 2); p[3] = 255; }
            let image = image::RgbaImage::from_raw(w as u32, h as u32, pixels).unwrap();
            image::imageops::crop_imm(&image, 0, 0, 620, 330)
                .to_image().save(out.join("nested_glass.png")).unwrap();
            DestroyWindow(hwnd);
        }
        set_liquid_glass(prev);
    }

    #[test]
    fn dispersion_filter_is_continuous_at_the_single_sample_threshold() {
        let (w, h) = (48usize, 48usize);
        let mut pixels = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 24 } else { 232 };
                pixels[(y * w + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let placed = SharpPlacement {
            buf: &pixels, w, h, stride: w * 4,
            left_f: 0., top_f: 0., dw: w as f32, dh: h as f32,
            tile: false, style: 10,
        };
        for dpi in [1.0, 1.25, 1.75, 2.0] {
            for (nx, ny) in [(0., 1.), (1., 0.), (0.6, 0.8)] {
                for sign in [-1., 1.] {
                    let a = sample_disp_2d(&pixels, w, h, 24., 24., nx, ny, 0., sign * 0.0499, dpi, 1);
                    let b = sample_disp_2d(&pixels, w, h, 24., 24., nx, ny, 0., sign * 0.0501, dpi, 1);
                    assert!((a - b).abs() < 0.5, "buffer threshold seam: {a} -> {b}, dpi={dpi}");
                    let a = sample_placed_disp_2d(&placed, 24.5, 24.5, nx, ny, 0., sign * 0.0499, dpi, 1);
                    let b = sample_placed_disp_2d(&placed, 24.5, 24.5, nx, ny, 0., sign * 0.0501, dpi, 1);
                    assert!((a - b).abs() < 0.5, "wallpaper threshold seam: {a} -> {b}, dpi={dpi}");
                }
            }
        }
    }

    #[test]
    fn liquid_inner_rim_blends_into_the_original_backing_without_a_scanline() {
        let (w, h) = (200usize, 140usize);
        let original = [64u8, 64, 64, 173].repeat(w * h);
        let different_filter = [240u8, 240, 240, 255].repeat(w * h);
        let placed = SharpPlacement {
            buf: &different_filter, w, h, stride: w * 4,
            left_f: 0., top_f: 0., dw: w as f32, dh: h as f32,
            tile: false, style: 10,
        };
        let mut out = original.clone();
        apply_liquid_glass_pixels(
            &mut out, w * 4, w, h, &original, SharpInput::None, Some(&placed),
            255, 1., true, Some(32.), None, None,
        );
        let at = |y: usize| out[(y * w + w / 2) * 4] as i32;
        for y in 24..60 {
            assert!((at(y + 1) - at(y)).abs() < 55, "inner rim cliff at y={y}: {} -> {}", at(y), at(y + 1));
        }
        assert_eq!(&out[(70 * w + 100) * 4..][..4], &[64, 64, 64, 173]);
        assert!(out.chunks_exact(4).all(|p| p[3] == 173), "optics must not change alpha coverage");
    }

    #[test]
    fn viewport_render_matches_the_old_monitor_crop_and_only_blends_in_viewport_memory() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let make_window = |x, y, w, h| {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    wide("STATIC").as_ptr(),
                    wide("Lumen crop test").as_ptr(),
                    WS_POPUP,
                    x,
                    y,
                    w,
                    h,
                    null_mut(),
                    null_mut(),
                    windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()),
                    std::ptr::null(),
                )
            };
            let hwnd = make_window(83, 47, 96, 64);
            let other = make_window(10, 10, 16, 16);
            assert!(!hwnd.is_null() && !other.is_null());
            let mut glass = Glass::new(0., 1.);
            struct Reset(bool, bool);
            impl Drop for Reset {
                fn drop(&mut self) {
                    set_liquid_glass(self.0);
                    crate::motion::set_low_power(self.1);
                }
            }
            let _reset = Reset(liquid_glass(), crate::motion::low_power());
            set_liquid_glass(false);
            crate::motion::set_low_power(false);
            glass.width = 512;
            glass.height = 320;
            glass.natural = [512., 320.];
            let pixels: Vec<u8> = (0..320)
                .flat_map(|y| {
                    (0..512).flat_map(move |x| {
                        let c = (20 + x * 150 / 512 + y * 50 / 320) as u8;
                        [c, c, c, 255]
                    })
                })
                .collect();
            glass.bitmap = Some(Rc::new(OwnedBitmap(dib(512, 320, &pixels).unwrap())));
            let mut window: RECT = zeroed();
            GetWindowRect(hwnd, &mut window);
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            );
            let area = monitor.rcMonitor;
            let full = crate::compositor::Buffer::plain(
                area.right - area.left,
                area.bottom - area.top,
                1.,
            )
            .unwrap();
            let crop = crate::compositor::Buffer::plain(96, 64, 1.).unwrap();
            for style in [10, 6] {
                glass.style = style;
                PatBlt(full.dc, 0, 0, full.width, full.height, BLACKNESS);
                PatBlt(crop.dc, 0, 0, 96, 64, WHITENESS);
                glass.draw_layer_uncached(
                    full.dc,
                    hwnd,
                    full.width,
                    full.height,
                    glass.bitmap.as_ref().unwrap().0,
                    glass.blur_size(),
                    area,
                    false,
                );
                glass.draw_blended(crop.dc, hwnd, 96, 64, 255);
                for y in (0..64).step_by(8) {
                    for x in (0..96).step_by(8) {
                        let expected = GetPixel(
                            full.dc,
                            window.left - area.left + x,
                            window.top - area.top + y,
                        );
                        let actual = GetPixel(crop.dc, x, y);
                        for shift in [0, 8, 16] {
                            assert!(
                                (((expected >> shift) & 255) as i32
                                    - ((actual >> shift) & 255) as i32)
                                    .abs()
                                    <= 2
                            );
                        }
                    }
                }
                assert!(glass.blend_buffer.borrow().is_none());
            }
            glass.style = 10;
            glass.draw_blended(crop.dc, hwnd, 96, 64, 128);
            {
                let scratch = glass.blend_buffer.borrow();
                let (owner, buffer) = scratch.as_ref().unwrap();
                assert_eq!(*owner, hwnd);
                assert_eq!((buffer.width, buffer.height), (96, 64));
            }
            let auxiliary = crate::compositor::Buffer::plain(16, 16, 1.).unwrap();
            glass.draw(auxiliary.dc, other, 16, 16);
            assert!(glass.blend_buffer.borrow().is_some());
            glass.draw(crop.dc, hwnd, 96, 64);
            assert!(glass.blend_buffer.borrow().is_none());
            DestroyWindow(other);
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn wallpaper_decoder_applies_exif_rotation_before_layout_and_sampling() {
        let image = image::RgbImage::from_fn(16, 8, |x, _| {
            image::Rgb(if x < 8 { [255, 0, 0] } else { [0, 0, 255] })
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
            .encode_image(&image)
            .unwrap();
        let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x08\0\0\0\0\0\0\0";
        let mut file = vec![255, 216, 255, 225];
        file.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        file.extend_from_slice(exif);
        file.extend_from_slice(&jpeg[2..]);
        let path = std::env::temp_dir().join(format!(
            "lumen-wallpaper-orientation-{}.jpg",
            std::process::id()
        ));
        std::fs::write(&path, file).unwrap();
        let result = super::decode_wallpaper(&path);
        std::fs::remove_file(path).unwrap();
        let decoded = result.unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (8, 16));
        assert!(decoded.get_pixel(4, 2)[2] > 230);
        assert!(decoded.get_pixel(4, 13)[0] > 230);
    }
    #[test]
    fn fill_crop_matches_windows_wallpaper_and_other_layouts_stay_centered() {
        // Windows Fill/Span crops 1/3 from the top and 2/3 from the bottom when taller than viewport.
        assert_eq!(super::vertical_offset(2160., 3840., 10, false), -560.);
        assert_eq!(super::vertical_offset(2160., 3840., 22, false), -560.);
        assert!((super::vertical_offset(2160., 2560., 10, false) + 133.33333).abs() < 0.01);
        assert_eq!(super::vertical_offset(2160., 1600., 6, false), 280.);
        assert_eq!(super::vertical_offset(2160., 6000., 0, false), -1920.);
        assert_eq!(super::vertical_offset(2160., 3840., 10, true), 0.);
    }
    #[test]
    fn invalid_bitmap_dimensions_are_rejected() {
        unsafe {
            assert!(super::dib(-1, 10, &[]).is_none());
            assert!(super::dib(2, 2, &[0; 4]).is_none());
        }
    }
    #[test]
    fn transcoded_cache_path_decodes_utf16_source_after_header() {
        let mut raw = vec![0u8; 24];
        for ch in "E:\\wallpaper\\test.jpg".encode_utf16() {
            raw.extend_from_slice(&ch.to_le_bytes());
        }
        raw.extend_from_slice(&[0, 0, 0, 0]);
        assert_eq!(
            super::parse_transcoded_cache_path(&raw),
            Some(PathBuf::from("E:\\wallpaper\\test.jpg"))
        );
        assert_eq!(super::parse_transcoded_cache_path(&[0u8; 10]), None);
    }
    #[test]
    fn liquid_glass_refracts_rim_with_channel_shift_and_preserves_interior() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                set_liquid_glass(self.0);
            }
        }
        let _reset = Reset(liquid_glass());
        set_liquid_glass(false);
        let glass = Glass::new(0., 1.);
        let rev_before = glass.revision();
        set_liquid_glass(true);
        assert_ne!(glass.revision(), rev_before);

        unsafe {
            let mut buf = crate::compositor::Buffer::plain(120, 80, 1.0).unwrap();
            let px = buf.pixels_mut();
            for y in 0..80usize {
                for x in 0..120usize {
                    let v = ((x * 2).min(240)) as u8;
                    let i = (y * 120 + x) * 4;
                    px[i] = v;
                    px[i + 1] = v;
                    px[i + 2] = v;
                    px[i + 3] = 255;
                }
            }
            let center_before = buf.pixels_mut()[(40 * 120 + 60) * 4..][..4].to_vec();
            glass.apply_liquid_glass(buf.dc, 120, 80);
            let after = buf.pixels_mut();
            // Center interior is untouched.
            assert_eq!(&after[(40 * 120 + 60) * 4..][..4], &center_before[..]);
            // Left rim pixel has chromatic channel separation (B, G, R sampled at different offsets).
            let rim_px = &after[(40 * 120 + 4) * 4..][..4];
            assert_ne!(rim_px[0], rim_px[2]);
        }
    }

    #[test]
    fn refresh_async_contended_loading_key_does_not_block_subsequent_refresh() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let Some((path, modified, _, _)) = Glass::query_wallpaper() else {
                return;
            };
            clear_cache();
            let mut glass = Glass::new(13., 1.);
            {
                *LOADING_KEY.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some((path.clone(), modified));
            }
            assert!(!glass.refresh_async(null_mut(), 0));
            assert!(glass.bitmap.is_none());
            assert!(glass.path.as_os_str().is_empty());
            *LOADING_KEY.lock().unwrap_or_else(|e| e.into_inner()) = None;
            assert!(glass.refresh());
            assert!(glass.bitmap.is_some());
        }
    }

    #[test]
    fn liquid_glass_checker_and_wave_visual_verification() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                set_liquid_glass(self.0);
            }
        }
        let _reset = Reset(liquid_glass());
        set_liquid_glass(true);

        let verify_dir = std::env::var_os("LUMEN_VERIFY_DIR").map(PathBuf::from);
        if let Some(ref dir) = verify_dir {
            std::fs::create_dir_all(dir).unwrap();
        }
        let temp_dir = std::env::temp_dir().join(format!(
            "lumen-glass-verify-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        struct CleanupDir(PathBuf);
        impl Drop for CleanupDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = CleanupDir(temp_dir.clone());

        let (cw, ch) = (640usize, 480usize);
        let checker_path = temp_dir.join("checker.png");
        let mut checker_img = image::RgbaImage::new(cw as u32, ch as u32);
        for y in 0..ch {
            for x in 0..cw {
                let u = (x as i32 * 3 + y as i32 * 2).div_euclid(54);
                let v = (x as i32 * 3 - y as i32 * 2).div_euclid(54);
                let dark = ((u + v) & 1) == 0;
                let base = if dark { 54.0 } else { 228.0 };
                let grad = (x as f32 / cw as f32 - 0.5) * 26.0 + (y as f32 / ch as f32 - 0.5) * 18.0;
                let l = (base + grad).clamp(18.0, 245.0).round() as u8;
                checker_img.put_pixel(x as u32, y as u32, image::Rgba([l, l, l.saturating_add(4), 255]));
            }
        }
        checker_img.save(&checker_path).unwrap();

        // 1) Optical metric check via `apply_liquid_glass_rgba` (SharpInput::Buffer, 1:1 native).
        let (pw, ph, pad) = (320usize, 240usize, 24usize);
        let sw = pw + pad * 2;
        let sh = ph + pad * 2;
        let mut sharp_rgba = vec![0u8; sw * sh * 4];
        for y in 0..sh {
            for x in 0..sw {
                let px = checker_img.get_pixel((x + 60) as u32, (y + 60) as u32);
                let i = (y * sw + x) * 4;
                sharp_rgba[i..i + 4].copy_from_slice(&px.0);
            }
        }
        // Separable box-blurred frosted interior matching the checker wallpaper.
        let mut frosted = vec![0u8; pw * ph * 4];
        let mut tmp_blur = vec![0f32; sw * sh * 3];
        for y in 0..sh {
            for x in 0..sw {
                let mut acc = [0f32; 3];
                let mut n = 0f32;
                for dx in -8i32..=8 {
                    let sx = (x as i32 + dx).clamp(0, sw as i32 - 1) as usize;
                    let i = (y * sw + sx) * 4;
                    for c in 0..3 {
                        acc[c] += sharp_rgba[i + c] as f32;
                    }
                    n += 1.0;
                }
                let o = (y * sw + x) * 3;
                for c in 0..3 {
                    tmp_blur[o + c] = acc[c] / n;
                }
            }
        }
        for y in 0..ph {
            for x in 0..pw {
                let mut acc = [0f32; 3];
                let mut n = 0f32;
                for dy in -8i32..=8 {
                    let sy = (y as i32 + pad as i32 + dy).clamp(0, sh as i32 - 1) as usize;
                    let o = (sy * sw + x + pad) * 3;
                    for c in 0..3 {
                        acc[c] += tmp_blur[o + c];
                    }
                    n += 1.0;
                }
                let i = (y * pw + x) * 4;
                for c in 0..3 {
                    frosted[i + c] = (acc[c] / n).round() as u8;
                }
                frosted[i + 3] = 255;
            }
        }
        let mut glass_out = frosted.clone();
        apply_liquid_glass_rgba(null_mut(), &mut glass_out, pw, ph, &sharp_rgba, sw, sh, pad, 1.0);
        // Interior beyond `rim = 40` (`x = 70..250, y = 70..170`) must be completely untouched.
        for y in 75..165 {
            for x in 75..245 {
                let i = (y * pw + x) * 4;
                assert_eq!(&glass_out[i..i + 3], &frosted[i..i + 3], "interior must stay untouched");
            }
        }
        // Chromatic aberration near outer rim (`x = 7, y = 120` has `d ~ -2.5`).
        let rim_i = (120 * pw + 7) * 4;
        assert!(
            glass_out[rim_i] != glass_out[rim_i + 2]
                || glass_out[rim_i + 4] != glass_out[rim_i + 6],
            "rim must show chromatic dispersion"
        );
        // Check soft clarity decay without any hard step cliff near u = 0.45 along the rim band.
        // Compare average absolute high-pass deviation from frosted across rings u = 0.15..0.85.
        let mut band_energy = Vec::new();
        for band_x in (6usize..40).step_by(4) {
            let mut sum = 0.0f32;
            let mut cnt = 0.0f32;
            for y in 60usize..180 {
                let i = (y * pw + band_x) * 4;
                let diff = (glass_out[i + 1] as f32 - frosted[i + 1] as f32).abs();
                sum += diff;
                cnt += 1.0;
            }
            band_energy.push(sum / cnt);
        }
        for w in band_energy.windows(2) {
            let step = (w[0] - w[1]).abs();
            assert!(
                step < 28.0,
                "rim clarity/bend transition must fall off softly without cliff steps: {band_energy:?}"
            );
        }

        // 2) Full desktop compositor scenes (Checker + Wave with outer widget + inner Coverflow pill).
        let wave_path = temp_dir.join("wave.png");
        let mut wave_img = image::RgbaImage::new(cw as u32, ch as u32);
        for y in 0..ch {
            let ny = y as f32 / ch as f32;
            for x in 0..cw {
                let nx = x as f32 / cw as f32;
                let ribbon1 = ((nx * 3.1 + (ny * 4.5).sin() * 0.42).sin() * 0.5 + 0.5).powf(2.2);
                let ribbon2 = (((nx - 0.35) * 4.4 - (ny * 3.2).cos() * 0.5).sin() * 0.5 + 0.5).powf(3.0);
                let crest = (1.0 - ((ny - (0.52 + 0.18 * (nx * 4.0).sin())).abs() * 7.5).min(1.0)).powf(2.0);
                let r = (10.0 + 48.0 * ribbon2 + 120.0 * crest).clamp(0.0, 255.0) as u8;
                let g = (42.0 + 135.0 * ribbon1 + 115.0 * crest).clamp(0.0, 255.0) as u8;
                let b = (115.0 + 130.0 * ribbon1.max(ribbon2) + 45.0 * crest).clamp(0.0, 255.0) as u8;
                wave_img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
            }
        }
        wave_img.save(&wave_path).unwrap();

        unsafe {
            let render_scene = |wall_path: &Path, wall: &image::RgbaImage, tint_rgba: [u8; 4]| -> image::RgbaImage {
                let mut canvas = wall.clone();
                let mut monitor: MONITORINFO = zeroed();
                monitor.cbSize = size_of::<MONITORINFO>() as u32;
                let primary = MonitorFromPoint(POINT { x: 10, y: 10 }, MONITOR_DEFAULTTOPRIMARY);
                GetMonitorInfoW(primary, &mut monitor);
                let ml = monitor.rcMonitor.left;
                let mt = monitor.rcMonitor.top;
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    wide("STATIC").as_ptr(),
                    wide("Lumen visual glass test").as_ptr(),
                    WS_POPUP,
                    ml + 100,
                    mt + 60,
                    440,
                    340,
                    null_mut(),
                    null_mut(),
                    windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()),
                    std::ptr::null(),
                );
                assert!(!hwnd.is_null());
                let glass = Glass::fixture_tiled_file(wall_path, 14.0, 1.0);
                let mut frame = crate::compositor::Buffer::new(440, 340, 1.0, 30.0).unwrap();
                let brush = CreateSolidBrush(0x281e18);
                FillRect(frame.dc, &RECT { left: 0, top: 0, right: 440, bottom: 340 }, brush);
                DeleteObject(brush);
                glass.draw_blended(frame.dc, hwnd, 440, 340, 225);

                // Subtle translucent theme tint across widget face, plus interior pill card via `restore_glass_card`.
                {
                    let px = frame.pixels_mut();
                    let a = tint_rgba[3] as f32 / 255.0;
                    for p in px.chunks_exact_mut(4) {
                        p[0] = (p[0] as f32 * (1.0 - a) + tint_rgba[2] as f32 * a).round() as u8;
                        p[1] = (p[1] as f32 * (1.0 - a) + tint_rgba[1] as f32 * a).round() as u8;
                        p[2] = (p[2] as f32 * (1.0 - a) + tint_rgba[0] as f32 * a).round() as u8;
                    }
                }
                let mut backing = crate::compositor::Buffer::new(440, 340, 1.0, 30.0).unwrap();
                let card_rect = [70.0f32, 226.0, 370.0, 294.0];
                backing.pixels_mut().copy_from_slice(frame.pixels_mut());
                backing.restore_glass_card(frame.dc, card_rect, 34.0);
                {
                    let fpx = frame.pixels_mut();
                    let (cx, cy) = ((card_rect[0] + card_rect[2]) * 0.5, (card_rect[1] + card_rect[3]) * 0.5);
                    let (hw, hh, r) = ((card_rect[2] - card_rect[0]) * 0.5, (card_rect[3] - card_rect[1]) * 0.5, 34.0);
                    for py in 226..294usize {
                        for px in 70..370usize {
                            let dx = (px as f32 + 0.5 - cx).abs() - (hw - r);
                            let dy = (py as f32 + 0.5 - cy).abs() - (hh - r);
                            let d = dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r;
                            if d < 0.5 {
                                let i = (py * 440 + px) * 4;
                                let rim = (1.0 - (-d).clamp(0.0, 2.0) * 0.5) * (0.5 - d).clamp(0.0, 1.0);
                                let add = rim * 55.0;
                                for c in 0..3 {
                                    fpx[i + c] = (fpx[i + c] as f32 + add).min(255.0) as u8;
                                }
                            }
                        }
                    }
                }
                let _cap = crate::compositor::test_capture::start();
                frame.background_alpha(255);
                frame.bevel_for(hwnd);
                assert!(frame.present_with_alpha(hwnd, true));
                let cap = crate::compositor::test_capture::get(hwnd).expect("widget upload");
                DestroyWindow(hwnd);

                // Alpha-composite premultiplied BGRA widget onto 640x480 RGBA background canvas.
                let ox = 100usize;
                let oy = 60usize;
                for y in 0..cap.h as usize {
                    for x in 0..cap.w as usize {
                        let si = (y * cap.w as usize + x) * 4;
                        let a = cap.bgra[si + 3] as u32;
                        if a == 0 {
                            continue;
                        }
                        let inv = 255 - a;
                        let dst = canvas.get_pixel_mut((ox + x) as u32, (oy + y) as u32);
                        dst.0[0] = (cap.bgra[si + 2] as u32 + (dst.0[0] as u32 * inv + 127) / 255).min(255) as u8;
                        dst.0[1] = (cap.bgra[si + 1] as u32 + (dst.0[1] as u32 * inv + 127) / 255).min(255) as u8;
                        dst.0[2] = (cap.bgra[si] as u32 + (dst.0[2] as u32 * inv + 127) / 255).min(255) as u8;
                        dst.0[3] = 255;
                    }
                }
                canvas
            };

            let checker_scene = render_scene(&checker_path, &checker_img, [235, 240, 248, 80]);
            let wave_scene = render_scene(&wave_path, &wave_img, [225, 238, 255, 58]);
            if let Some(ref dir) = verify_dir {
                checker_scene.save(dir.join("verify_glass_checker.png")).unwrap();
                wave_scene.save(dir.join("verify_glass_wave.png")).unwrap();
            }
            // Verify wave widget corners are smooth transparent rounded & outer rim has no extreme blowout.
            let mut wave_clipped = 0usize;
            for y in 85..355u32 {
                for x in 125..495u32 {
                    let p = wave_scene.get_pixel(x, y).0;
                    if p[0] == 255 && p[1] == 255 && p[2] == 255 {
                        wave_clipped += 1;
                    }
                }
            }
            assert!(
                wave_clipped < 120,
                "Liquid Glass wave scene must not clamp/blow out highlights ({wave_clipped} pure white px)"
            );
        }
    }
    #[test]
    fn low_power_wallpaper_cap_uses_hd_to_full_hd_range() {
        assert_eq!(Glass::wallpaper_cap_for_screen(3840, 2160, true), Some((1920, 1920)));
        assert_eq!(Glass::wallpaper_cap_for_screen(2560, 1440, true), Some((1920, 1920)));
        assert_eq!(Glass::wallpaper_cap_for_screen(1920, 1080, true), Some((1920, 1920)));
        assert_eq!(Glass::wallpaper_cap_for_screen(1080, 1920, true), Some((1920, 1920)));
        assert_eq!(Glass::wallpaper_cap_for_screen(1366, 768, true), Some((1366, 1366)));
        assert_eq!(Glass::wallpaper_cap_for_screen(800, 600, true), Some((1280, 1280)));
        assert_eq!(Glass::wallpaper_cap_for_screen(0, 0, true), Some((1920, 1920)));
        assert_eq!(Glass::wallpaper_cap_for_screen(3840, 2160, false), Some((3840, 2160)));
        assert_eq!(Glass::wallpaper_cap_for_screen(1366, 768, false), Some((1920, 1080)));
        assert_eq!(Glass::wallpaper_cap_for_screen(0, 0, false), None);
    }
    #[test]
    fn sharp_bitmap_reuses_full_preview_after_release_large() {
        let _ui = crate::native_ui_test_lock();
        let preview = image::RgbaImage::from_pixel(1920, 1080, image::Rgba([24, 64, 128, 255]));
        let source = WallpaperSource {
            path: PathBuf::from("Z:\\nonexistent\\wallpaper.png"),
            small: RefCell::new(Some(preview.clone())),
            preview,
            sharp: Cell::new(null_mut()),
            width: 1920,
            height: 1080,
            natural: [3840., 2160.],
            accent: 0,
        };
        source.release_large();
        assert!(source.small.borrow().is_none());
        assert!(source.has_full_preview());
        unsafe {
            let bmp = source.sharp_bitmap();
            assert!(!bmp.is_null());
            let mut ds: DIBSECTION = zeroed();
            let got = GetObjectW(bmp as _, size_of::<DIBSECTION>() as i32, &mut ds as *mut _ as _);
            assert!(got >= size_of::<BITMAP>() as i32);
            assert_eq!(ds.dsBm.bmWidth, 1920);
            assert_eq!(ds.dsBm.bmHeight, 1080);
        }
    }
    #[test]
    fn raw_wallpaper_accent_returns_unboosted_dominant_color() {
        let _ui = crate::native_ui_test_lock();
        let glass = Glass::new(13.0, 1.0);
        let dominant = crate::accent::dominant(&[24, 52, 92, 255].repeat(64));
        glass.accent.set(dominant);
        assert_eq!(glass.raw_wallpaper_accent(), dominant);
        assert_eq!(glass.wallpaper_accent(), crate::accent::vivid(dominant));
        assert_ne!(glass.raw_wallpaper_accent(), glass.wallpaper_accent());
    }
    #[test]
    fn liquid_glass_stays_active_during_resize_outside_low_power() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                wide("STATIC").as_ptr(),
                wide("Lumen liquid resize test").as_ptr(),
                WS_POPUP,
                100,
                100,
                120,
                80,
                null_mut(),
                null_mut(),
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            struct Reset(HWND, bool, bool);
            impl Drop for Reset {
                fn drop(&mut self) {
                    set_resizing(self.0, false);
                    set_liquid_glass(self.1);
                    crate::motion::set_low_power(self.2);
                    unsafe {
                        DestroyWindow(self.0);
                    }
                }
            }
            let _reset = Reset(hwnd, liquid_glass(), crate::motion::low_power());
            set_liquid_glass(true);
            set_resizing(hwnd, true);

            let mut glass = Glass::new(0., 1.);
            glass.style = 2; // Stretch across desktop
            glass.width = 256;
            glass.height = 256;
            glass.natural = [256., 256.];
            let wall: Vec<u8> = (0..256)
                .flat_map(|y| {
                    (0..256).flat_map(move |x| {
                        let r = ((x * 3 + y) % 240) as u8;
                        let g = ((x + y * 2) % 220) as u8;
                        let b = 220u8.saturating_sub(((x * 2) % 180) as u8);
                        [b, g, r, 255]
                    })
                })
                .collect();
            glass.bitmap = Some(Rc::new(OwnedBitmap(dib(256, 256, &wall).unwrap())));

            crate::motion::set_low_power(false);
            let normal = crate::compositor::Buffer::plain(120, 80, 1.0).unwrap();
            glass.draw_blended(normal.dc, hwnd, 120, 80, 255);

            crate::motion::set_low_power(true);
            let low = crate::compositor::Buffer::plain(120, 80, 1.0).unwrap();
            glass.draw_blended(low.dc, hwnd, 120, 80, 255);

            let npx = normal.snapshot();
            let lpx = low.snapshot();
            let mut rim_diff = 0u32;
            for x in 1..12usize {
                let i = (40 * 120 + x) * 4;
                rim_diff += npx[i].abs_diff(lpx[i]) as u32
                    + npx[i + 1].abs_diff(lpx[i + 1]) as u32
                    + npx[i + 2].abs_diff(lpx[i + 2]) as u32;
            }
            assert!(
                rim_diff > 15,
                "liquid glass rim must remain active while resizing outside low-power mode (rim_diff={rim_diff})"
            );
        }
    }
}
