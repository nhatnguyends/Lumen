//! Frosted wallpaper, rendered from the wallpaper FILE, never from screen pixels.
use crate::model::wide;
use image::{imageops, DynamicImage, ImageDecoder, ImageReader};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant, SystemTime},
};
use windows_sys::{
    core::GUID,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_LOCAL_SERVER,
            COINIT_MULTITHREADED,
        },
        System::Registry::*,
        System::Threading::*,
        UI::WindowsAndMessaging::*,
    },
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

const SHARP_MAX_DIM: u32 = 1920;
const PREVIEW_MAX_DIM: u32 = 960;

fn fit_dimensions(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w <= max_w && h <= max_h {
        return (w, h);
    }
    let scale = (max_w as f32 / w.max(1) as f32).min(max_h as f32 / h.max(1) as f32);
    (
        ((w as f32 * scale).round() as u32).max(1),
        ((h as f32 * scale).round() as u32).max(1),
    )
}

fn build_sharp_bytes(rgba: &image::RgbaImage) -> (Vec<u8>, i32, i32) {
    let (sw, sh) = fit_dimensions(rgba.width(), rgba.height(), SHARP_MAX_DIM, SHARP_MAX_DIM);
    let mut sharp_bytes = if sw == rgba.width() && sh == rgba.height() {
        rgba.as_raw().clone()
    } else {
        imageops::thumbnail(rgba, sw, sh).into_raw()
    };
    for p in sharp_bytes.chunks_exact_mut(4) {
        p.swap(0, 2);
        p[3] = 255;
    }
    (sharp_bytes, sw as i32, sh as i32)
}

struct WallpaperSource {
    path: PathBuf,
    cap: Option<(u32, u32)>,
    small: RefCell<Option<image::RgbaImage>>,
    preview: image::RgbaImage,
    sharp: Cell<HBITMAP>,
    sharp_size: Cell<(i32, i32)>,
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
        let (pw, ph) =
            fit_dimensions(small.width(), small.height(), PREVIEW_MAX_DIM, PREVIEW_MAX_DIM);
        if pw == small.width() && ph == small.height() {
            small.clone()
        } else {
            imageops::thumbnail(small, pw, ph)
        }
    }
    fn release_large(&self) {
        self.small.borrow_mut().take();
    }
    fn has_full_preview(&self) -> bool {
        self.preview.width() as i32 == self.width && self.preview.height() as i32 == self.height
    }
    fn sharp_dimensions(&self) -> (i32, i32) {
        let (sw, sh) = self.sharp_size.get();
        if sw > 0 && sh > 0 {
            (sw, sh)
        } else {
            (self.width, self.height)
        }
    }
    unsafe fn clear_sharp(&self) {
        let sharp = self.sharp.replace(null_mut());
        if !sharp.is_null() {
            DeleteObject(sharp);
        }
        self.sharp_size.set((0, 0));
    }
    unsafe fn sharp_bitmap(&self) -> HBITMAP {
        let existing = self.sharp.get();
        if !existing.is_null() {
            return existing;
        }
        let (sharp_bytes, sw, sh) = {
            let guard = self.small.borrow();
            if let Some(small) = guard.as_ref() {
                build_sharp_bytes(small)
            } else if self.has_full_preview() {
                build_sharp_bytes(&self.preview)
            } else {
                drop(guard);
                let sharp_cap = self
                    .cap
                    .or_else(|| Glass::wallpaper_cap())
                    .map(|(cw, ch)| (cw.min(SHARP_MAX_DIM), ch.min(SHARP_MAX_DIM)))
                    .or(Some((SHARP_MAX_DIM, SHARP_MAX_DIM)));
                let Some((decoded, _)) = Glass::decode_small(&self.path, sharp_cap) else {
                    return null_mut();
                };
                build_sharp_bytes(&decoded)
            }
        };
        if let Some(bitmap) = dib(sw, sh, &sharp_bytes) {
            self.sharp.set(bitmap);
            self.sharp_size.set((sw, sh));
            bitmap
        } else {
            null_mut()
        }
    }
}

pub(crate) fn normalize_path_key(path: &Path) -> String {
    let s = path.to_string_lossy();
    let trimmed = if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.into_owned()
    };
    trimmed.replace('/', "\\").to_ascii_lowercase()
}

type SourceKey = (String, Option<SystemTime>, u64);
type BlurKey = (SourceKey, i32, i32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WallpaperSpec {
    pub path: PathBuf,
    pub key: SourceKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MonitorWallpaperSpec {
    pub rect: [i32; 4],
    pub slot: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DesktopWallpaperSpec {
    pub unique_sources: Vec<WallpaperSpec>,
    pub monitors: Vec<MonitorWallpaperSpec>,
    pub shared: bool,
    pub style: u32,
    pub tile: bool,
    pub bg_color: u32,
}

impl DesktopWallpaperSpec {
    fn virtual_bounds(&self) -> Option<RECT> {
        let first = self.monitors.first()?;
        let mut l = first.rect[0];
        let mut t = first.rect[1];
        let mut r = first.rect[2];
        let mut b = first.rect[3];
        for m in &self.monitors[1..] {
            l = l.min(m.rect[0]);
            t = t.min(m.rect[1]);
            r = r.max(m.rect[2]);
            b = b.max(m.rect[3]);
        }
        (r > l && b > t).then_some(RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        })
    }
}

#[derive(Clone)]
struct LoadedWallpaper {
    spec: WallpaperSpec,
    bitmap: Rc<OwnedBitmap>,
    source: Rc<WallpaperSource>,
    blur_width: i32,
    blur_height: i32,
}

struct PendingSlot {
    spec: WallpaperSpec,
    width: i32,
    height: i32,
    preview: image::RgbaImage,
    cap: Option<(u32, u32)>,
    natural: [f32; 2],
    accent: u32,
    blurred_bytes: Vec<u8>,
    blur_w: i32,
    blur_h: i32,
    sharp_bytes: Option<(Vec<u8>, i32, i32)>,
}

struct PendingDesktopWallpaper {
    spec: DesktopWallpaperSpec,
    blur: f32,
    dpi: f32,
    low_power: bool,
    slots: Vec<(usize, PendingSlot)>,
}

static PENDING_WALLPAPER: Mutex<Option<PendingDesktopWallpaper>> = Mutex::new(None);
static LOADING_KEY: Mutex<Option<Vec<SourceKey>>> = Mutex::new(None);

thread_local! {
    static WALLPAPER_SOURCES: RefCell<HashMap<SourceKey, Rc<WallpaperSource>>> = RefCell::new(HashMap::new());
    static BLURRED_CACHE: RefCell<HashMap<BlurKey, (Rc<OwnedBitmap>, i32, i32)>> = RefCell::new(HashMap::new());
    static PATH_BUF: RefCell<Vec<u16>> = RefCell::new(vec![0u16; 32768]);
    static LIQUID_SRC: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static BLIT_XS: RefCell<Vec<(usize, usize, u32)>> = const { RefCell::new(Vec::new()) };
    static BLIT_YS: RefCell<Vec<(usize, usize, u32)>> = const { RefCell::new(Vec::new()) };
    static LAST_QUERY: RefCell<Option<(Instant, DesktopWallpaperSpec)>> = const { RefCell::new(None) };
    static LIVE_WP_CACHE: RefCell<Option<(Instant, Option<DesktopWallpaperSpec>)>> = const { RefCell::new(None) };
}

pub fn invalidate_query_cache() {
    LAST_QUERY.with(|c| *c.borrow_mut() = None);
}

pub fn clear_cache() {
    WALLPAPER_SOURCES.with(|cache| cache.borrow_mut().clear());
    BLURRED_CACHE.with(|cache| cache.borrow_mut().clear());
    LIQUID_SRC.with(|buf| *buf.borrow_mut() = Vec::new());
    BLIT_XS.with(|buf| *buf.borrow_mut() = Vec::new());
    BLIT_YS.with(|buf| *buf.borrow_mut() = Vec::new());
    LAST_QUERY.with(|c| *c.borrow_mut() = None);
    LIVE_WP_CACHE.with(|c| *c.borrow_mut() = None);
}

fn prune_caches(active_keys: &[SourceKey]) -> bool {
    let mut evicted = false;
    WALLPAPER_SOURCES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let before = cache.len();
        cache.retain(|k, _| active_keys.contains(k));
        if cache.len() < before {
            evicted = true;
        }
    });
    let blur_cap = active_keys.len().max(1) * 4;
    BLURRED_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let before = cache.len();
        cache.retain(|(k, _, _), _| active_keys.contains(k));
        if cache.len() > blur_cap {
            cache.clear();
        }
        if cache.len() < before {
            evicted = true;
        }
    });
    evicted
}

pub(crate) fn trim_working_set() {
    LIQUID_SRC.with(|buf| *buf.borrow_mut() = Vec::new());
    BLIT_XS.with(|buf| *buf.borrow_mut() = Vec::new());
    BLIT_YS.with(|buf| *buf.borrow_mut() = Vec::new());
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
        if !enabled && frosted_glass() {
            WALLPAPER_SOURCES.with(|cache| {
                for source in cache.borrow().values() {
                    unsafe {
                        source.clear_sharp();
                    }
                }
            });
        }
    }
}
pub fn liquid_glass() -> bool {
    LIQUID_GLASS.load(Ordering::Relaxed)
}
pub fn set_frosted_glass(enabled: bool) {
    if FROSTED_GLASS.swap(enabled, Ordering::Relaxed) != enabled {
        LIQUID_GLASS_REVISION.fetch_add(1, Ordering::Relaxed);
        if enabled && !liquid_glass() {
            WALLPAPER_SOURCES.with(|cache| {
                for source in cache.borrow().values() {
                    unsafe {
                        source.clear_sharp();
                    }
                }
            });
        }
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
    slots: Vec<LoadedWallpaper>,
    monitors: Vec<MonitorWallpaperSpec>,
    shared: bool,
    bg_color: u32,
    desktop_spec: Option<DesktopWallpaperSpec>,
}

unsafe fn fill_solid_bg(dc: HDC, width: i32, height: i32, colorref: u32) {
    if width <= 0 || height <= 0 {
        return;
    }
    if colorref == 0 {
        PatBlt(dc, 0, 0, width, height, BLACKNESS);
    } else {
        let brush = CreateSolidBrush(colorref);
        let rr = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        FillRect(dc, &rr, brush);
        DeleteObject(brush);
    }
}

fn colorref_to_bgr_f32(colorref: u32) -> [f32; 3] {
    [
        ((colorref >> 16) & 0xff) as f32,
        ((colorref >> 8) & 0xff) as f32,
        (colorref & 0xff) as f32,
    ]
}

fn rect_intersect(a: RECT, b: RECT) -> Option<RECT> {
    let left = a.left.max(b.left);
    let top = a.top.max(b.top);
    let right = a.right.min(b.right);
    let bottom = a.bottom.min(b.bottom);
    (right > left && bottom > top).then_some(RECT {
        left,
        top,
        right,
        bottom,
    })
}

fn pick_best_monitor_index(monitors: &[MonitorWallpaperSpec], target: RECT) -> Option<usize> {
    if monitors.is_empty() {
        return None;
    }
    let has_area = target.right > target.left && target.bottom > target.top;
    if has_area {
        let mut best_area = 0i64;
        let mut best_idx = None;
        for (idx, m) in monitors.iter().enumerate() {
            let mr = RECT {
                left: m.rect[0],
                top: m.rect[1],
                right: m.rect[2],
                bottom: m.rect[3],
            };
            if let Some(inter) = rect_intersect(mr, target) {
                let area = (inter.right - inter.left) as i64 * (inter.bottom - inter.top) as i64;
                if area > best_area {
                    best_area = area;
                    best_idx = Some(idx);
                }
            }
        }
        if best_idx.is_some() {
            return best_idx;
        }
    }
    let cx = if has_area {
        (target.left as i64 + target.right as i64) / 2
    } else {
        target.left as i64
    };
    let cy = if has_area {
        (target.top as i64 + target.bottom as i64) / 2
    } else {
        target.top as i64
    };
    monitors
        .iter()
        .enumerate()
        .min_by_key(|(_, m)| {
            let dx = if cx < m.rect[0] as i64 {
                m.rect[0] as i64 - cx
            } else if cx > m.rect[2] as i64 {
                cx - m.rect[2] as i64
            } else {
                0
            };
            let dy = if cy < m.rect[1] as i64 {
                m.rect[1] as i64 - cy
            } else if cy > m.rect[3] as i64 {
                cy - m.rect[3] as i64
            } else {
                0
            };
            dx * dx + dy * dy
        })
        .map(|(i, _)| i)
}

impl Glass {
    pub fn new(blur: f32, dpi: f32) -> Self {
        Self {
            blend_buffer: RefCell::new(None),
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
            slots: Vec::new(),
            monitors: Vec::new(),
            shared: true,
            bg_color: 0,
            desktop_spec: None,
        }
    }
    pub fn wallpaper_accent(&self) -> u32 {
        crate::accent::vivid(self.accent.get())
    }
    /// Dominant wallpaper accent without vibrance boosting.
    pub fn raw_wallpaper_accent(&self) -> u32 {
        self.accent.get()
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn is_shared_wallpaper(&self) -> bool {
        self.shared
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn unique_wallpaper_count(&self) -> usize {
        if !self.slots.is_empty() {
            self.slots.len()
        } else if self.bitmap.is_some() {
            1
        } else {
            0
        }
    }
    unsafe fn slot_index_for_hwnd(&self, hwnd: HWND) -> Option<usize> {
        if self.slots.is_empty() {
            return None;
        }
        if self.shared || self.monitors.is_empty() {
            return Some(0);
        }
        let mut wr = if !hwnd.is_null() {
            crate::fusion::window_rect(hwnd)
        } else {
            zeroed()
        };
        if wr.right <= wr.left || wr.bottom <= wr.top {
            let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = zeroed();
            info.cbSize = size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(hmon, &mut info) != 0 {
                wr = info.rcMonitor;
            }
        }
        let idx = pick_best_monitor_index(&self.monitors, wr)?;
        self.monitors[idx].slot
    }
    pub fn raw_wallpaper_accent_for(&self, hwnd: HWND) -> u32 {
        if self.slots.is_empty() {
            return if !self.monitors.is_empty() && self.bitmap.is_none() {
                self.bg_color
            } else {
                self.accent.get()
            };
        }
        if self.shared
            || (self.slots.len() <= 1 && self.monitors.iter().all(|m| m.slot.is_some()))
        {
            return self.accent.get();
        }
        unsafe {
            match self.slot_index_for_hwnd(hwnd) {
                Some(idx) => self
                    .slots
                    .get(idx)
                    .map_or_else(|| self.accent.get(), |s| s.source.accent),
                None => self.bg_color,
            }
        }
    }
    pub fn wallpaper_accent_for(&self, hwnd: HWND) -> u32 {
        crate::accent::vivid(self.raw_wallpaper_accent_for(hwnd))
    }
    pub fn available(&self) -> bool {
        !self.slots.is_empty() || self.bitmap.is_some()
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
        if let Some(vb) = self
            .desktop_spec
            .as_ref()
            .and_then(|s| s.virtual_bounds())
            .or_else(|| {
                let spec = DesktopWallpaperSpec {
                    unique_sources: Vec::new(),
                    monitors: self.monitors.clone(),
                    shared: self.shared,
                    style: self.style,
                    tile: self.tile,
                    bg_color: self.bg_color,
                };
                spec.virtual_bounds()
            })
        {
            return [
                vb.left,
                vb.top,
                (vb.right - vb.left).max(1),
                (vb.bottom - vb.top).max(1),
            ];
        }
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
    fn virtual_bounds_rect(&self) -> RECT {
        let [vx, vy, vw, vh] = self.desktop_origin_size();
        RECT {
            left: vx,
            top: vy,
            right: vx + vw,
            bottom: vy + vh,
        }
    }
    pub fn monitor_origin_size(&self, hwnd: HWND) -> [i32; 4] {
        unsafe {
            let wr = if !hwnd.is_null() {
                crate::fusion::window_rect(hwnd)
            } else {
                zeroed()
            };
            if !self.monitors.is_empty() {
                let target = if wr.right > wr.left && wr.bottom > wr.top {
                    wr
                } else {
                    let mut info: MONITORINFO = zeroed();
                    info.cbSize = size_of::<MONITORINFO>() as u32;
                    if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info)
                        != 0
                    {
                        info.rcMonitor
                    } else {
                        wr
                    }
                };
                if let Some(idx) = pick_best_monitor_index(&self.monitors, target) {
                    let r = self.monitors[idx].rect;
                    return [r[0], r[1], (r[2] - r[0]).max(1), (r[3] - r[1]).max(1)];
                }
            }
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            let hmon = if wr.right > wr.left && wr.bottom > wr.top {
                MonitorFromRect(&wr, MONITOR_DEFAULTTONEAREST)
            } else {
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
            };
            if GetMonitorInfoW(hmon, &mut monitor) != 0 {
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
        let [vx, vy, vw, vh] = self.monitor_origin_size(hwnd);
        self.gpu_wallpaper_pair_for_rect(
            hwnd,
            RECT {
                left: vx,
                top: vy,
                right: vx + vw,
                bottom: vy + vh,
            },
        )
    }
    pub fn gpu_wallpaper_pair_for_rect(
        &self,
        hwnd: HWND,
        rect: RECT,
    ) -> Option<([i32; 4], Vec<u8>, Option<Vec<u8>>)> {
        unsafe {
            let vw = rect.right - rect.left;
            let vh = rect.bottom - rect.top;
            if vw <= 0 || vh <= 0 || (vw as i64) * (vh as i64) > 32_000_000 {
                return None;
            }
            if !self.available() && self.monitors.is_empty() {
                return None;
            }
            let area = [rect.left, rect.top, vw, vh];
            let mut buffer = crate::compositor::Buffer::plain(vw, vh, 1.0)?;
            let fallback_accent = self.wallpaper_accent_for(hwnd);
            let render_pass = |buffer: &mut crate::compositor::Buffer, want_sharp: bool| {
                if self.style == 0 || self.style == 6 {
                    fill_solid_bg(buffer.dc, vw, vh, self.bg_color);
                } else {
                    fill_solid_bg(buffer.dc, vw, vh, fallback_accent);
                }
                self.draw_rect_layer(buffer.dc, hwnd, vw, vh, rect, want_sharp, false);
                let pixels = buffer.pixels_mut();
                for p in pixels.chunks_exact_mut(4) {
                    p[3] = 255;
                }
                pixels.to_vec()
            };
            let blurred = render_pass(&mut buffer, false);
            let sharp = self
                .has_any_sharp_for_rect(hwnd, rect)
                .then(|| render_pass(&mut buffer, true));
            Some((area, blurred, sharp))
        }
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn gpu_wallpaper_bgra(&self) -> Option<(i32, i32, Vec<u8>)> {
        unsafe {
            let [vx, vy, vw, vh] = self.desktop_origin_size();
            if vw <= 0 || vh <= 0 || (vw as i64) * (vh as i64) > 32_000_000 {
                return None;
            }
            if !self.available() {
                return None;
            }
            let desktop_rect = RECT {
                left: vx,
                top: vy,
                right: vx + vw,
                bottom: vy + vh,
            };
            let want_sharp = self.has_any_sharp_for_rect(GetDesktopWindow(), desktop_rect);
            let mut buffer = crate::compositor::Buffer::plain(vw, vh, 1.0)?;
            if self.style == 0 || self.style == 6 {
                fill_solid_bg(buffer.dc, vw, vh, self.bg_color);
            } else {
                fill_solid_bg(buffer.dc, vw, vh, self.wallpaper_accent());
            }
            self.draw_rect_layer(
                buffer.dc,
                GetDesktopWindow(),
                vw,
                vh,
                desktop_rect,
                want_sharp,
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
        let spec = file_spec(path.to_path_buf());
        glass.path = spec.path.clone();
        glass.style = 2;
        let loaded = Self::load_slot(&spec, blur, dpi, Self::wallpaper_cap())
            .expect("fixture wallpaper");
        glass.apply_legacy_from_slot(&loaded);
        glass.slots = vec![loaded];
        glass.shared = true;
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
    #[cfg(test)]
    pub(crate) unsafe fn fixture_multi_monitor(
        entries: Vec<([i32; 4], Option<PathBuf>)>,
        style: u32,
        tile: bool,
        bg_color: u32,
        blur: f32,
        dpi: f32,
    ) -> Self {
        let mut glass = Self::new(blur, dpi);
        let spec = dedup_monitors(entries, style, tile, bg_color);
        glass.apply_spec_sync(&spec);
        glass
    }
    unsafe fn apply_legacy_from_slot(&mut self, slot: &LoadedWallpaper) {
        self.bitmap = Some(slot.bitmap.clone());
        self.blur_width = slot.blur_width;
        self.blur_height = slot.blur_height;
        self.width = slot.source.width;
        self.height = slot.source.height;
        self.natural = slot.source.natural;
        self.accent.set(slot.source.accent);
        self.source = Some(slot.source.clone());
        self.path = slot.spec.path.clone();
        self.modified = slot.spec.key.1;
    }
    unsafe fn clear_legacy_slot(&mut self) {
        self.bitmap = None;
        self.source = None;
        self.blur_width = 0;
        self.blur_height = 0;
        self.width = 0;
        self.height = 0;
        self.natural = [0.0; 2];
        self.path = PathBuf::new();
        self.modified = None;
        self.accent.set(if !self.monitors.is_empty() {
            self.bg_color
        } else {
            crate::accent::FALLBACK
        });
    }
    fn primary_slot_from_monitors(&self) -> Option<usize> {
        self.monitors
            .iter()
            .find(|m| m.rect[0] <= 0 && m.rect[2] > 0 && m.rect[1] <= 0 && m.rect[3] > 0)
            .or_else(|| self.monitors.first())
            .and_then(|m| m.slot)
            .or_else(|| (!self.slots.is_empty()).then_some(0))
    }
    unsafe fn commit_loaded(
        &mut self,
        raw_spec: DesktopWallpaperSpec,
        effective_monitors: Vec<MonitorWallpaperSpec>,
        slots: Vec<LoadedWallpaper>,
        low_power: bool,
    ) {
        self.style = raw_spec.style;
        self.tile = raw_spec.tile;
        self.bg_color = raw_spec.bg_color;
        self.monitors = effective_monitors;
        self.slots = slots;
        self.shared = if let Some(first) = self.monitors.first() {
            self.slots.len() <= 1 && self.monitors.iter().all(|m| m.slot == first.slot)
        } else {
            self.slots.len() <= 1
        };
        self.low_power = low_power;
        if let Some(slot) = self
            .primary_slot_from_monitors()
            .and_then(|idx| self.slots.get(idx).cloned())
            .or_else(|| self.slots.first().cloned())
        {
            self.apply_legacy_from_slot(&slot);
        } else {
            self.clear_legacy_slot();
        }
        let active_keys: Vec<SourceKey> =
            self.slots.iter().map(|s| s.spec.key.clone()).collect();
        if prune_caches(&active_keys) {
            windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                usize::MAX,
                usize::MAX,
                0,
            );
        }
        self.desktop_spec = Some(raw_spec);
        self.generation += 1;
        self.log_wallpaper_state();
    }
    fn log_wallpaper_state(&self) {
        let mode = if self.style == 22 {
            "span"
        } else if self.slots.is_empty() && !self.monitors.is_empty() {
            "solid"
        } else if self.shared {
            "shared"
        } else {
            "per-monitor"
        };
        crate::logger::info(
            "glass",
            format!(
                "wallpaper mode={mode} unique_sources={} monitors={} style={} tile={} bg={:06x}",
                self.slots.len(),
                self.monitors.len(),
                self.style,
                self.tile,
                self.bg_color,
            ),
        );
    }
    pub unsafe fn set_blur(&mut self, blur: f32, dpi: f32) {
        if (self.blur - blur).abs() < 0.01 && (self.dpi - dpi).abs() < 0.01 {
            return;
        }
        self.blur = blur;
        self.dpi = dpi;
        if !self.slots.is_empty() {
            let mut ok = true;
            for slot in &mut self.slots {
                if let Some((bitmap, bw, bh)) =
                    Self::get_or_build_blurred(&slot.spec, &slot.source, blur, dpi)
                {
                    slot.bitmap = bitmap;
                    slot.blur_width = bw;
                    slot.blur_height = bh;
                } else {
                    ok = false;
                    break;
                }
            }
            if ok {
                if let Some(primary) = self
                    .primary_slot_from_monitors()
                    .and_then(|idx| self.slots.get(idx))
                    .or_else(|| self.slots.first())
                    .cloned()
                {
                    self.apply_legacy_from_slot(&primary);
                }
                self.generation += 1;
                return;
            }
        } else if let Some(source) = &self.source {
            let spec = WallpaperSpec {
                path: self.path.clone(),
                key: (normalize_path_key(&self.path), self.modified, 0),
            };
            if let Some((bitmap, bw, bh)) =
                Self::get_or_build_blurred(&spec, source, blur, dpi)
            {
                self.bitmap = Some(bitmap);
                self.blur_width = bw;
                self.blur_height = bh;
                self.generation += 1;
                return;
            }
        }
        self.desktop_spec = None;
        self.modified = None;
        self.refresh();
    }
    pub(crate) unsafe fn query_desktop_wallpaper() -> Option<DesktopWallpaperSpec> {
        let now = Instant::now();
        if let Some(cached) = LAST_QUERY.with(|c| {
            c.borrow()
                .as_ref()
                .filter(|(t, _)| now.duration_since(*t) < Duration::from_millis(50))
                .map(|(_, spec)| spec.clone())
        }) {
            return Some(cached);
        }
        let resolved = resolve_live_wallpaper_spec()
            .or_else(|| query_desktop_wallpaper_com())
            .or_else(|| query_desktop_wallpaper_legacy());
        if let Some(ref spec) = resolved {
            LAST_QUERY.with(|c| *c.borrow_mut() = Some((now, spec.clone())));
        }
        resolved
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
    fn wallpaper_cap_for_mode(w: i32, h: i32, low_power: bool, span: bool) -> Option<(u32, u32)> {
        if low_power || !span {
            Self::wallpaper_cap_for_screen(w, h, low_power)
        } else if w > 0 && h > 0 {
            Some(((w as u32).clamp(1920, 5120), (h as u32).clamp(1080, 2160)))
        } else {
            None
        }
    }
    unsafe fn cap_for_spec(spec: &DesktopWallpaperSpec, slot_idx: usize, low_power: bool) -> Option<(u32, u32)> {
        if spec.style == 22 {
            if let Some(vb) = spec.virtual_bounds() {
                return Self::wallpaper_cap_for_mode(
                    vb.right - vb.left,
                    vb.bottom - vb.top,
                    low_power,
                    true,
                );
            }
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(GetSystemMetrics(SM_CXSCREEN));
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(GetSystemMetrics(SM_CYSCREEN));
            return Self::wallpaper_cap_for_mode(w, h, low_power, true);
        }
        let mut mw = 0i32;
        let mut mh = 0i32;
        for m in &spec.monitors {
            if m.slot == Some(slot_idx) {
                mw = mw.max(m.rect[2] - m.rect[0]);
                mh = mh.max(m.rect[3] - m.rect[1]);
            }
        }
        if mw <= 0 || mh <= 0 {
            mw = GetSystemMetrics(SM_CXSCREEN);
            mh = GetSystemMetrics(SM_CYSCREEN);
        }
        Self::wallpaper_cap_for_screen(mw, mh, low_power)
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
        let Some(spec) = Self::query_desktop_wallpaper() else {
            return false;
        };
        let low_power = crate::motion::low_power();
        if self.desktop_spec.as_ref() == Some(&spec) && self.low_power == low_power {
            return false;
        }
        let all_cached = WALLPAPER_SOURCES.with(|c| {
            let cache = c.borrow();
            spec.unique_sources.iter().enumerate().all(|(idx, u)| {
                let cap = Self::cap_for_spec(&spec, idx, low_power);
                cache.get(&u.key).is_some_and(|s| s.cap == cap)
            })
        });
        if all_cached {
            return self.refresh();
        }
        let loading_keys: Vec<SourceKey> =
            spec.unique_sources.iter().map(|u| u.key.clone()).collect();
        {
            let mut loading = LOADING_KEY.lock().unwrap_or_else(|e| e.into_inner());
            if loading.as_ref() == Some(&loading_keys) {
                return false;
            }
            *loading = Some(loading_keys);
        }
        let blur = self.blur;
        let dpi = self.dpi;
        let caps: Vec<Option<(u32, u32)>> = (0..spec.unique_sources.len())
            .map(|idx| Self::cap_for_spec(&spec, idx, low_power))
            .collect();
        let notify_addr = notify as usize;
        std::thread::spawn(move || {
            let mut slots = Vec::with_capacity(spec.unique_sources.len());
            let need_sharp = liquid_glass() || !frosted_glass();
            for (idx, (src_spec, cap)) in spec.unique_sources.iter().zip(caps).enumerate() {
                let Some((small, natural)) = Self::decode_small(&src_spec.path, cap) else {
                    crate::logger::warn(
                        "glass",
                        format!("wallpaper decode failed for {}", src_spec.path.display()),
                    );
                    continue;
                };
                let width = small.width() as i32;
                let height = small.height() as i32;
                let palette = imageops::thumbnail(&small, 128, 128);
                let accent = crate::accent::dominant(palette.as_raw());
                let (blurred_bytes, blur_w, blur_h) =
                    Self::prepare_blurred_bytes(&small, blur, dpi, natural[0]);
                let sharp_bytes = need_sharp.then(|| build_sharp_bytes(&small));
                let preview = WallpaperSource::make_preview(&small);
                drop(small);
                slots.push((
                    idx,
                    PendingSlot {
                        spec: src_spec.clone(),
                        width,
                        height,
                        preview,
                        cap,
                        natural,
                        accent,
                        blurred_bytes,
                        blur_w,
                        blur_h,
                        sharp_bytes,
                    },
                ));
            }
            let pending = PendingDesktopWallpaper {
                spec,
                blur,
                dpi,
                low_power,
                slots,
            };
            *LOADING_KEY.lock().unwrap_or_else(|e| e.into_inner()) = None;
            *PENDING_WALLPAPER.lock().unwrap_or_else(|e| e.into_inner()) = Some(pending);
            PostMessageW(notify_addr as HWND, message, 0, 0);
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
        let mut remap = vec![None; pending.spec.unique_sources.len()];
        let mut loaded_slots = Vec::with_capacity(pending.slots.len());
        let mut trim_mem = false;
        for (orig_idx, slot) in pending.slots {
            let source_key = slot.spec.key.clone();
            let w = slot.width;
            let h = slot.height;
            if w > 1280 {
                trim_mem = true;
            }
            let sharp_cell = Cell::new(null_mut());
            let sharp_size_cell = Cell::new((0, 0));
            if let Some((ref sharp_bytes, sw, sh)) = slot.sharp_bytes {
                if let Some(sharp_bmp) = dib(sw, sh, sharp_bytes) {
                    sharp_cell.set(sharp_bmp);
                    sharp_size_cell.set((sw, sh));
                }
            }
            let source = Rc::new(WallpaperSource {
                path: slot.spec.path.clone(),
                cap: slot.cap,
                small: RefCell::new(None),
                preview: slot.preview,
                sharp: sharp_cell,
                sharp_size: sharp_size_cell,
                width: w,
                height: h,
                natural: slot.natural,
                accent: slot.accent,
            });
            WALLPAPER_SOURCES.with(|cache| {
                let mut cache = cache.borrow_mut();
                cache.insert(source_key.clone(), source.clone());
            });
            BLURRED_CACHE.with(|cache| {
                cache.borrow_mut().retain(|(k, _, _), _| k != &source_key);
            });
            let Some(bitmap) = dib(slot.blur_w, slot.blur_h, &slot.blurred_bytes) else {
                source.release_large();
                continue;
            };
            let owned = Rc::new(OwnedBitmap(bitmap));
            let blur_key: BlurKey = (
                source_key,
                (pending.blur * 100.).round() as i32,
                (pending.dpi * 100.).round() as i32,
            );
            BLURRED_CACHE.with(|cache| {
                cache
                    .borrow_mut()
                    .insert(blur_key, (owned.clone(), slot.blur_w, slot.blur_h));
            });
            let (final_bitmap, bw, bh) = if (self.blur - pending.blur).abs() < 0.01
                && (self.dpi - pending.dpi).abs() < 0.01
            {
                (owned, slot.blur_w, slot.blur_h)
            } else if let Some(built) =
                Self::get_or_build_blurred(&slot.spec, &source, self.blur, self.dpi)
            {
                built
            } else {
                (owned, slot.blur_w, slot.blur_h)
            };
            source.release_large();
            if let Some(target) = remap.get_mut(orig_idx) {
                *target = Some(loaded_slots.len());
            }
            loaded_slots.push(LoadedWallpaper {
                spec: slot.spec,
                bitmap: final_bitmap,
                source,
                blur_width: bw,
                blur_height: bh,
            });
        }
        if trim_mem {
            windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx(
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                usize::MAX,
                usize::MAX,
                0,
            );
        }
        let effective_monitors: Vec<MonitorWallpaperSpec> = pending
            .spec
            .monitors
            .iter()
            .map(|m| MonitorWallpaperSpec {
                rect: m.rect,
                slot: m.slot.and_then(|old| remap.get(old).copied().flatten()),
            })
            .collect();
        self.commit_loaded(pending.spec, effective_monitors, loaded_slots, pending.low_power);
        true
    }
    /// Windows writes into our owned UTF-16 buffer. We only read the returned
    /// wallpaper state and files; no registry writes.
    pub unsafe fn refresh(&mut self) -> bool {
        let Some(spec) = Self::query_desktop_wallpaper() else {
            return false;
        };
        if self.desktop_spec.as_ref() == Some(&spec)
            && self.low_power == crate::motion::low_power()
        {
            return false;
        }
        let loading_keys: Vec<SourceKey> =
            spec.unique_sources.iter().map(|u| u.key.clone()).collect();
        if LOADING_KEY
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            == Some(&loading_keys)
        {
            return false;
        }
        self.apply_spec_sync(&spec);
        true
    }
    unsafe fn apply_spec_sync(&mut self, spec: &DesktopWallpaperSpec) {
        let low_power = crate::motion::low_power();
        let mut remap = vec![None; spec.unique_sources.len()];
        let mut loaded_slots = Vec::with_capacity(spec.unique_sources.len());
        for (idx, src_spec) in spec.unique_sources.iter().enumerate() {
            let cap = Self::cap_for_spec(spec, idx, low_power);
            if let Some(loaded) = Self::load_slot(src_spec, self.blur, self.dpi, cap) {
                remap[idx] = Some(loaded_slots.len());
                loaded_slots.push(loaded);
            } else {
                crate::logger::warn(
                    "glass",
                    format!("wallpaper decode failed for {}", src_spec.path.display()),
                );
            }
        }
        let effective_monitors: Vec<MonitorWallpaperSpec> = spec
            .monitors
            .iter()
            .map(|m| MonitorWallpaperSpec {
                rect: m.rect,
                slot: m.slot.and_then(|old| remap.get(old).copied().flatten()),
            })
            .collect();
        self.commit_loaded(spec.clone(), effective_monitors, loaded_slots, low_power);
    }
    unsafe fn get_or_build_blurred(
        spec: &WallpaperSpec,
        source: &WallpaperSource,
        blur: f32,
        dpi: f32,
    ) -> Option<(Rc<OwnedBitmap>, i32, i32)> {
        let key: BlurKey = (
            spec.key.clone(),
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
            let (decoded, _) =
                Self::decode_small(&source.path, source.cap.or_else(|| Self::wallpaper_cap()))?;
            Self::build_blurred(&decoded, blur, dpi, source.natural[0])?
        };
        let owned = Rc::new(OwnedBitmap(bitmap));
        BLURRED_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= 12 {
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
        let sigma_src = (blur * dpi * small.width() as f32 / natural_w.max(1.0)).clamp(0.0, 40.0);
        let (work, sigma) = if sigma_src > 0.05 && (small.width() > 320 || small.height() > 180) {
            let scale = (sigma_src / 2.2).clamp(1.0, 6.0);
            let min_w = 256u32.min(small.width());
            let min_h = 144u32.min(small.height());
            let target_w = ((small.width() as f32 / scale).round() as u32).clamp(min_w, small.width());
            let target_h =
                ((small.height() as f32 / scale).round() as u32).clamp(min_h, small.height());
            if target_w < small.width() || target_h < small.height() {
                let resized =
                    imageops::resize(small, target_w, target_h, imageops::FilterType::Triangle);
                let scaled_sigma =
                    (sigma_src * (resized.width() as f32 / small.width().max(1) as f32)).clamp(0.0, 40.0);
                (std::borrow::Cow::Owned(resized), scaled_sigma)
            } else {
                (std::borrow::Cow::Borrowed(small), sigma_src)
            }
        } else {
            (std::borrow::Cow::Borrowed(small), sigma_src)
        };
        let mut blurred = if sigma <= 0.05 {
            work.into_owned()
        } else if sigma < 3.5 {
            imageops::blur(&*work, sigma)
        } else {
            imageops::fast_blur(&*work, sigma)
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
    unsafe fn load_slot(
        spec: &WallpaperSpec,
        blur: f32,
        dpi: f32,
        cap: Option<(u32, u32)>,
    ) -> Option<LoadedWallpaper> {
        let source = if let Some(cached) = WALLPAPER_SOURCES.with(|cache| {
            cache
                .borrow()
                .get(&spec.key)
                .filter(|s| s.cap == cap)
                .cloned()
        }) {
            cached
        } else {
            let (small, natural) = Self::decode_small(&spec.path, cap)?;
            let palette = imageops::thumbnail(&small, 128, 128);
            let accent = crate::accent::dominant(palette.as_raw());
            if std::env::var_os("LUMEN_TRACE_FRAMES").is_some() {
                eprintln!("native-wallpaper-palette colorref={:06x}", accent);
            }
            let w = small.width() as i32;
            let h = small.height() as i32;
            let preview = WallpaperSource::make_preview(&small);
            let source = Rc::new(WallpaperSource {
                path: spec.path.clone(),
                cap,
                small: RefCell::new(Some(small)),
                preview,
                sharp: Cell::new(null_mut()),
                sharp_size: Cell::new((0, 0)),
                width: w,
                height: h,
                natural,
                accent,
            });
            BLURRED_CACHE.with(|cache| {
                cache.borrow_mut().retain(|(k, _, _), _| k != &spec.key);
            });
            WALLPAPER_SOURCES.with(|cache| {
                let mut cache = cache.borrow_mut();
                if cache.len() >= 8 {
                    cache.clear();
                }
                cache.insert(spec.key.clone(), source.clone());
            });
            source
        };
        let (bitmap, blur_width, blur_height) =
            Self::get_or_build_blurred(spec, &source, blur, dpi)?;
        source.release_large();
        Some(LoadedWallpaper {
            spec: spec.clone(),
            bitmap,
            source,
            blur_width,
            blur_height,
        })
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
        if !self.available() && self.monitors.is_empty() {
            return false;
        }
        if self.has_any_sharp_for_rect(hwnd, window) {
            self.draw_rect_layer(dc, hwnd, width, height, window, true, low_moving);
            let mut scratch = self.blend_buffer.borrow_mut();
            if scratch
                .as_ref()
                .is_none_or(|(owner, b)| *owner != hwnd || b.width != width || b.height != height)
            {
                *scratch = crate::compositor::Buffer::plain(width, height, 1.).map(|b| (hwnd, b));
            }
            if let Some((_, buffer)) = scratch.as_mut() {
                if self.style == 0 || self.style == 6 {
                    fill_solid_bg(buffer.dc, width, height, self.bg_color);
                }
                self.draw_rect_layer(buffer.dc, hwnd, width, height, window, false, moving);
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
            if !moving {
                scratch.take();
            }
            true
        } else {
            self.draw_rect_layer(dc, hwnd, width, height, window, false, moving);
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
        if (!self.available() && self.monitors.is_empty())
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
            fill_solid_bg(dc, width, height, self.bg_color);
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
        if (!self.available() && self.monitors.is_empty())
            || width <= 0
            || height <= 0
            || width as i64 * height as i64 > 32_000_000
        {
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
            fill_solid_bg(dc, width, height, self.bg_color);
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
            self.draw_rect_layer(dc, hwnd, width, height, window, false, moving);
            self.finish_liquid_glass(dc, hwnd, width, height, window, apply_liquid, amount, None);
            return;
        }
        if self.has_any_sharp_for_rect(hwnd, window) {
            self.draw_rect_layer(dc, hwnd, width, height, window, true, low_moving);
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
            fill_solid_bg(buffer.dc, width, height, self.bg_color);
            self.draw_rect_layer(buffer.dc, hwnd, width, height, window, false, moving);
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
        let placed = self.build_placed_surface(hwnd, window, false)?;
        let win_w = (window.right - window.left).max(1) as f32;
        let win_h = (window.bottom - window.top).max(1) as f32;
        let mut tile = Vec::with_capacity(tw * th);
        for ty in 0..th {
            let ly = (ty as f32 + 0.5) / th as f32 * win_h;
            for tx in 0..tw {
                let lx = (tx as f32 + 0.5) / tw as f32 * win_w;
                let b = placed
                    .sample_channel(lx, ly, 0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                let g = placed
                    .sample_channel(lx, ly, 1)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                let r = placed
                    .sample_channel(lx, ly, 2)
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
    unsafe fn build_placed_surface(
        &self,
        hwnd: HWND,
        window: RECT,
        want_sharp: bool,
    ) -> Option<PlacedSurface<'_>> {
        if !self.available() && self.monitors.is_empty() {
            return None;
        }
        if want_sharp && !self.has_any_sharp_for_rect(hwnd, window) {
            return None;
        }
        let bg_bgr = colorref_to_bgr_f32(self.bg_color);
        let shift = if liquid_glass() {
            6.0 * self.dpi.max(1.0)
        } else {
            0.0
        };
        let slot_placement = |slot_idx: Option<usize>, canvas: RECT| -> Option<SharpPlacement<'_>> {
            let idx = slot_idx?;
            let (bmp, natural) = if !self.slots.is_empty() {
                let slot = self.slots.get(idx)?;
                let bmp = if want_sharp {
                    let sharp = slot.source.sharp_bitmap();
                    if !sharp.is_null() {
                        sharp
                    } else {
                        slot.bitmap.0
                    }
                } else {
                    slot.bitmap.0
                };
                (bmp, slot.source.natural)
            } else {
                let bmp = if want_sharp {
                    self.source
                        .as_ref()
                        .map(|s| s.sharp_bitmap())
                        .filter(|b| !b.is_null())?
                } else {
                    self.bitmap.as_ref()?.0
                };
                (bmp, self.natural)
            };
            let (buf, bw, bh, stride) = self.dib_slice(bmp)?;
            let pl = compute_placement(canvas, natural, window, self.style, self.tile, shift);
            Some(SharpPlacement {
                buf,
                w: bw,
                h: bh,
                stride,
                left_f: pl.left_f,
                top_f: pl.top_f,
                dw: pl.dw,
                dh: pl.dh,
                tile: self.tile,
                style: self.style,
                bg_bgr,
            })
        };
        let mons = self.active_monitors_for_rect(hwnd, window);
        if self.style == 22 {
            let canvas = self.virtual_bounds_rect();
            let slot_idx = mons
                .iter()
                .find_map(|(_, s)| *s)
                .or_else(|| (!self.slots.is_empty() || self.bitmap.is_some()).then_some(0));
            return Some(
                slot_placement(slot_idx, canvas)
                    .map(PlacedSurface::Single)
                    .unwrap_or(PlacedSurface::Solid(bg_bgr)),
            );
        }
        if self.tile && self.shared && !self.monitors.is_empty() {
            let canvas = self.shared_tile_canvas(&mons);
            let slot_idx = mons
                .iter()
                .find_map(|(_, s)| *s)
                .or_else(|| (!self.slots.is_empty() || self.bitmap.is_some()).then_some(0));
            return Some(
                slot_placement(slot_idx, canvas)
                    .map(PlacedSurface::Single)
                    .unwrap_or(PlacedSurface::Solid(bg_bgr)),
            );
        }
        if mons.len() <= 1 {
            let (mon_rect, slot_idx) = mons.into_iter().next()?;
            return Some(
                slot_placement(slot_idx, mon_rect)
                    .map(PlacedSurface::Single)
                    .unwrap_or(PlacedSurface::Solid(bg_bgr)),
            );
        }
        let regions = mons
            .into_iter()
            .map(|(mon_rect, slot_idx)| (mon_rect, slot_placement(slot_idx, mon_rect)))
            .collect();
        Some(PlacedSurface::Multi {
            window,
            regions,
            bg_bgr,
        })
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
        let sharp = self.build_placed_surface(hwnd, window, true);
        let blur = self.build_placed_surface(hwnd, window, false);
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
    #[cfg_attr(not(test), allow(dead_code))]
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
        let Some(blur_placed) = self.build_placed_surface(hwnd, window, false) else {
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
        let sharp_placed = self.build_placed_surface(hwnd, window, true);
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

            let fb = blur_placed.sample_disp_2d(fx, fy, nx, ny, disp, ca, dpi, 0);
            let fg = blur_placed.sample_disp_2d(fx, fy, nx, ny, disp, 0.0, dpi, 1);
            let fr = blur_placed.sample_disp_2d(fx, fy, nx, ny, disp, -ca, dpi, 2);

            let local_frost = frost_at(x, y).clamp(0.0, 1.0);
            let (fb, fg, fr) = if local_frost < 0.999 {
                if let Some(ref sp) = sharp_placed {
                    let sb0 = sp.sample_disp_2d(fx, fy, nx, ny, disp, ca, dpi, 0);
                    let sg0 = sp.sample_disp_2d(fx, fy, nx, ny, disp, 0.0, dpi, 1);
                    let sr0 = sp.sample_disp_2d(fx, fy, nx, ny, disp, -ca, dpi, 2);
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
                    let sb = sp.sample_disp_2d(fx, fy, nx, ny, disp, ca_s, dpi, 0);
                    let sg = sp.sample_disp_2d(fx, fy, nx, ny, disp, 0.0, dpi, 1);
                    let sr = sp.sample_disp_2d(fx, fy, nx, ny, disp, -ca_s, dpi, 2);
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
        blur_placed: Option<&PlacedSurface<'_>>,
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
        let moving = fusion_owner
            .is_some_and(|(hwnd, _)| crate::fusion::is_moving(hwnd) || is_resizing(hwnd));
        LIQUID_SRC.with(|src_cell| {
            let mut src_guard = src_cell.borrow_mut();
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
            if !moving {
                *src_guard = Vec::new();
            }
        });
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
    bg_bgr: [f32; 3],
}

enum PlacedSurface<'a> {
    Single(SharpPlacement<'a>),
    Solid([f32; 3]),
    Multi {
        window: RECT,
        regions: Vec<(RECT, Option<SharpPlacement<'a>>)>,
        bg_bgr: [f32; 3],
    },
}

#[inline]
fn find_region<'a, 'b>(
    regions: &'b [(RECT, Option<SharpPlacement<'a>>)],
    gx: f32,
    gy: f32,
) -> Option<&'b (RECT, Option<SharpPlacement<'a>>)> {
    if let Some(exact) = regions.iter().find(|(r, _)| {
        gx >= r.left as f32 && gx < r.right as f32 && gy >= r.top as f32 && gy < r.bottom as f32
    }) {
        return Some(exact);
    }
    regions.iter().min_by(|(a, _), (b, _)| {
        let dist = |r: &RECT| {
            let dx = if gx < r.left as f32 {
                r.left as f32 - gx
            } else if gx >= r.right as f32 {
                gx - (r.right as f32 - 1.0)
            } else {
                0.0
            };
            let dy = if gy < r.top as f32 {
                r.top as f32 - gy
            } else if gy >= r.bottom as f32 {
                gy - (r.bottom as f32 - 1.0)
            } else {
                0.0
            };
            dx * dx + dy * dy
        };
        dist(a).total_cmp(&dist(b))
    })
}

impl<'a> PlacedSurface<'a> {
    #[inline]
    fn sample_channel(&self, lx: f32, ly: f32, ch: usize) -> f32 {
        match self {
            Self::Single(p) => sample_placed_channel(p, lx, ly, ch),
            Self::Solid(bgr) => bgr[ch],
            Self::Multi {
                window,
                regions,
                bg_bgr,
            } => {
                let gx = window.left as f32 + lx;
                let gy = window.top as f32 + ly;
                match find_region(regions, gx, gy) {
                    Some((_, Some(p))) => sample_placed_channel(p, lx, ly, ch),
                    _ => bg_bgr[ch],
                }
            }
        }
    }
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn sample_disp_2d(
        &self,
        bx: f32,
        by: f32,
        nx: f32,
        ny: f32,
        disp: f32,
        signed_ca: f32,
        dpi: f32,
        ch: usize,
    ) -> f32 {
        match self {
            Self::Single(p) => {
                sample_placed_disp_2d(p, bx, by, nx, ny, disp, signed_ca, dpi, ch)
            }
            Self::Solid(bgr) => bgr[ch],
            Self::Multi { .. } => {
                sample_refracted_channel(bx, by, nx, ny, disp, signed_ca, dpi, [1., 1.], |x, y| {
                    self.sample_channel(x, y, ch)
                })
            }
        }
    }
}

enum SharpInput<'a> {
    None,
    #[cfg_attr(not(test), allow(dead_code))]
    Buffer {
        buf: &'a [u8],
        w: usize,
        h: usize,
        pad_f: f32,
    },
    Placed(&'a PlacedSurface<'a>),
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
        return p.bg_bgr[ch];
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
    blur_placed: Option<&PlacedSurface<'_>>,
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
                let bb = bp.sample_disp_2d(fx, fy, nx, ny, disp, ca, dpi, ch_b);
                let bg = bp.sample_disp_2d(fx, fy, nx, ny, disp, 0.0, dpi, ch_g);
                let br = bp.sample_disp_2d(fx, fy, nx, ny, disp, -ca, dpi, ch_r);
                if local_blur < 0.999 {
                    if let Some(sp) = sharp_placed {
                        let sb = sp.sample_disp_2d(fx, fy, nx, ny, disp, ca, dpi, ch_b);
                        let sg = sp.sample_disp_2d(fx, fy, nx, ny, disp, 0.0, dpi, ch_g);
                        let sr = sp.sample_disp_2d(fx, fy, nx, ny, disp, -ca, dpi, ch_r);
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
                        let sb = p.sample_disp_2d(fx, fy, nx, ny, disp, ca_s, dpi, ch_b);
                        let sg = p.sample_disp_2d(fx, fy, nx, ny, disp, 0.0, dpi, ch_g);
                        let sr = p.sample_disp_2d(fx, fy, nx, ny, disp, -ca_s, dpi, ch_r);
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
struct Placement {
    left_f: f32,
    top_f: f32,
    dw: f32,
    dh: f32,
    shift: f32,
}

fn compute_placement(
    canvas: RECT,
    natural: [f32; 2],
    window: RECT,
    style: u32,
    tile: bool,
    shift: f32,
) -> Placement {
    let aw = (canvas.right - canvas.left).max(1) as f32;
    let ah = (canvas.bottom - canvas.top).max(1) as f32;
    let nw = natural[0].max(1.0);
    let nh = natural[1].max(1.0);
    let (dw, dh) = if style == 2 {
        (aw, ah)
    } else if style == 0 || tile {
        (nw, nh)
    } else {
        let ratio = if style == 6 {
            (aw / nw).min(ah / nh)
        } else {
            (aw / nw).max(ah / nh)
        };
        (nw * ratio, nh * ratio)
    };
    let left_f = canvas.left as f32
        + if tile { 0.0 } else { (aw - dw) / 2.0 }
        - window.left as f32
        + shift;
    let top_f = canvas.top as f32
        + vertical_offset(ah, dh, style, tile)
        - window.top as f32
        + shift;
    Placement {
        left_f,
        top_f,
        dw,
        dh,
        shift,
    }
}

impl Glass {
    unsafe fn active_monitors_for_rect(&self, hwnd: HWND, window: RECT) -> Vec<(RECT, Option<usize>)> {
        if !self.monitors.is_empty() {
            let mut hits: Vec<(RECT, Option<usize>)> = self
                .monitors
                .iter()
                .filter_map(|m| {
                    let mr = RECT {
                        left: m.rect[0],
                        top: m.rect[1],
                        right: m.rect[2],
                        bottom: m.rect[3],
                    };
                    rect_intersect(mr, window).map(|_| (mr, m.slot))
                })
                .collect();
            if hits.is_empty() {
                if let Some(idx) = pick_best_monitor_index(&self.monitors, window) {
                    let m = &self.monitors[idx];
                    hits.push((
                        RECT {
                            left: m.rect[0],
                            top: m.rect[1],
                            right: m.rect[2],
                            bottom: m.rect[3],
                        },
                        m.slot,
                    ));
                }
            }
            return hits;
        }
        let mut monitor: MONITORINFO = zeroed();
        monitor.cbSize = size_of::<MONITORINFO>() as u32;
        let hmon = if window.right > window.left && window.bottom > window.top {
            MonitorFromRect(&window, MONITOR_DEFAULTTONEAREST)
        } else {
            MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST)
        };
        let rect = if GetMonitorInfoW(hmon, &mut monitor) != 0 {
            monitor.rcMonitor
        } else {
            self.virtual_bounds_rect()
        };
        let slot = (!self.slots.is_empty() || self.bitmap.is_some()).then_some(0);
        vec![(rect, slot)]
    }
    fn shared_tile_canvas(&self, mons: &[(RECT, Option<usize>)]) -> RECT {
        let base = mons.first().map(|(r, _)| *r).unwrap_or(RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        });
        RECT {
            left: 0,
            top: 0,
            right: (base.right - base.left).max(1),
            bottom: (base.bottom - base.top).max(1),
        }
    }
    unsafe fn slot_resources(
        &self,
        slot_idx: Option<usize>,
        want_sharp: bool,
    ) -> Option<(HBITMAP, (i32, i32), [f32; 2])> {
        let idx = slot_idx?;
        if !self.slots.is_empty() {
            let slot = self.slots.get(idx)?;
            let (bmp, size) = if want_sharp {
                let sharp = slot.source.sharp_bitmap();
                if !sharp.is_null() {
                    (sharp, slot.source.sharp_dimensions())
                } else {
                    (
                        slot.bitmap.0,
                        (slot.blur_width.max(1), slot.blur_height.max(1)),
                    )
                }
            } else {
                (
                    slot.bitmap.0,
                    (slot.blur_width.max(1), slot.blur_height.max(1)),
                )
            };
            Some((bmp, size, slot.source.natural))
        } else if want_sharp {
            let src = self.source.as_ref()?;
            let sharp = src.sharp_bitmap();
            if sharp.is_null() {
                return None;
            }
            Some((sharp, src.sharp_dimensions(), self.natural))
        } else {
            let bmp = self.bitmap.as_ref()?.0;
            Some((bmp, self.blur_size(), self.natural))
        }
    }
    unsafe fn has_any_sharp_for_rect(&self, hwnd: HWND, window: RECT) -> bool {
        if !self.slots.is_empty() {
            let mons = self.active_monitors_for_rect(hwnd, window);
            mons.iter().any(|(_, slot_idx)| {
                slot_idx
                    .and_then(|idx| self.slots.get(idx))
                    .is_some_and(|slot| !slot.source.sharp_bitmap().is_null())
            })
        } else {
            self.source
                .as_ref()
                .is_some_and(|s| !s.sharp_bitmap().is_null())
        }
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_rect_layer(
        &self,
        dc: HDC,
        hwnd: HWND,
        width: i32,
        height: i32,
        window: RECT,
        want_sharp: bool,
        fast: bool,
    ) {
        let shift = if liquid_glass() {
            6.0 * self.dpi.max(1.0)
        } else {
            0.0
        };
        let mons = self.active_monitors_for_rect(hwnd, window);
        if self.style == 22 || (self.tile && self.shared && !self.monitors.is_empty()) {
            let canvas = if self.style == 22 {
                self.virtual_bounds_rect()
            } else {
                self.shared_tile_canvas(&mons)
            };
            let slot_idx = mons
                .iter()
                .find_map(|(_, s)| *s)
                .or_else(|| (!self.slots.is_empty() || self.bitmap.is_some()).then_some(0));
            let Some((bmp, src_size, natural)) = self.slot_resources(slot_idx, want_sharp) else {
                fill_solid_bg(dc, width, height, self.bg_color);
                return;
            };
            self.draw_layer_uncached_at(
                dc, width, height, window, canvas, natural, bmp, src_size, shift, fast,
            );
            return;
        }
        if mons.len() <= 1 {
            let Some((m, slot_idx)) = mons.into_iter().next() else {
                return;
            };
            let Some((bmp, src_size, natural)) = self.slot_resources(slot_idx, want_sharp) else {
                fill_solid_bg(dc, width, height, self.bg_color);
                return;
            };
            self.draw_layer_uncached_at(
                dc, width, height, window, m, natural, bmp, src_size, shift, fast,
            );
            return;
        }
        fill_solid_bg(dc, width, height, self.bg_color);
        for (m, slot_idx) in mons {
            let Some(inter) = rect_intersect(m, window) else {
                continue;
            };
            let dx = inter.left - window.left;
            let dy = inter.top - window.top;
            let iw = inter.right - inter.left;
            let ih = inter.bottom - inter.top;
            if let Some((bmp, src_size, natural)) = self.slot_resources(slot_idx, want_sharp) {
                let saved = SaveDC(dc);
                IntersectClipRect(dc, dx, dy, dx + iw, dy + ih);
                self.draw_layer_uncached_at(
                    dc, width, height, window, m, natural, bmp, src_size, shift, fast,
                );
                RestoreDC(dc, saved);
            }
        }
    }
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
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
        let shift = if liquid_glass() {
            6.0 * self.dpi.max(1.0)
        } else {
            0.0
        };
        let canvas = if self.style == 22 {
            self.virtual_bounds_rect()
        } else {
            let mut monitor: MONITORINFO = zeroed();
            monitor.cbSize = size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(
                MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            );
            monitor.rcMonitor
        };
        self.draw_layer_uncached_at(
            dc,
            width,
            height,
            window,
            canvas,
            self.natural,
            bitmap,
            src_size,
            shift,
            fast,
        );
    }

    unsafe fn blit_placed_bilinear(
        &self,
        dc: HDC,
        width: i32,
        height: i32,
        bitmap: HBITMAP,
        pl: &Placement,
    ) -> bool {
        let hbmp = GetCurrentObject(dc, OBJ_BITMAP as u32);
        if hbmp.is_null() {
            return false;
        }
        let mut ds: DIBSECTION = zeroed();
        if GetObjectW(hbmp, size_of::<DIBSECTION>() as i32, &mut ds as *mut _ as _)
            < size_of::<BITMAP>() as i32
        {
            return false;
        }
        if ds.dsBm.bmBits.is_null()
            || ds.dsBm.bmBitsPixel != 32
            || ds.dsBmih.biHeight >= 0
            || ds.dsBm.bmWidth < width
            || ds.dsBm.bmHeight < height
        {
            return false;
        }
        let mut clip: RECT = zeroed();
        match GetClipBox(dc, &mut clip) {
            1 => return true,  // NULLREGION
            2 => {}            // SIMPLEREGION
            _ => return false, // COMPLEXREGION or ERROR
        }
        let Some((src, sw, sh, src_stride)) = self.dib_slice(bitmap) else {
            return false;
        };
        if sw == 0 || sh == 0 {
            return false;
        }
        let mut x0 = clip.left.max(0);
        let mut y0 = clip.top.max(0);
        let mut x1 = clip.right.min(width);
        let mut y1 = clip.bottom.min(height);
        if !self.tile && (self.style == 0 || self.style == 6) {
            x0 = x0.max(pl.left_f.round() as i32);
            y0 = y0.max(pl.top_f.round() as i32);
            x1 = x1.min((pl.left_f + pl.dw).round() as i32);
            y1 = y1.min((pl.top_f + pl.dh).round() as i32);
        }
        if x0 >= x1 || y0 >= y1 {
            return true;
        }
        GdiFlush();
        let dst_stride = ds.dsBm.bmWidthBytes.max(width * 4) as usize;
        let dst = std::slice::from_raw_parts_mut(
            ds.dsBm.bmBits as *mut u8,
            height as usize * dst_stride,
        );
        let dw = pl.dw.max(1.0);
        let dh = pl.dh.max(1.0);
        let tile = self.tile;
        let build_axis = |start: i32,
                          end: i32,
                          origin: f32,
                          span: f32,
                          src_dim: usize,
                          out: &mut Vec<(usize, usize, u32)>| {
            out.clear();
            let max_s = (src_dim - 1) as f32;
            let inv_span = 1.0 / span;
            let dim_f = src_dim as f32;
            let dim_i = src_dim as i32;
            for p in start..end {
                let u = ((p as f32 + 0.5) - origin) * inv_span;
                if tile {
                    let s = u.rem_euclid(1.0) * dim_f - 0.5;
                    let f = s.floor();
                    let i0 = (f as i32).rem_euclid(dim_i) as usize;
                    let i1 = (i0 + 1) % src_dim;
                    let frac = ((s - f) * 256.0).round() as u32;
                    out.push((i0, i1, frac.min(256)));
                } else {
                    let s = (u * dim_f - 0.5).clamp(0.0, max_s);
                    let i0 = s as usize;
                    let i1 = (i0 + 1).min(src_dim - 1);
                    let frac = ((s - i0 as f32) * 256.0).round() as u32;
                    out.push((i0, i1, frac.min(256)));
                }
            }
        };
        BLIT_XS.with(|xs_cell| {
            BLIT_YS.with(|ys_cell| {
                let mut xs = xs_cell.borrow_mut();
                let mut ys = ys_cell.borrow_mut();
                build_axis(x0, x1, pl.left_f, dw, sw, &mut xs);
                build_axis(y0, y1, pl.top_f, dh, sh, &mut ys);
                let x_byte_off = x0 as usize * 4;
                let row_bytes = (x1 - x0) as usize * 4;
                for (yy, &(sy0, sy1, fy)) in ys.iter().enumerate() {
                    let wy0 = 256 - fy;
                    let wy1 = fy;
                    let srow0 = &src[sy0 * src_stride..sy0 * src_stride + sw * 4];
                    let srow1 = &src[sy1 * src_stride..sy1 * src_stride + sw * 4];
                    let d_off = (y0 as usize + yy) * dst_stride + x_byte_off;
                    let drow = &mut dst[d_off..d_off + row_bytes];
                    for (dpx, &(sx0, sx1, fx)) in drow.chunks_exact_mut(4).zip(xs.iter()) {
                        let wx0 = 256 - fx;
                        let w00 = wx0 * wy0;
                        let w10 = fx * wy0;
                        let w01 = wx0 * wy1;
                        let w11 = fx * wy1;
                        let i00 = sx0 * 4;
                        let i10 = sx1 * 4;
                        dpx[0] = ((srow0[i00] as u32 * w00
                            + srow0[i10] as u32 * w10
                            + srow1[i00] as u32 * w01
                            + srow1[i10] as u32 * w11
                            + 32768)
                            >> 16) as u8;
                        dpx[1] = ((srow0[i00 + 1] as u32 * w00
                            + srow0[i10 + 1] as u32 * w10
                            + srow1[i00 + 1] as u32 * w01
                            + srow1[i10 + 1] as u32 * w11
                            + 32768)
                            >> 16) as u8;
                        dpx[2] = ((srow0[i00 + 2] as u32 * w00
                            + srow0[i10 + 2] as u32 * w10
                            + srow1[i00 + 2] as u32 * w01
                            + srow1[i10 + 2] as u32 * w11
                            + 32768)
                            >> 16) as u8;
                        dpx[3] = 255;
                    }
                }
            });
        });
        true
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw_layer_uncached_at(
        &self,
        dc: HDC,
        width: i32,
        height: i32,
        window: RECT,
        canvas: RECT,
        natural: [f32; 2],
        bitmap: HBITMAP,
        src_size: (i32, i32),
        shift: f32,
        fast: bool,
    ) {
        if bitmap.is_null() || width <= 0 || height <= 0 {
            return;
        }
        let pl = compute_placement(canvas, natural, window, self.style, self.tile, shift);
        if self.blit_placed_bilinear(dc, width, height, bitmap, &pl) {
            return;
        }
        let left = pl.left_f.round() as i32;
        let top = pl.top_f.round() as i32;
        let source = CreateCompatibleDC(dc);
        let previous = SelectObject(source, bitmap);
        SetStretchBltMode(dc, if fast { COLORONCOLOR } else { HALFTONE });
        if !fast {
            SetBrushOrgEx(dc, 0, 0, null_mut());
        }
        if self.tile {
            let step_x = (pl.dw.round() as i32).max(1);
            let step_y = (pl.dh.round() as i32).max(1);
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
            let right = (pl.left_f + pl.dw).round() as i32;
            let bottom = (pl.top_f + pl.dh).round() as i32;
            if pl.shift > 0.0 && self.style != 6 && self.style != 0 {
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
fn file_spec(path: PathBuf) -> WallpaperSpec {
    let meta = std::fs::metadata(&path).ok();
    let modified = meta.as_ref().and_then(|m| m.modified().ok());
    let size = meta.as_ref().map_or(0, |m| m.len());
    let key = (normalize_path_key(&path), modified, size);
    WallpaperSpec { path, key }
}

fn dedup_monitors(
    mut entries: Vec<([i32; 4], Option<PathBuf>)>,
    style: u32,
    tile: bool,
    bg_color: u32,
) -> DesktopWallpaperSpec {
    entries.sort_by_key(|(rect, _)| *rect);
    let mut unique_sources: Vec<WallpaperSpec> = Vec::new();
    let mut monitors = Vec::with_capacity(entries.len());
    for (rect, path_opt) in entries {
        let slot = path_opt.map(|path| {
            let spec = file_spec(path);
            if let Some(existing) = unique_sources.iter().position(|u| u.key == spec.key) {
                existing
            } else {
                unique_sources.push(spec);
                unique_sources.len() - 1
            }
        });
        monitors.push(MonitorWallpaperSpec { rect, slot });
    }
    let shared = if let Some(first) = monitors.first() {
        unique_sources.len() <= 1 && monitors.iter().all(|m| m.slot == first.slot)
    } else {
        unique_sources.len() <= 1
    };
    DesktopWallpaperSpec {
        unique_sources,
        monitors,
        shared,
        style,
        tile,
        bg_color,
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

fn parse_utf16le_at(raw: &[u8], offset: usize) -> Option<PathBuf> {
    if raw.len() < offset + 2 {
        return None;
    }
    let words: Vec<u16> = raw[offset..]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&w| w != 0)
        .collect();
    if words.is_empty() {
        return None;
    }
    Some(PathBuf::from(String::from_utf16_lossy(&words)))
}

fn parse_transcoded_cache_path(raw: &[u8]) -> Option<PathBuf> {
    parse_utf16le_at(raw, 24)
}

fn parse_transcoded_meta_path(raw: &[u8]) -> Option<PathBuf> {
    parse_utf16le_at(raw, 20)
}

fn is_decodable_image_file(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let mut hdr = [0u8; 12];
    let Ok(n) = std::io::Read::read(&mut f, &mut hdr) else {
        return false;
    };
    (n >= 3 && hdr[..3] == [0xFF, 0xD8, 0xFF])
        || (n >= 8 && hdr[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
        || (n >= 2 && hdr[..2] == [b'B', b'M'])
}

unsafe fn read_desktop_binary(name: &str) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; 2048];
    let mut bytes = buffer.len() as u32;
    if RegGetValueW(
        HKEY_CURRENT_USER,
        wide("Control Panel\\Desktop").as_ptr(),
        wide(name).as_ptr(),
        RRF_RT_REG_BINARY,
        null_mut(),
        buffer.as_mut_ptr() as _,
        &mut bytes,
    ) == 0
    {
        buffer.truncate(bytes as usize);
        Some(buffer)
    } else {
        None
    }
}

unsafe fn resolve_per_monitor_path(raw: PathBuf) -> PathBuf {
    if is_decodable_image_file(&raw) {
        return raw;
    }
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return raw;
    };
    let themes_dir = PathBuf::from(appdata).join("Microsoft\\Windows\\Themes");
    let target_key = normalize_path_key(&raw);

    for idx in 0..16 {
        let reg_name = format!("TranscodedImageCache_{idx:03}");
        if let Some(bytes) = read_desktop_binary(&reg_name) {
            if let Some(cached_source) = parse_transcoded_cache_path(&bytes) {
                if normalize_path_key(&cached_source) == target_key {
                    let cand = themes_dir.join(format!("Transcoded_{idx:03}"));
                    if is_decodable_image_file(&cand) {
                        return cand;
                    }
                }
            }
        }
    }
    if let Some(bytes) = read_desktop_binary("TranscodedImageCache") {
        if let Some(cached_source) = parse_transcoded_cache_path(&bytes) {
            if normalize_path_key(&cached_source) == target_key {
                let cand = themes_dir.join("TranscodedWallpaper");
                if is_decodable_image_file(&cand) {
                    return cand;
                }
            }
        }
    }
    let cache_dir = themes_dir.join("TranscodedWallpaperCache");
    if let Ok(entries) = std::fs::read_dir(&cache_dir) {
        for entry in entries.flatten() {
            let meta_path = entry.path();
            if meta_path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("meta"))
            {
                if let Ok(bytes) = std::fs::read(&meta_path) {
                    if let Some(cached_source) = parse_transcoded_meta_path(&bytes) {
                        if normalize_path_key(&cached_source) == target_key {
                            let img_path = meta_path.with_extension("");
                            if is_decodable_image_file(&img_path) {
                                return img_path;
                            }
                        }
                    }
                }
            }
        }
    }
    raw
}

unsafe fn resolve_wallpaper_path(spi_path: PathBuf) -> PathBuf {
    if let Some(bytes) = read_desktop_binary("TranscodedImageCache") {
        if let Some(cached_source) = parse_transcoded_cache_path(&bytes) {
            if normalize_path_key(&cached_source) == normalize_path_key(&spi_path) {
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

unsafe extern "system" fn collect_active_monitor(
    hmon: HMONITOR,
    _: HDC,
    _: *mut RECT,
    param: LPARAM,
) -> BOOL {
    let list = &mut *(param as *mut Vec<(String, [i32; 4])>);
    let mut info: MONITORINFOEXW = zeroed();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if GetMonitorInfoW(hmon, &mut info as *mut _ as _) != 0 {
        let len = info
            .szDevice
            .iter()
            .position(|v| *v == 0)
            .unwrap_or(info.szDevice.len());
        let name = String::from_utf16_lossy(&info.szDevice[..len]);
        let r = info.monitorInfo.rcMonitor;
        if r.right > r.left && r.bottom > r.top {
            list.push((name, [r.left, r.top, r.right, r.bottom]));
        }
    }
    1
}

unsafe fn enumerate_active_monitors() -> Vec<(String, [i32; 4])> {
    let mut list: Vec<(String, [i32; 4])> = Vec::new();
    EnumDisplayMonitors(
        null_mut(),
        null(),
        Some(collect_active_monitor),
        &mut list as *mut _ as isize,
    );
    list
}

const RPC_E_CHANGED_MODE_HR: i32 = 0x8001_0106u32 as i32;
const CLSID_DESKTOP_WALLPAPER: GUID = GUID::from_u128(0xc2cf3110_460e_4fc1_b9d0_8a1c0c9cc4bd);
const IID_IDESKTOP_WALLPAPER: GUID = GUID::from_u128(0xb92b56a9_8b55_4e14_9a89_0199bbb6f93b);

struct ComInitGuard {
    uninit: bool,
}
impl ComInitGuard {
    unsafe fn enter_mta() -> Option<Self> {
        let hr = CoInitializeEx(null(), COINIT_MULTITHREADED as u32);
        if hr >= 0 {
            Some(Self { uninit: true })
        } else if hr == RPC_E_CHANGED_MODE_HR {
            Some(Self { uninit: false })
        } else {
            None
        }
    }
}
impl Drop for ComInitGuard {
    fn drop(&mut self) {
        if self.uninit {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

struct DesktopWallpaperPtr(*mut c_void);
impl DesktopWallpaperPtr {
    #[inline]
    unsafe fn vtable(&self) -> *const *const c_void {
        *(self.0 as *const *const *const c_void)
    }
}
impl Drop for DesktopWallpaperPtr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let release: extern "system" fn(*mut c_void) -> u32 =
                    std::mem::transmute(*self.vtable().add(2));
                release(self.0);
            }
        }
    }
}

struct CoTaskWStr(*mut u16);
impl CoTaskWStr {
    unsafe fn to_string(&self) -> String {
        if self.0.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        while *self.0.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(self.0, len))
    }
}
impl Drop for CoTaskWStr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                CoTaskMemFree(self.0 as _);
            }
        }
    }
}

unsafe fn query_spi_wallpaper_path() -> Option<PathBuf> {
    PATH_BUF.with(|buf| {
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
        if len == 0 {
            return None;
        }
        Some(PathBuf::from(String::from_utf16_lossy(&name[..len])))
    })
}

unsafe fn query_desktop_wallpaper_com_inner() -> Option<DesktopWallpaperSpec> {
    let mut raw_ptr = null_mut();
    let hr = CoCreateInstance(
        &CLSID_DESKTOP_WALLPAPER,
        null_mut(),
        CLSCTX_LOCAL_SERVER,
        &IID_IDESKTOP_WALLPAPER,
        &mut raw_ptr,
    );
    if hr < 0 || raw_ptr.is_null() {
        return None;
    }
    let wp = DesktopWallpaperPtr(raw_ptr);
    let vt = wp.vtable();

    let get_wallpaper: extern "system" fn(*mut c_void, *const u16, *mut *mut u16) -> i32 =
        std::mem::transmute(*vt.add(4));
    let get_path_at: extern "system" fn(*mut c_void, u32, *mut *mut u16) -> i32 =
        std::mem::transmute(*vt.add(5));
    let get_count: extern "system" fn(*mut c_void, *mut u32) -> i32 =
        std::mem::transmute(*vt.add(6));
    let get_monitor_rect: extern "system" fn(*mut c_void, *const u16, *mut RECT) -> i32 =
        std::mem::transmute(*vt.add(7));
    let get_bg_color: extern "system" fn(*mut c_void, *mut u32) -> i32 =
        std::mem::transmute(*vt.add(9));
    let get_position: extern "system" fn(*mut c_void, *mut i32) -> i32 =
        std::mem::transmute(*vt.add(11));

    let mut pos = -1i32;
    let (style, tile) = if get_position(wp.0, &mut pos) >= 0 {
        match pos {
            0 => (0, false),
            1 => (0, true),
            2 => (2, false),
            3 => (6, false),
            4 => (10, false),
            5 => (22, false),
            _ => (
                registry_number("WallpaperStyle", 10),
                registry_number("TileWallpaper", 0) != 0,
            ),
        }
    } else {
        (
            registry_number("WallpaperStyle", 10),
            registry_number("TileWallpaper", 0) != 0,
        )
    };
    let mut bg_color = 0u32;
    if get_bg_color(wp.0, &mut bg_color) < 0 {
        bg_color = 0;
    }
    let mut count = 0u32;
    if get_count(wp.0, &mut count) < 0 || count == 0 {
        return None;
    }
    let mut active_mons: Vec<(CoTaskWStr, [i32; 4])> = Vec::new();
    for idx in 0..count.min(64) {
        let mut mid_ptr = null_mut();
        if get_path_at(wp.0, idx, &mut mid_ptr) < 0 || mid_ptr.is_null() {
            continue;
        }
        let mid = CoTaskWStr(mid_ptr);
        let mut r: RECT = zeroed();
        if get_monitor_rect(wp.0, mid.0, &mut r) == 0 && r.right > r.left && r.bottom > r.top {
            active_mons.push((mid, [r.left, r.top, r.right, r.bottom]));
        }
    }
    if active_mons.is_empty() {
        return None;
    }

    let mut global_ptr = null_mut();
    let hr_global = get_wallpaper(wp.0, null(), &mut global_ptr);
    let global_ws = CoTaskWStr(global_ptr);
    let global_str = if hr_global == 0 {
        global_ws.to_string()
    } else {
        String::new()
    };

    if !global_str.is_empty() && style != 22 {
        let resolved = resolve_per_monitor_path(PathBuf::from(global_str));
        let entries = active_mons
            .into_iter()
            .map(|(_, rect)| (rect, Some(resolved.clone())))
            .collect();
        return Some(dedup_monitors(entries, style, tile, bg_color));
    }

    if style == 22 {
        let mut span_path = if !global_str.is_empty() {
            Some(PathBuf::from(global_str))
        } else {
            None
        };
        if span_path.is_none() {
            for (mid, _) in &active_mons {
                let mut wp_ptr = null_mut();
                if get_wallpaper(wp.0, mid.0, &mut wp_ptr) >= 0 && !wp_ptr.is_null() {
                    let s = CoTaskWStr(wp_ptr).to_string();
                    if !s.is_empty() {
                        span_path = Some(PathBuf::from(s));
                        break;
                    }
                }
            }
        }
        if span_path.is_none() {
            span_path = query_spi_wallpaper_path();
        }
        let resolved = span_path.map(|p| resolve_per_monitor_path(p));
        let entries = active_mons
            .into_iter()
            .map(|(_, rect)| (rect, resolved.clone()))
            .collect();
        return Some(dedup_monitors(entries, 22, false, bg_color));
    }

    let mut entries = Vec::with_capacity(active_mons.len());
    for (mid, rect) in active_mons {
        let mut wp_ptr = null_mut();
        let path_opt = if get_wallpaper(wp.0, mid.0, &mut wp_ptr) >= 0 && !wp_ptr.is_null() {
            let s = CoTaskWStr(wp_ptr).to_string();
            if s.is_empty() {
                None
            } else {
                Some(resolve_per_monitor_path(PathBuf::from(s)))
            }
        } else {
            None
        };
        entries.push((rect, path_opt));
    }
    Some(dedup_monitors(entries, style, tile, bg_color))
}

fn query_desktop_wallpaper_com() -> Option<DesktopWallpaperSpec> {
    use std::sync::{mpsc, OnceLock};
    type Req = mpsc::SyncSender<Option<DesktopWallpaperSpec>>;
    static WORKER: OnceLock<Mutex<mpsc::Sender<Req>>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (req_tx, req_rx) = mpsc::channel::<Req>();
        let _ = std::thread::Builder::new()
            .name("lumen-wallpaper-com".into())
            .spawn(move || unsafe {
                // Run out-of-process IDesktopWallpaper IPC in an MTA worker so COM never
                // pumps a modal STA message loop on the UI thread while RefCell<Glass>
                // or widget state is mutably borrowed.
                let _com = ComInitGuard::enter_mta();
                while let Ok(reply) = req_rx.recv() {
                    let _ = reply.send(query_desktop_wallpaper_com_inner());
                }
            });
        Mutex::new(req_tx)
    });
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    let sent = worker
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .send(reply_tx)
        .is_ok();
    if !sent {
        return None;
    }
    reply_rx
        .recv_timeout(Duration::from_millis(1500))
        .ok()
        .flatten()
}

unsafe fn query_desktop_wallpaper_legacy() -> Option<DesktopWallpaperSpec> {
    let spi_opt = query_spi_wallpaper_path().map(|p| resolve_wallpaper_path(p));
    let style = registry_number("WallpaperStyle", 10);
    let tile = registry_number("TileWallpaper", 0) != 0;
    let mut mons = enumerate_active_monitors();
    if mons.is_empty() {
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(GetSystemMetrics(SM_CXSCREEN)).max(1);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(GetSystemMetrics(SM_CYSCREEN)).max(1);
        mons.push((String::from(r"\\.\DISPLAY1"), [vx, vy, vx + vw, vy + vh]));
    }
    let entries = mons
        .into_iter()
        .map(|(_, rect)| (rect, spi_opt.clone()))
        .collect();
    Some(dedup_monitors(entries, style, tile, 0))
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

fn resolve_wallpaper_engine_entry_image(entry: &serde_json::Value) -> Option<PathBuf> {
    let file_str = entry.get("file").and_then(|v| v.as_str())?;
    let file_path = PathBuf::from(file_str);
    if is_supported_image(&file_path) {
        return Some(file_path);
    }
    if file_path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("project.json"))
    {
        return resolve_project_json_image(&file_path);
    }
    None
}

fn parse_wallpaper_engine_config(
    raw: &str,
    active_monitors: &[(String, [i32; 4])],
) -> Option<DesktopWallpaperSpec> {
    if active_monitors.is_empty() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_str(raw).ok()?;
    let root = json.as_object()?;
    for profile in root.values() {
        let wp_cfg = profile.get("general").and_then(|g| g.get("wallpaperconfig"));
        let Some(selected) = wp_cfg
            .and_then(|w| w.get("selectedwallpapers"))
            .or_else(|| profile.get("selectedwallpapers"))
            .and_then(|s| s.as_object())
        else {
            continue;
        };
        let layout = wp_cfg
            .and_then(|w| w.get("layout"))
            .or_else(|| profile.get("layout"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let fallback_img = selected
            .get("Monitor0")
            .or_else(|| selected.values().next())
            .and_then(resolve_wallpaper_engine_entry_image);

        if layout.contains("stretch") || layout.contains("span") {
            let img = fallback_img?;
            let entries = active_monitors
                .iter()
                .map(|(_, r)| (*r, Some(img.clone())))
                .collect();
            return Some(dedup_monitors(entries, 22, false, 0));
        }
        if layout.contains("clone") {
            let img = fallback_img?;
            let entries = active_monitors
                .iter()
                .map(|(_, r)| (*r, Some(img.clone())))
                .collect();
            return Some(dedup_monitors(entries, 10, false, 0));
        }

        let mut entries = Vec::with_capacity(active_monitors.len());
        let mut any_img = false;
        for (idx, (dev_name, rect)) in active_monitors.iter().enumerate() {
            let matched = selected
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(dev_name))
                .map(|(_, v)| v)
                .or_else(|| {
                    let digits: String = dev_name
                        .chars()
                        .rev()
                        .take_while(|c| c.is_ascii_digit())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    let mon_idx = digits
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| n.checked_sub(1))
                        .unwrap_or(idx);
                    selected.get(&format!("Monitor{mon_idx}"))
                })
                .or_else(|| selected.get(&format!("Monitor{idx}")))
                .and_then(resolve_wallpaper_engine_entry_image)
                .or_else(|| fallback_img.clone());
            if matched.is_some() {
                any_img = true;
            }
            entries.push((*rect, matched));
        }
        if any_img {
            return Some(dedup_monitors(entries, 10, false, 0));
        }
    }
    None
}

fn resolve_wallpaper_engine_spec(
    exe_dir: &Path,
    active_monitors: &[(String, [i32; 4])],
) -> Option<DesktopWallpaperSpec> {
    let config_path = exe_dir.join("config.json");
    let raw = std::fs::read_to_string(config_path).ok()?;
    parse_wallpaper_engine_config(&raw, active_monitors)
}

fn resolve_lively_item_image(item: &serde_json::Value) -> Option<PathBuf> {
    if let Some(info_dir) = item.get("LivelyInfoPath").and_then(|v| v.as_str()) {
        let info_path = Path::new(info_dir).join("LivelyInfo.json");
        if let Some(img) = resolve_project_json_image(&info_path) {
            return Some(img);
        }
    }
    if let Some(file_str) = item.get("FilePath").and_then(|v| v.as_str()) {
        let path = PathBuf::from(file_str);
        if is_supported_image(&path) {
            return Some(path);
        }
    }
    None
}

fn parse_lively_config(
    layout_raw: &str,
    settings_raw: Option<&str>,
    active_monitors: &[(String, [i32; 4])],
) -> Option<DesktopWallpaperSpec> {
    if active_monitors.is_empty() {
        return None;
    }
    let layouts_val: serde_json::Value = serde_json::from_str(layout_raw).ok()?;
    let layouts = layouts_val.as_array()?;
    if layouts.is_empty() {
        return None;
    }
    let arrangement = settings_raw
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| {
            v.get("WallpaperArrangement")
                .and_then(|a| {
                    a.as_str()
                        .map(|s| s.to_ascii_lowercase())
                        .or_else(|| a.as_u64().map(|n| match n {
                            1 => "span".to_string(),
                            2 => "duplicate".to_string(),
                            _ => "per".to_string(),
                        }))
                })
        })
        .unwrap_or_else(|| "per".to_string());

    let first_img = layouts.first().and_then(resolve_lively_item_image);
    if arrangement.contains("span") {
        let img = first_img?;
        let entries = active_monitors
            .iter()
            .map(|(_, r)| (*r, Some(img.clone())))
            .collect();
        return Some(dedup_monitors(entries, 22, false, 0));
    }
    if arrangement.contains("dup") {
        let img = first_img?;
        let entries = active_monitors
            .iter()
            .map(|(_, r)| (*r, Some(img.clone())))
            .collect();
        return Some(dedup_monitors(entries, 10, false, 0));
    }

    let mut entries = Vec::with_capacity(active_monitors.len());
    let mut any_img = false;
    for (dev_name, rect) in active_monitors {
        let matched = layouts
            .iter()
            .find(|item| {
                let Some(screen) = item.get("LivelyScreen") else {
                    return false;
                };
                let dev_match = screen
                    .get("DeviceName")
                    .and_then(|v| v.as_str())
                    .is_some_and(|d| d.eq_ignore_ascii_case(dev_name));
                let bounds_match = screen.get("Bounds").is_some_and(|b| {
                    let x = b.get("X").and_then(|v| v.as_i64());
                    let y = b.get("Y").and_then(|v| v.as_i64());
                    let w = b.get("Width").and_then(|v| v.as_i64());
                    let h = b.get("Height").and_then(|v| v.as_i64());
                    matches!((x, y, w, h), (Some(x), Some(y), Some(w), Some(h))
                        if [x as i32, y as i32, (x + w) as i32, (y + h) as i32] == *rect)
                });
                dev_match || bounds_match
            })
            .and_then(resolve_lively_item_image)
            .or_else(|| first_img.clone());
        if matched.is_some() {
            any_img = true;
        }
        entries.push((*rect, matched));
    }
    any_img.then(|| dedup_monitors(entries, 10, false, 0))
}

fn resolve_lively_spec(active_monitors: &[(String, [i32; 4])]) -> Option<DesktopWallpaperSpec> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")?;
    let base = PathBuf::from(local_app_data).join("Lively Wallpaper\\SaveData\\wpdata");
    let layout_raw = std::fs::read_to_string(base.join("WallpaperLayout.json")).ok()?;
    let settings_raw = base
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("Settings.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .or_else(|| {
            base.parent()
                .map(|p| p.join("Settings.json"))
                .and_then(|p| std::fs::read_to_string(p).ok())
        });
    parse_lively_config(&layout_raw, settings_raw.as_deref(), active_monitors)
}

unsafe fn resolve_live_wallpaper_spec() -> Option<DesktopWallpaperSpec> {
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
        let active_monitors = enumerate_active_monitors();
        if exe_name.contains("wallpaper32") || exe_name.contains("wallpaper64") {
            if let Some(dir) = exe_path.parent() {
                if let Some(spec) = resolve_wallpaper_engine_spec(dir, &active_monitors) {
                    return Some(spec);
                }
            }
        }
        if exe_name.contains("lively") || exe_name.contains("cefsharp") {
            if let Some(spec) = resolve_lively_spec(&active_monitors) {
                return Some(spec);
            }
        }
        if let Some(dir) = exe_path.parent() {
            if let Some(spec) = resolve_wallpaper_engine_spec(dir, &active_monitors) {
                return Some(spec);
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
                        dw, dh, tile, style, bg_bgr: [0.0; 3],
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
            tile: false, style: 10, bg_bgr: [0.0; 3],
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
            tile: false, style: 10, bg_bgr: [0.0; 3],
        };
        let placed_surface = PlacedSurface::Single(placed);
        let mut out = original.clone();
        apply_liquid_glass_pixels(
            &mut out, w * 4, w, h, &original, SharpInput::None, Some(&placed_surface),
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
            let Some(spec) = Glass::query_desktop_wallpaper() else {
                return;
            };
            clear_cache();
            let mut glass = Glass::new(13., 1.);
            let keys: Vec<SourceKey> = spec.unique_sources.iter().map(|u| u.key.clone()).collect();
            {
                *LOADING_KEY.lock().unwrap_or_else(|e| e.into_inner()) = Some(keys);
            }
            assert!(!glass.refresh_async(null_mut(), 0));
            assert!(glass.bitmap.is_none());
            assert!(glass.path.as_os_str().is_empty());
            *LOADING_KEY.lock().unwrap_or_else(|e| e.into_inner()) = None;
            assert!(glass.refresh());
            assert!(glass.available());
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
            cap: Some((1920, 1080)),
            small: RefCell::new(Some(preview.clone())),
            preview,
            sharp: Cell::new(null_mut()),
            sharp_size: Cell::new((0, 0)),
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

    struct TestTempDir(PathBuf);
    impl TestTempDir {
        fn new(tag: &str) -> Self {
            static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!(
                "lumen-{tag}-{}-{n}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestTempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct TestWindow(HWND);
    impl TestWindow {
        unsafe fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                wide("STATIC").as_ptr(),
                wide("Lumen multi-monitor test").as_ptr(),
                WS_POPUP,
                x,
                y,
                w,
                h,
                null_mut(),
                null_mut(),
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            Self(hwnd)
        }
    }
    impl Drop for TestWindow {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }

    struct TestGlassState(bool, bool);
    impl TestGlassState {
        fn enter() -> Self {
            clear_cache();
            let prev_lg = liquid_glass();
            let prev_lp = crate::motion::low_power();
            crate::motion::set_low_power(false);
            set_liquid_glass(false);
            Self(prev_lg, prev_lp)
        }
    }
    impl Drop for TestGlassState {
        fn drop(&mut self) {
            set_liquid_glass(self.0);
            crate::motion::set_low_power(self.1);
            clear_cache();
        }
    }

    fn write_solid_png(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
        image::RgbaImage::from_pixel(w, h, image::Rgba(rgba))
            .save(path)
            .unwrap();
    }

    #[test]
    fn per_monitor_different_wallpapers_sample_independently_and_across_border() {
        let _ui = crate::native_ui_test_lock();
        let _state = TestGlassState::enter();
        let dir = TestTempDir::new("per-monitor");
        let red_path = dir.path().join("red.png");
        let blue_path = dir.path().join("blue.png");
        write_solid_png(&red_path, 160, 120, [220, 40, 40, 255]);
        write_solid_png(&blue_path, 160, 120, [40, 60, 220, 255]);

        unsafe {
            let glass = Glass::fixture_multi_monitor(
                vec![
                    ([-320, 0, 0, 240], Some(red_path)),
                    ([0, 0, 320, 240], Some(blue_path)),
                ],
                10,
                false,
                0,
                0.0,
                1.0,
            );
            assert_eq!(glass.unique_wallpaper_count(), 2);
            assert!(!glass.is_shared_wallpaper());

            let win_left = TestWindow::new(-200, 40, 80, 60);
            let win_right = TestWindow::new(40, 40, 80, 60);
            let win_straddle = TestWindow::new(-40, 20, 80, 80);

            let left_buf = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            glass.draw_blended(left_buf.dc, win_left.0, 80, 60, 255);
            let lpx = left_buf.snapshot();
            let li = (30 * 80 + 40) * 4;
            assert!(
                lpx[li + 2] > lpx[li] + 100,
                "left monitor should sample red: BGRA={:?}",
                &lpx[li..li + 4]
            );

            let right_buf = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            glass.draw_blended(right_buf.dc, win_right.0, 80, 60, 255);
            let rpx = right_buf.snapshot();
            let ri = (30 * 80 + 40) * 4;
            assert!(
                rpx[ri] > rpx[ri + 2] + 100,
                "right monitor should sample blue: BGRA={:?}",
                &rpx[ri..ri + 4]
            );

            let straddle_buf = crate::compositor::Buffer::plain(80, 80, 1.0).unwrap();
            glass.draw_blended(straddle_buf.dc, win_straddle.0, 80, 80, 255);
            let spx = straddle_buf.snapshot();
            let s_left = (40 * 80 + 10) * 4;
            let s_right = (40 * 80 + 70) * 4;
            assert!(
                spx[s_left + 2] > spx[s_left] + 100,
                "straddle left half should sample red: BGRA={:?}",
                &spx[s_left..s_left + 4]
            );
            assert!(
                spx[s_right] > spx[s_right + 2] + 100,
                "straddle right half should sample blue: BGRA={:?}",
                &spx[s_right..s_right + 4]
            );

            let tile = glass
                .sample_blurred_tile(
                    win_straddle.0,
                    RECT {
                        left: -40,
                        top: 20,
                        right: 40,
                        bottom: 100,
                    },
                    8,
                    8,
                )
                .expect("blurred tile");
            let tile_left = tile[4 * 8 + 1];
            let tile_right = tile[4 * 8 + 6];
            assert!(
                tile_left[2] > tile_left[0] + 100,
                "tile left should be red: {tile_left:?}"
            );
            assert!(
                tile_right[0] > tile_right[2] + 100,
                "tile right should be blue: {tile_right:?}"
            );

            set_liquid_glass(true);
            let lg_buf = crate::compositor::Buffer::plain(80, 80, 1.0).unwrap();
            glass.draw_blended(lg_buf.dc, win_straddle.0, 80, 80, 255);
            let lgpx = lg_buf.snapshot();
            let rim_l = (40 * 80 + 4) * 4;
            let rim_r = (40 * 80 + 75) * 4;
            assert!(
                lgpx[rim_l + 2] > lgpx[rim_l] + 60,
                "liquid glass left rim should preserve red monitor: BGRA={:?}",
                &lgpx[rim_l..rim_l + 4]
            );
            assert!(
                lgpx[rim_r] > lgpx[rim_r + 2] + 60,
                "liquid glass right rim should preserve blue monitor: BGRA={:?}",
                &lgpx[rim_r..rim_r + 4]
            );

            let acc_left = glass.wallpaper_accent_for(win_left.0);
            let acc_right = glass.wallpaper_accent_for(win_right.0);
            assert!(
                (acc_left & 0xff) > ((acc_left >> 16) & 0xff),
                "left accent must favor red: {acc_left:#08x}"
            );
            assert!(
                ((acc_right >> 16) & 0xff) > (acc_right & 0xff),
                "right accent must favor blue: {acc_right:#08x}"
            );
        }
    }

    #[test]
    fn shared_wallpaper_deduplicates_sources_blur_and_native_layers() {
        let _ui = crate::native_ui_test_lock();
        let _state = TestGlassState::enter();
        let dir = TestTempDir::new("shared-dedup");
        let teal_path = dir.path().join("teal.png");
        write_solid_png(&teal_path, 160, 120, [20, 180, 190, 255]);
        let canon = std::fs::canonicalize(&teal_path).unwrap();
        let plain_str = canon
            .to_string_lossy()
            .strip_prefix(r"\\?\")
            .unwrap_or(&canon.to_string_lossy())
            .to_string();
        let path_mon0 = PathBuf::from(&plain_str);
        let path_mon1 = PathBuf::from(format!(r"\\?\{}", plain_str.to_ascii_uppercase()));

        unsafe {
            let glass = Glass::fixture_multi_monitor(
                vec![
                    ([-320, 0, 0, 240], Some(path_mon0)),
                    ([0, 0, 320, 240], Some(path_mon1)),
                ],
                10,
                false,
                0,
                0.0,
                1.0,
            );
            assert_eq!(glass.unique_wallpaper_count(), 1);
            assert!(glass.is_shared_wallpaper());
            assert_eq!(glass.monitors.len(), 2);
            assert_eq!(glass.monitors[0].slot, Some(0));
            assert_eq!(glass.monitors[1].slot, Some(0));

            let win0 = TestWindow::new(-200, 40, 80, 60);
            let win1 = TestWindow::new(40, 40, 80, 60);
            let b0 = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            let b1 = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            glass.draw_blended(b0.dc, win0.0, 80, 60, 255);
            glass.draw_blended(b1.dc, win1.0, 80, 60, 255);

            assert_eq!(WALLPAPER_SOURCES.with(|c| c.borrow().len()), 1);
            assert_eq!(BLURRED_CACHE.with(|c| c.borrow().len()), 1);
            let p0 = b0.snapshot();
            let p1 = b1.snapshot();
            let idx = (30 * 80 + 40) * 4;
            assert_eq!(&p0[idx..idx + 3], &p1[idx..idx + 3]);
            assert!(
                p0[idx] > p0[idx + 2] + 80 && p0[idx + 1] > p0[idx + 2] + 80,
                "shared wallpaper must render teal on both monitors: BGRA={:?}",
                &p0[idx..idx + 4]
            );
        }
    }

    #[test]
    fn partial_monitor_sharing_deduplicates_matching_subset() {
        let _ui = crate::native_ui_test_lock();
        let _state = TestGlassState::enter();
        let dir = TestTempDir::new("partial-dedup");
        let path_a = dir.path().join("wp_a.png");
        let path_b = dir.path().join("wp_b.png");
        write_solid_png(&path_a, 160, 120, [210, 50, 50, 255]);
        write_solid_png(&path_b, 160, 120, [50, 210, 60, 255]);

        unsafe {
            let glass = Glass::fixture_multi_monitor(
                vec![
                    ([-640, 0, -320, 240], Some(path_a.clone())),
                    ([-320, 0, 0, 240], Some(path_a.clone())),
                    ([0, 0, 320, 240], Some(path_b)),
                ],
                10,
                false,
                0,
                0.0,
                1.0,
            );
            assert_eq!(glass.unique_wallpaper_count(), 2);
            assert!(!glass.is_shared_wallpaper());
            assert_eq!(glass.monitors.len(), 3);
            assert_eq!(glass.monitors[0].slot, Some(0));
            assert_eq!(glass.monitors[1].slot, Some(0));
            assert_eq!(glass.monitors[2].slot, Some(1));

            let bg_color = 0x00_30_20_10u32;
            let partial_fail = Glass::fixture_multi_monitor(
                vec![
                    ([-320, 0, 0, 240], Some(path_a)),
                    ([0, 0, 320, 240], Some(dir.path().join("missing.png"))),
                ],
                10,
                false,
                bg_color,
                0.0,
                1.0,
            );
            assert_eq!(partial_fail.unique_wallpaper_count(), 1);
            assert_eq!(partial_fail.monitors[0].slot, Some(0));
            assert_eq!(partial_fail.monitors[1].slot, None);

            let win_ok = TestWindow::new(-200, 40, 80, 60);
            let win_fail = TestWindow::new(40, 40, 80, 60);
            let ok_buf = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            let fail_buf = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            partial_fail.draw_blended(ok_buf.dc, win_ok.0, 80, 60, 255);
            partial_fail.draw_blended(fail_buf.dc, win_fail.0, 80, 60, 255);

            let ok_px = ok_buf.snapshot();
            let ok_i = (30 * 80 + 40) * 4;
            assert!(
                ok_px[ok_i + 2] > ok_px[ok_i] + 100,
                "valid monitor must still render red when sibling monitor fails: {:?}",
                &ok_px[ok_i..ok_i + 4]
            );
            let fail_px = fail_buf.snapshot();
            let fail_i = (30 * 80 + 40) * 4;
            assert_eq!(
                &fail_px[fail_i..fail_i + 3],
                &[0x30, 0x20, 0x10],
                "failed monitor must fall back to bg_color"
            );
        }
    }

    #[test]
    fn span_mode_samples_continuous_virtual_canvas_and_caches_per_monitor_slices() {
        let _ui = crate::native_ui_test_lock();
        let _state = TestGlassState::enter();
        let dir = TestTempDir::new("span-mode");
        let span_path = dir.path().join("span.png");
        let span_img = image::RgbaImage::from_fn(640, 240, |x, _| {
            if x < 320 {
                image::Rgba([30, 210, 40, 255]) // left half: green
            } else {
                image::Rgba([230, 210, 30, 255]) // right half: yellow
            }
        });
        span_img.save(&span_path).unwrap();

        unsafe {
            let glass = Glass::fixture_multi_monitor(
                vec![
                    ([-320, 0, 0, 240], Some(span_path.clone())),
                    ([0, 0, 320, 240], Some(span_path)),
                ],
                22,
                false,
                0,
                0.0,
                1.0,
            );
            assert_eq!(glass.unique_wallpaper_count(), 1);
            assert!(glass.is_shared_wallpaper());

            let win_left = TestWindow::new(-200, 40, 80, 60);
            let win_right = TestWindow::new(40, 40, 80, 60);
            let win_straddle = TestWindow::new(-40, 20, 80, 80);

            let lb = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            glass.draw_blended(lb.dc, win_left.0, 80, 60, 255);
            let lpx = lb.snapshot();
            let li = (30 * 80 + 40) * 4;
            assert!(
                lpx[li + 1] > lpx[li + 2] + 80 && lpx[li + 1] > lpx[li] + 80,
                "span left monitor should sample green: BGRA={:?}",
                &lpx[li..li + 4]
            );

            let rb = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            glass.draw_blended(rb.dc, win_right.0, 80, 60, 255);
            let rpx = rb.snapshot();
            let ri = (30 * 80 + 40) * 4;
            assert!(
                rpx[ri + 2] > rpx[ri] + 80 && rpx[ri + 1] > rpx[ri] + 80,
                "span right monitor should sample yellow: BGRA={:?}",
                &rpx[ri..ri + 4]
            );

            assert_eq!(WALLPAPER_SOURCES.with(|c| c.borrow().len()), 1);
            assert_eq!(BLURRED_CACHE.with(|c| c.borrow().len()), 1);

            let sb = crate::compositor::Buffer::plain(80, 80, 1.0).unwrap();
            glass.draw_blended(sb.dc, win_straddle.0, 80, 80, 255);
            let spx = sb.snapshot();
            let sl = (40 * 80 + 10) * 4;
            let sr = (40 * 80 + 70) * 4;
            assert!(
                spx[sl + 1] > spx[sl + 2] + 80,
                "span straddle x=10 should be green: BGRA={:?}",
                &spx[sl..sl + 4]
            );
            assert!(
                spx[sr + 2] > spx[sr] + 80 && spx[sr + 1] > spx[sr] + 80,
                "span straddle x=70 should be yellow: BGRA={:?}",
                &spx[sr..sr + 4]
            );
        }
    }

    #[test]
    fn shared_tile_mode_anchors_at_primary_origin_across_negative_monitor() {
        let _ui = crate::native_ui_test_lock();
        let _state = TestGlassState::enter();
        let dir = TestTempDir::new("shared-tile");
        let tile_path = dir.path().join("tile.png");
        let tile_img = image::RgbaImage::from_fn(320, 240, |x, _| {
            if x < 160 {
                image::Rgba([220, 40, 40, 255]) // left half: red
            } else {
                image::Rgba([40, 60, 220, 255]) // right half: blue
            }
        });
        tile_img.save(&tile_path).unwrap();

        unsafe {
            let glass = Glass::fixture_multi_monitor(
                vec![
                    ([-160, 0, 0, 240], Some(tile_path.clone())),
                    ([0, 0, 320, 240], Some(tile_path)),
                ],
                0,
                true,
                0,
                0.0,
                1.0,
            );
            assert_eq!(glass.unique_wallpaper_count(), 1);
            assert!(glass.is_shared_wallpaper());

            let win_neg = TestWindow::new(-120, 40, 80, 60);
            let win_pos = TestWindow::new(20, 40, 80, 60);

            let neg_buf = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            glass.draw_blended(neg_buf.dc, win_neg.0, 80, 60, 255);
            let npx = neg_buf.snapshot();
            let ni = (30 * 80 + 40) * 4;
            assert!(
                npx[ni] > npx[ni + 2] + 80,
                "negative monitor x=-120..-40 in 320px shared tile anchored at (0,0) should wrap into right/blue half: BGRA={:?}",
                &npx[ni..ni + 4]
            );

            let pos_buf = crate::compositor::Buffer::plain(80, 60, 1.0).unwrap();
            glass.draw_blended(pos_buf.dc, win_pos.0, 80, 60, 255);
            let ppx = pos_buf.snapshot();
            let pi = (30 * 80 + 40) * 4;
            assert!(
                ppx[pi + 2] > ppx[pi] + 80,
                "primary monitor x=20..100 in 320px shared tile should sample left/red half: BGRA={:?}",
                &ppx[pi..pi + 4]
            );
        }
    }

    #[test]
    fn solid_color_monitor_and_fit_letterbox_use_desktop_background_color() {
        let _ui = crate::native_ui_test_lock();
        let _state = TestGlassState::enter();
        let dir = TestTempDir::new("solid-and-fit");
        let wide_path = dir.path().join("wide.png");
        write_solid_png(&wide_path, 320, 80, [20, 220, 30, 255]);
        let bg_color = 0x00_20_40_80u32; // COLORREF: R=0x80, G=0x40, B=0x20

        unsafe {
            let solid_glass = Glass::fixture_multi_monitor(
                vec![([0, 0, 320, 240], None)],
                10,
                false,
                bg_color,
                0.0,
                1.0,
            );
            let win_solid = TestWindow::new(40, 40, 64, 48);
            assert_eq!(solid_glass.raw_wallpaper_accent_for(win_solid.0), bg_color);
            let sbuf = crate::compositor::Buffer::plain(64, 48, 1.0).unwrap();
            solid_glass.draw_blended(sbuf.dc, win_solid.0, 64, 48, 255);
            let spx = sbuf.snapshot();
            let si = (24 * 64 + 32) * 4;
            assert_eq!(
                &spx[si..si + 3],
                &[0x20, 0x40, 0x80],
                "solid-color monitor must render bg_color"
            );

            let fit_glass = Glass::fixture_multi_monitor(
                vec![([0, 0, 320, 240], Some(wide_path))],
                6,
                false,
                bg_color,
                0.0,
                1.0,
            );
            let win_letterbox = TestWindow::new(40, 4, 40, 20);
            let win_image = TestWindow::new(40, 100, 40, 40);
            let lbuf = crate::compositor::Buffer::plain(40, 20, 1.0).unwrap();
            fit_glass.draw_blended(lbuf.dc, win_letterbox.0, 40, 20, 255);
            let lpx = lbuf.snapshot();
            let li = (10 * 40 + 20) * 4;
            assert_eq!(
                &lpx[li..li + 3],
                &[0x20, 0x40, 0x80],
                "fit mode top letterbox must render bg_color"
            );

            let ibuf = crate::compositor::Buffer::plain(40, 40, 1.0).unwrap();
            fit_glass.draw_blended(ibuf.dc, win_image.0, 40, 40, 255);
            let ipx = ibuf.snapshot();
            let ii = (20 * 40 + 20) * 4;
            assert!(
                ipx[ii + 1] > ipx[ii + 2] + 80,
                "fit mode image region must render green wallpaper: BGRA={:?}",
                &ipx[ii..ii + 4]
            );
        }
    }

    #[test]
    fn transcoded_meta_path_decodes_utf16_source_at_offset_20() {
        let mut raw = vec![0u8; 20];
        for ch in r"E:\wallpaper\sample.jpg".encode_utf16() {
            raw.extend_from_slice(&ch.to_le_bytes());
        }
        raw.extend_from_slice(&[0, 0, 1, 2, 3, 4]);
        assert_eq!(
            super::parse_transcoded_meta_path(&raw),
            Some(PathBuf::from(r"E:\wallpaper\sample.jpg"))
        );
        assert_eq!(super::parse_transcoded_meta_path(&[0u8; 18]), None);
    }

    #[test]
    fn live_wallpaper_parsers_handle_per_monitor_clone_and_span_configs() {
        let dir = TestTempDir::new("live-parsers");
        let wp1 = dir.path().join("wp1.png");
        let wp2 = dir.path().join("wp2.png");
        write_solid_png(&wp1, 8, 8, [200, 30, 30, 255]);
        write_solid_png(&wp2, 8, 8, [30, 30, 200, 255]);

        let active = vec![
            (r"\\.\DISPLAY1".to_string(), [-1920, 0, 0, 1080]),
            (r"\\.\DISPLAY2".to_string(), [0, 0, 1920, 1080]),
        ];

        let we_per = serde_json::json!({
            "steamuser": {
                "general": {
                    "wallpaperconfig": {
                        "layout": "permonitor",
                        "selectedwallpapers": {
                            "Monitor0": { "file": wp1 },
                            "Monitor1": { "file": wp2 }
                        }
                    }
                }
            }
        })
        .to_string();
        let spec_per = parse_wallpaper_engine_config(&we_per, &active).unwrap();
        assert_eq!(spec_per.unique_sources.len(), 2);
        assert!(!spec_per.shared);
        assert_eq!(spec_per.style, 10);

        let we_clone = serde_json::json!({
            "steamuser": {
                "general": {
                    "wallpaperconfig": {
                        "layout": "clone",
                        "selectedwallpapers": {
                            "Monitor0": { "file": wp1 },
                            "Monitor1": { "file": wp2 }
                        }
                    }
                }
            }
        })
        .to_string();
        let spec_clone = parse_wallpaper_engine_config(&we_clone, &active).unwrap();
        assert_eq!(spec_clone.unique_sources.len(), 1);
        assert!(spec_clone.shared);
        assert_eq!(spec_clone.style, 10);

        let we_stretch = serde_json::json!({
            "steamuser": {
                "general": {
                    "wallpaperconfig": {
                        "layout": "stretch",
                        "selectedwallpapers": {
                            "Monitor0": { "file": wp1 }
                        }
                    }
                }
            }
        })
        .to_string();
        let spec_stretch = parse_wallpaper_engine_config(&we_stretch, &active).unwrap();
        assert!(spec_stretch.shared);
        assert_eq!(spec_stretch.style, 22);

        let lively_layout = serde_json::json!([
            {
                "FilePath": wp1,
                "LivelyScreen": {
                    "DeviceName": r"\\.\DISPLAY1",
                    "Bounds": { "X": -1920, "Y": 0, "Width": 1920, "Height": 1080 }
                }
            },
            {
                "FilePath": wp2,
                "LivelyScreen": {
                    "DeviceName": r"\\.\DISPLAY2",
                    "Bounds": { "X": 0, "Y": 0, "Width": 1920, "Height": 1080 }
                }
            }
        ])
        .to_string();
        let lively_per = parse_lively_config(&lively_layout, None, &active).unwrap();
        assert_eq!(lively_per.unique_sources.len(), 2);
        assert!(!lively_per.shared);

        let lively_dup = parse_lively_config(
            &lively_layout,
            Some(r#"{"WallpaperArrangement":"duplicate"}"#),
            &active,
        )
        .unwrap();
        assert_eq!(lively_dup.unique_sources.len(), 1);
        assert!(lively_dup.shared);

        let lively_span = parse_lively_config(
            &lively_layout,
            Some(r#"{"WallpaperArrangement":"span"}"#),
            &active,
        )
        .unwrap();
        assert!(lively_span.shared);
        assert_eq!(lively_span.style, 22);
    }

    #[test]
    fn live_desktop_wallpaper_query_discovers_active_monitors() {
        let _ui = crate::native_ui_test_lock();
        let spec = unsafe { Glass::query_desktop_wallpaper() }
            .expect("host desktop wallpaper query must return active monitor layout");
        assert!(!spec.monitors.is_empty());
        for m in &spec.monitors {
            assert!(
                m.rect[2] > m.rect[0] && m.rect[3] > m.rect[1],
                "monitor rect must be non-empty: {:?}",
                m.rect
            );
        }
    }

    #[test]
    fn com_wallpaper_query_does_not_pump_ui_messages_or_reenter_refcell() {
        use windows_sys::Win32::System::Com::COINIT_APARTMENTTHREADED;
        let _ui = crate::native_ui_test_lock();
        unsafe {
            struct StaGuard(bool);
            impl Drop for StaGuard {
                fn drop(&mut self) {
                    if self.0 {
                        unsafe {
                            CoUninitialize();
                        }
                    }
                }
            }
            let hr = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
            let _sta = StaGuard(hr >= 0);

            let win = TestWindow::new(0, 0, 32, 32);
            let custom_msg = WM_APP + 77;
            assert_ne!(PostMessageW(win.0, custom_msg, 123, 456), 0);

            let glass = RefCell::new(Glass::new(13.0, 1.0));
            let _guard = glass.borrow_mut();
            invalidate_query_cache();
            let _ = Glass::query_desktop_wallpaper();
            drop(_guard);

            let mut msg: MSG = zeroed();
            let got = PeekMessageW(&mut msg, win.0, custom_msg, custom_msg, PM_REMOVE);
            assert_ne!(
                got, 0,
                "COM wallpaper query on STA UI thread must not pump Win32 messages re-entrantly"
            );
            assert_eq!(msg.wParam, 123);
        }
    }

    #[test]
    fn blurred_wallpaper_downsamples_large_source_for_memory_savings() {
        let large = image::RgbaImage::from_fn(3840, 2160, |x, y| {
            let r = (x * 255 / 3839) as u8;
            let b = (y * 255 / 2159) as u8;
            image::Rgba([r, 80, b, 255])
        });
        let (bytes, bw, bh) = Glass::prepare_blurred_bytes(&large, 13.0, 1.75, 3840.0);
        assert!(
            bw <= 960 && bh <= 540,
            "4K blurred wallpaper should be downsampled, got {bw}x{bh}"
        );
        assert!(bw >= 256 && bh >= 144);
        assert_eq!(bytes.len(), (bw * bh * 4) as usize);

        let (zero_bytes, zw, zh) = Glass::prepare_blurred_bytes(&large, 0.0, 1.0, 3840.0);
        assert_eq!((zw, zh), (3840, 2160));
        assert_eq!(zero_bytes.len(), 3840 * 2160 * 4);
    }

    #[test]
    fn prune_caches_immediately_evicts_inactive_wallpapers() {
        let _ui = crate::native_ui_test_lock();
        let _state = TestGlassState::enter();
        let dir = TestTempDir::new("prune-immediate");
        let path_a = dir.path().join("a.png");
        let path_b = dir.path().join("b.png");
        write_solid_png(&path_a, 160, 120, [200, 40, 40, 255]);
        write_solid_png(&path_b, 160, 120, [40, 40, 200, 255]);

        unsafe {
            let mut glass = Glass::fixture_multi_monitor(
                vec![([0, 0, 320, 240], Some(path_a.clone()))],
                10,
                false,
                0,
                10.0,
                1.0,
            );
            assert_eq!(WALLPAPER_SOURCES.with(|c| c.borrow().len()), 1);
            assert_eq!(BLURRED_CACHE.with(|c| c.borrow().len()), 1);

            let spec_b = dedup_monitors(
                vec![([0, 0, 320, 240], Some(path_b.clone()))],
                10,
                false,
                0,
            );
            glass.apply_spec_sync(&spec_b);

            assert_eq!(WALLPAPER_SOURCES.with(|c| c.borrow().len()), 1);
            assert_eq!(BLURRED_CACHE.with(|c| c.borrow().len()), 1);
            let expected_key = file_spec(path_b).key;
            assert!(WALLPAPER_SOURCES.with(|c| c.borrow().contains_key(&expected_key)));
        }
    }

    #[test]
    fn sharp_bitmap_caps_4k_source_to_full_hd_and_clears_on_disable() {
        let _ui = crate::native_ui_test_lock();
        let _state = TestGlassState::enter();
        let dir = TestTempDir::new("sharp-cap");
        let path_4k = dir.path().join("wall4k.png");
        write_solid_png(&path_4k, 3840, 2160, [40, 160, 220, 255]);

        unsafe {
            let glass = Glass::fixture_multi_monitor(
                vec![([0, 0, 3840, 2160], Some(path_4k))],
                10,
                false,
                0,
                10.0,
                1.0,
            );
            let slot = &glass.slots[0];
            assert!(slot.source.small.borrow().is_none());
            assert!(slot.source.preview.width() <= PREVIEW_MAX_DIM);
            assert!(slot.source.preview.height() <= PREVIEW_MAX_DIM);

            set_liquid_glass(true);
            let bmp = slot.source.sharp_bitmap();
            assert!(!bmp.is_null());
            let mut ds: DIBSECTION = zeroed();
            assert!(
                GetObjectW(bmp as _, size_of::<DIBSECTION>() as i32, &mut ds as *mut _ as _)
                    >= size_of::<BITMAP>() as i32
            );
            assert_eq!((ds.dsBm.bmWidth, ds.dsBm.bmHeight), (1920, 1080));
            assert_eq!(slot.source.sharp_dimensions(), (1920, 1080));

            set_frosted_glass(true);
            set_liquid_glass(false);
            assert!(slot.source.sharp.get().is_null());
        }
    }

    #[test]
    fn bilinear_blit_matches_fit_fill_and_tile_with_clip() {
        let _ui = crate::native_ui_test_lock();
        let _state = TestGlassState::enter();
        let dir = TestTempDir::new("blit-modes");
        let wall_path = dir.path().join("quad.png");
        let img = image::RgbaImage::from_fn(120, 80, |x, y| {
            let r = if x < 60 { 220 } else { 30 };
            let b = if y < 40 { 30 } else { 220 };
            image::Rgba([r, 90, b, 255])
        });
        img.save(&wall_path).unwrap();

        unsafe {
            // Fit mode with bg_color letterbox and SaveDC/IntersectClipRect clip
            let bg = 0x00_44_22_11u32;
            let fit_glass = Glass::fixture_multi_monitor(
                vec![([0, 0, 240, 240], Some(wall_path.clone()))],
                6,
                false,
                bg,
                0.0,
                1.0,
            );
            let win = TestWindow::new(0, 0, 240, 240);
            let buf = crate::compositor::Buffer::plain(240, 240, 1.0).unwrap();
            fill_solid_bg(buf.dc, 240, 240, bg);
            let saved = SaveDC(buf.dc);
            IntersectClipRect(buf.dc, 20, 20, 220, 220);
            fit_glass.draw_rect_layer(
                buf.dc,
                win.0,
                240,
                240,
                RECT {
                    left: 0,
                    top: 0,
                    right: 240,
                    bottom: 240,
                },
                false,
                false,
            );
            RestoreDC(buf.dc, saved);
            let px = buf.snapshot();
            // Letterbox row y=25 inside clip must remain bg_color [0x44, 0x22, 0x11]
            let lb = (25 * 240 + 120) * 4;
            assert_eq!(&px[lb..lb + 3], &[0x44, 0x22, 0x11]);
            // Top-left quadrant (x=60, y=60) inside image (y in 40..200) must be red-dominant
            let tl = (60 * 240 + 60) * 4;
            assert!(px[tl + 2] > px[tl] + 100, "top-left should be red: {:?}", &px[tl..tl + 4]);
            // Bottom-right quadrant (x=180, y=180) must be blue-dominant
            let br = (180 * 240 + 180) * 4;
            assert!(px[br] > px[br + 2] + 100, "bottom-right should be blue: {:?}", &px[br..br + 4]);

            // Tile mode wrap across tile boundary
            let tile_glass = Glass::fixture_multi_monitor(
                vec![([0, 0, 240, 240], Some(wall_path))],
                0,
                true,
                0,
                0.0,
                1.0,
            );
            let tbuf = crate::compositor::Buffer::plain(240, 160, 1.0).unwrap();
            tile_glass.draw_blended(tbuf.dc, win.0, 240, 160, 255);
            let tpx = tbuf.snapshot();
            let tile0 = (20 * 240 + 20) * 4;
            let tile1 = (100 * 240 + 140) * 4;
            assert_eq!(&tpx[tile0..tile0 + 3], &tpx[tile1..tile1 + 3]);
        }
    }
}
