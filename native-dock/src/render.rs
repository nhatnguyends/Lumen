//! GDI+ vector/text renderer with stable premultiplied subpixel sprite sampling.
//! All objects are UI-thread-owned and released through RAII. No extra runtime.
use crate::model::wide;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    path::Path,
    ptr::null_mut,
};
use windows_sys::Win32::Graphics::Gdi::*;
type Ptr = *mut c_void;
#[repr(C)]
struct Startup {
    version: u32,
    callback: Ptr,
    no_thread: i32,
    no_codecs: i32,
}
#[repr(C)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f32,
    y: f32,
}
#[repr(C)]
struct ColorMatrix([f32; 25]);
#[link(name = "gdiplus")]
extern "system" {
    fn GdipCreateImageAttributes(out: *mut Ptr) -> i32;
    fn GdipDisposeImageAttributes(attributes: Ptr) -> i32;
    fn GdipSetImageAttributesColorMatrix(
        attributes: Ptr,
        kind: i32,
        enable: i32,
        matrix: *const ColorMatrix,
        gray: *const ColorMatrix,
        flags: i32,
    ) -> i32;
    fn GdiplusStartup(token: *mut usize, input: *const Startup, output: Ptr) -> i32;
    fn GdiplusShutdown(token: usize);
    fn GdipCreateFromHDC(dc: HDC, out: *mut Ptr) -> i32;
    fn GdipDeleteGraphics(g: Ptr) -> i32;
    fn GdipFlush(g: Ptr, intention: i32) -> i32;
    fn GdipSaveGraphics(g: Ptr, state: *mut u32) -> i32;
    fn GdipRestoreGraphics(g: Ptr, state: u32) -> i32;
    fn GdipResetWorldTransform(g: Ptr) -> i32;
    fn GdipGetImageGraphicsContext(image: Ptr, out: *mut Ptr) -> i32;
    fn GdipDrawImageRectRect(
        g: Ptr,
        image: Ptr,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        sx: f32,
        sy: f32,
        sw: f32,
        sh: f32,
        unit: i32,
        attributes: Ptr,
        callback: Ptr,
        data: Ptr,
    ) -> i32;
    fn GdipSetSmoothingMode(g: Ptr, mode: i32) -> i32;
    fn GdipSetTextRenderingHint(g: Ptr, mode: i32) -> i32;
    fn GdipSetInterpolationMode(g: Ptr, mode: i32) -> i32;
    fn GdipSetPixelOffsetMode(g: Ptr, mode: i32) -> i32;
    fn GdipCreatePath(fill: i32, out: *mut Ptr) -> i32;
    fn GdipDeletePath(path: Ptr) -> i32;
    fn GdipAddPathArc(path: Ptr, x: f32, y: f32, w: f32, h: f32, start: f32, sweep: f32) -> i32;
    fn GdipClosePathFigure(path: Ptr) -> i32;
    fn GdipCreateSolidFill(color: u32, out: *mut Ptr) -> i32;
    fn GdipDeleteBrush(brush: Ptr) -> i32;
    fn GdipFillPath(g: Ptr, brush: Ptr, path: Ptr) -> i32;
    fn GdipCreatePen1(color: u32, width: f32, unit: i32, out: *mut Ptr) -> i32;
    fn GdipDeletePen(pen: Ptr) -> i32;
    fn GdipSetPenDashStyle(pen: Ptr, style: i32) -> i32;
    fn GdipSetPenStartCap(pen: Ptr, cap: i32) -> i32;
    fn GdipSetPenEndCap(pen: Ptr, cap: i32) -> i32;
    fn GdipDrawPath(g: Ptr, pen: Ptr, path: Ptr) -> i32;
    fn GdipDrawLines(g: Ptr, pen: Ptr, points: *const Point, count: i32) -> i32;
    fn GdipCreateFontFamilyFromName(name: *const u16, collection: Ptr, out: *mut Ptr) -> i32;
    fn GdipDeleteFontFamily(family: Ptr) -> i32;
    fn GdipCreateFont(family: Ptr, size: f32, style: i32, unit: i32, out: *mut Ptr) -> i32;
    fn GdipDeleteFont(font: Ptr) -> i32;
    fn GdipCreateStringFormat(flags: i32, language: u16, out: *mut Ptr) -> i32;
    fn GdipStringFormatGetGenericTypographic(out: *mut Ptr) -> i32;
    fn GdipCloneStringFormat(format: Ptr, out: *mut Ptr) -> i32;
    fn GdipGetStringFormatFlags(format: Ptr, flags: *mut i32) -> i32;
    fn GdipSetStringFormatFlags(format: Ptr, flags: i32) -> i32;
    fn GdipDeleteStringFormat(format: Ptr) -> i32;
    fn GdipSetStringFormatAlign(format: Ptr, align: i32) -> i32;
    fn GdipSetStringFormatLineAlign(format: Ptr, align: i32) -> i32;
    fn GdipSetStringFormatTrimming(format: Ptr, trimming: i32) -> i32;
    fn GdipDrawString(
        g: Ptr,
        text: *const u16,
        len: i32,
        font: Ptr,
        rect: *const Rect,
        format: Ptr,
        brush: Ptr,
    ) -> i32;
    fn GdipMeasureString(g:Ptr,text:*const u16,len:i32,font:Ptr,layout:*const Rect,format:Ptr,bounds:*mut Rect,fit:*mut i32,lines:*mut i32)->i32;
    fn GdipDisposeImage(image: Ptr) -> i32;
    fn GdipCreateBitmapFromScan0(
        w: i32,
        h: i32,
        stride: i32,
        format: i32,
        data: *mut u8,
        out: *mut Ptr,
    ) -> i32;
    fn GdipTranslateWorldTransform(g: Ptr, x: f32, y: f32, order: i32) -> i32;
    fn GdipScaleWorldTransform(g: Ptr, x: f32, y: f32, order: i32) -> i32;
    fn GdipRotateWorldTransform(g: Ptr, angle: f32, order: i32) -> i32;
    fn GdipSetClipPath(g: Ptr, path: Ptr, mode: i32) -> i32;
    fn GdipResetClip(g: Ptr) -> i32;
    fn GdipSetClipRect(g: Ptr, x: f32, y: f32, w: f32, h: f32, mode: i32) -> i32;
    fn GdipCreatePathGradientFromPath(path: Ptr, out: *mut Ptr) -> i32;
    fn GdipSetPathGradientCenterColor(brush: Ptr, color: u32) -> i32;
    fn GdipSetPathGradientSurroundColorsWithCount(
        brush: Ptr,
        colors: *const u32,
        count: *mut i32,
    ) -> i32;
    fn GdipSetPathGradientCenterPoint(brush: Ptr, point: *const Point) -> i32;
    fn GdipCreateLineBrushFromRectWithAngle(
        rect: *const Rect,
        a: u32,
        b: u32,
        angle: f32,
        scalable: i32,
        wrap: i32,
        out: *mut Ptr,
    ) -> i32;
    fn GdipCreatePen2(brush: Ptr, width: f32, unit: i32, out: *mut Ptr) -> i32;
    fn GdipSetLineBlend(brush: Ptr, factors: *const f32, positions: *const f32, count: i32) -> i32;
}
struct CachedFont {
    font: Ptr,
    family: Ptr,
}
impl Drop for CachedFont {
    fn drop(&mut self) {
        unsafe {
            GdipDeleteFont(self.font);
            GdipDeleteFontFamily(self.family);
        }
    }
}
thread_local! {static REFLECTIONS:RefCell<HashMap<(u32,bool),Image>>=RefCell::new(HashMap::new());}
struct ShadowSurface {
    image: Option<Image>,
}
thread_local! {static GROUPS:RefCell<HashMap<(u32,u32,u32,u32),Image>>=RefCell::new(HashMap::new());}
thread_local! {static GROUP_POLYS:RefCell<HashMap<Vec<i32>,Image>>=RefCell::new(HashMap::new());}
thread_local! {static HOVERS:RefCell<HashMap<(u32,u32,u32,u32,bool),Image>>=RefCell::new(HashMap::new());}
#[inline]
pub(crate) fn stable_dim_px(delta_dip: f32, scale: f32) -> i32 {
    (((delta_dip * 64.0).round() / 64.0) * scale).round() as i32
}
#[inline]
fn group_surface_key(r: [f32; 4], radius: f32, scale: f32) -> (u32, u32, u32, u32) {
    (
        stable_dim_px(r[2] - r[0], scale).max(1) as u32,
        stable_dim_px(r[3] - r[1], scale).max(1) as u32,
        (radius * scale * 16.).round() as u32,
        (scale * 1000.).round() as u32,
    )
}
thread_local! {static GLOW_SPRITE:RefCell<Option<Image>>=RefCell::new(None);}
thread_local! {static SHADOWS:RefCell<HashMap<(u32,u32,u32,u32,u32),ShadowSurface>>=RefCell::new(HashMap::new());}
thread_local! {static FONTS:RefCell<HashMap<(i32,u16),CachedFont>>=RefCell::new(HashMap::new());}
thread_local! {static CUSTOM_FONTS:RefCell<HashMap<(String,i32,u8),CachedFont>>=RefCell::new(HashMap::new());}
unsafe fn font_at(size: f32, weight: u16) -> Ptr {
    FONTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = ((size * 64.).round() as i32, weight);
        if !cache.contains_key(&key) && cache.len() >= 128 {
            cache.clear();
        }
        let entry = cache
            .entry(key)
            .or_insert_with(|| {
                let base_weight = weight & 0x0fff;
                let extra_style = ((weight >> 12) & 0x0f) as i32;
                let mut family = null_mut();
                if GdipCreateFontFamilyFromName(
                    wide(if base_weight >= 3000 {
                        "Segoe UI Light"
                    } else if base_weight >= 2000 {
                        "Impact"
                    } else if base_weight >= 1000 {
                        "Consolas"
                    } else if base_weight == 600 {
                        "Segoe UI Semibold"
                    } else {
                        "Segoe UI Variable Text"
                    })
                    .as_ptr(),
                    null_mut(),
                    &mut family,
                ) != 0
                {
                    GdipCreateFontFamilyFromName(
                        wide("Segoe UI").as_ptr(),
                        null_mut(),
                        &mut family,
                    );
                }
                let mut font = null_mut();
                GdipCreateFont(
                    family,
                    size,
                    extra_style | if base_weight % 1000 >= 700 { 1 } else { 0 },
                    2,
                    &mut font,
                );
                CachedFont { font, family }
            });
        entry.font
    })
}
unsafe fn styled_font_at(family_name: Option<&str>, size: f32, style: u8) -> Ptr {
    let style = style & 0x0f;
    let Some(name) = family_name.map(str::trim).filter(|s| {
        !s.is_empty()
            && !s.eq_ignore_ascii_case("Segoe UI Variable Text")
            && !s.eq_ignore_ascii_case("Segoe UI")
    }) else {
        return font_at(size, 400 | ((style as u16) << 12));
    };
    CUSTOM_FONTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (name.to_ascii_lowercase(), (size * 64.).round() as i32, style);
        if !cache.contains_key(&key) && cache.len() >= 128 {
            cache.clear();
        }
        let entry = cache.entry(key).or_insert_with(|| {
            let mut family = null_mut();
            if GdipCreateFontFamilyFromName(wide(name).as_ptr(), null_mut(), &mut family) != 0
                && GdipCreateFontFamilyFromName(
                    wide("Segoe UI Variable Text").as_ptr(),
                    null_mut(),
                    &mut family,
                ) != 0
            {
                GdipCreateFontFamilyFromName(wide("Segoe UI").as_ptr(), null_mut(), &mut family);
            }
            let mut font = null_mut();
            GdipCreateFont(family, size, style as i32, 2, &mut font);
            CachedFont { font, family }
        });
        entry.font
    })
}
unsafe fn rich_string_format() -> Ptr {
    let mut base = null_mut();
    let mut format = null_mut();
    if GdipStringFormatGetGenericTypographic(&mut base) == 0 && !base.is_null() {
        GdipCloneStringFormat(base, &mut format);
    }
    if format.is_null() {
        GdipCreateStringFormat(0x0800 | 0x1000 | 0x4000, 0, &mut format);
    }
    if !format.is_null() {
        let mut flags = 0i32;
        if GdipGetStringFormatFlags(format, &mut flags) == 0 {
            // Measure trailing spaces and keep single-line runs typographic without glyph clipping.
            GdipSetStringFormatFlags(format, (flags | 0x0800 | 0x1000 | 0x4000) & !0x2000);
        }
        GdipSetStringFormatAlign(format, 0);
        GdipSetStringFormatLineAlign(format, 1);
        GdipSetStringFormatTrimming(format, 0);
    }
    format
}
pub struct Runtime(usize);
impl Runtime {
    pub fn start() -> Result<Self, String> {
        unsafe {
            let mut token = 0;
            let input = Startup {
                version: 1,
                callback: null_mut(),
                no_thread: 0,
                no_codecs: 0,
            };
            if GdiplusStartup(&mut token, &input, null_mut()) != 0 {
                return Err("Cannot initialize the native graphics renderer".into());
            }
            Ok(Self(token))
        }
    }
}
thread_local! {
    static OVERLAYS: RefCell<HashMap<([u32; 2], u32, i32, bool), std::rc::Rc<Image>>> =
        RefCell::new(HashMap::new());
    static GRAIN_TILE: RefCell<Option<Image>> = RefCell::new(None);
}
pub fn clear_caches() {
    crate::surface::clear_cache();
    crate::lenses::clear_cache();
    crate::settings_ui::clear_cache();
    crate::compositor::clear_group_rim_cache();
    SHADOWS.with(|shadows| shadows.borrow_mut().clear());
    GROUPS.with(|groups| groups.borrow_mut().clear());
    GROUP_POLYS.with(|groups| groups.borrow_mut().clear());
    HOVERS.with(|hovers| hovers.borrow_mut().clear());
    GLOW_SPRITE.with(|glow| *glow.borrow_mut() = None);
    REFLECTIONS.with(|cache| cache.borrow_mut().clear());
    OVERLAYS.with(|cache| cache.borrow_mut().clear());
    GRAIN_TILE.with(|tile| *tile.borrow_mut() = None);
}
impl Drop for Runtime {
    fn drop(&mut self) {
        clear_caches();
        FONTS.with(|fonts| fonts.borrow_mut().clear());
        CUSTOM_FONTS.with(|fonts| fonts.borrow_mut().clear());
        unsafe {
            GdiplusShutdown(self.0);
        }
    }
}
pub(crate) fn recover_icon_alpha(black: &[u8], white: &[u8]) -> Vec<u8> {
    black
        .chunks_exact(4)
        .zip(white.chunks_exact(4))
        .flat_map(|(b, w)| {
            let transparency = (0..3)
                .map(|c| w[c].saturating_sub(b[c]) as u32)
                .sum::<u32>()
                / 3;
            let alpha = 255 - transparency as u8;
            [b[2].min(alpha), b[1].min(alpha), b[0].min(alpha), alpha]
        })
        .collect()
}
pub struct Image {
    raw: Ptr,
    _pixels: Option<Vec<u8>>,
    width: u32,
    height: u32,
    tint: Option<[f32; 3]>,
    horizontal_parts: RefCell<Option<(u32, Vec<Image>)>>,
}
impl Image {
    pub unsafe fn from_icon(
        icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON,
    ) -> Option<Self> {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let mut info: ICONINFO = std::mem::zeroed();
        if GetIconInfo(icon, &mut info) == 0 {
            return None;
        }
        let mut bitmap: BITMAP = std::mem::zeroed();
        let color = !info.hbmColor.is_null();
        GetObjectW(
            if color { info.hbmColor } else { info.hbmMask },
            std::mem::size_of::<BITMAP>() as i32,
            &mut bitmap as *mut _ as _,
        );
        let width = bitmap.bmWidth.clamp(1, 1024);
        let height = (bitmap.bmHeight / if color { 1 } else { 2 }).clamp(1, 1024);
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask);
        }
        let buffer = crate::compositor::Buffer::plain(width, height, 1.)?;
        PatBlt(buffer.dc, 0, 0, width, height, BLACKNESS);
        DrawIconEx(
            buffer.dc,
            0,
            0,
            icon,
            width,
            height,
            0,
            null_mut(),
            DI_NORMAL,
        );
        let black = buffer.snapshot();
        PatBlt(buffer.dc, 0, 0, width, height, WHITENESS);
        DrawIconEx(
            buffer.dc,
            0,
            0,
            icon,
            width,
            height,
            0,
            null_mut(),
            DI_NORMAL,
        );
        let white = buffer.snapshot();
        Self::from_rgba(
            width as u32,
            height as u32,
            recover_icon_alpha(&black, &white),
        )
    }
    pub unsafe fn from_hbitmap(
        hbitmap: windows_sys::Win32::Graphics::Gdi::HBITMAP,
    ) -> Option<Self> {
        if hbitmap.is_null() {
            return None;
        }
        let mut bitmap: BITMAP = std::mem::zeroed();
        if GetObjectW(
            hbitmap,
            std::mem::size_of::<BITMAP>() as i32,
            &mut bitmap as *mut _ as _,
        ) == 0
        {
            return None;
        }
        let width = bitmap.bmWidth;
        let height = bitmap.bmHeight.abs();
        if width <= 0 || height <= 0 || width > 2048 || height > 2048 {
            return None;
        }
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width;
        bmi.bmiHeader.biHeight = -height;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        let mut bgra = vec![0u8; (width as usize) * (height as usize) * 4];
        let dc = GetDC(null_mut());
        let rows = GetDIBits(
            dc,
            hbitmap,
            0,
            height as u32,
            bgra.as_mut_ptr() as _,
            &mut bmi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(null_mut(), dc);
        if rows == 0 {
            return None;
        }
        let opaque_fallback = bgra.chunks_exact(4).all(|p| p[3] == 0);
        let w = width as u32;
        let h = height as u32;
        let side = w.max(h);
        let ox = (side - w) / 2;
        let oy = (side - h) / 2;
        let mut rgba = vec![0u8; (side as usize) * (side as usize) * 4];
        for y in 0..h {
            for x in 0..w {
                let src = ((y * w + x) as usize) * 4;
                let dst = (((y + oy) * side + (x + ox)) as usize) * 4;
                let a = if opaque_fallback { 255 } else { bgra[src + 3] };
                rgba[dst] = bgra[src + 2].min(a);
                rgba[dst + 1] = bgra[src + 1].min(a);
                rgba[dst + 2] = bgra[src].min(a);
                rgba[dst + 3] = a;
            }
        }
        Self::from_rgba(side, side, rgba)
    }
    pub fn opaque_bounds(&self) -> [u32; 4] {
        self.bounds_above(8, 0)
    }
    fn bounds_above(&self, threshold: u8, margin: u32) -> [u32; 4] {
        let Some(pixels) = &self._pixels else {
            return [0, 0, self.width, self.height];
        };
        if self.width <= margin * 2 || self.height <= margin * 2 {
            return [0, 0, 0, 0];
        }
        let (mut min_x, mut min_y) = (self.width, self.height);
        let (mut max_x, mut max_y) = (0u32, 0u32);
        for y in margin..self.height - margin {
            for x in margin..self.width - margin {
                let a = pixels[((y * self.width + x) as usize) * 4 + 3];
                if a > threshold {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x + 1);
                    max_y = max_y.max(y + 1);
                }
            }
        }
        if max_x <= min_x || max_y <= min_y {
            [0, 0, 0, 0]
        } else {
            [min_x, min_y, max_x, max_y]
        }
    }
    /// Detect real glyph bounds even when Windows packs a 16-64px icon inside
    /// a 256x256 jumbo frame with a faint outer border.
    pub fn effective_icon_bounds(&self) -> [u32; 4] {
        let full = {
            let b = self.bounds_above(20, 0);
            if b[2] > b[0] && b[3] > b[1] {
                b
            } else {
                self.bounds_above(8, 0)
            }
        };
        let min_dim = self.width.min(self.height);
        let full_max = full[2].saturating_sub(full[0]).max(full[3].saturating_sub(full[1]));
        if let Some(pixels) = self._pixels.as_ref().filter(|_| {
            self.width >= 128 && self.height >= 128 && full_max >= (min_dim * 85) / 100
        }) {
            let inner = self.bounds_above(20, 8);
            let iw = inner[2].saturating_sub(inner[0]);
            let ih = inner[3].saturating_sub(inner[1]);
            let inner_max = iw.max(ih);
            if iw > 0
                && ih > 0
                && inner_max <= (min_dim * 48) / 100
                && inner[0] >= 12
                && inner[1] >= 12
                && inner[2] <= self.width - 12
                && inner[3] <= self.height - 12
            {
                let mx0 = inner[0].saturating_sub(5).max(8);
                let my0 = inner[1].saturating_sub(5).max(8);
                let mx1 = (inner[2] + 5).min(self.width - 8);
                let my1 = (inner[3] + 5).min(self.height - 8);
                let mut outer_opaque = 0u64;
                let mut outer_total = 0u64;
                for y in 0..self.height {
                    for x in 0..self.width {
                        if x < mx0 || x >= mx1 || y < my0 || y >= my1 {
                            outer_total += 1;
                            if pixels[((y * self.width + x) as usize) * 4 + 3] > 20 {
                                outer_opaque += 1;
                            }
                        }
                    }
                }
                let mut moat_opaque = 0u64;
                let mut moat_total = 0u64;
                for y in my0..my1 {
                    for x in mx0..mx1 {
                        if x < inner[0].saturating_sub(1)
                            || x >= inner[2] + 1
                            || y < inner[1].saturating_sub(1)
                            || y >= inner[3] + 1
                        {
                            moat_total += 1;
                            if pixels[((y * self.width + x) as usize) * 4 + 3] > 20 {
                                moat_opaque += 1;
                            }
                        }
                    }
                }
                if outer_total > 0
                    && outer_opaque * 100 < outer_total * 15
                    && moat_total > 0
                    && moat_opaque * 100 < moat_total * 5
                {
                    return inner;
                }
            }
        }
        full
    }
    pub fn is_jumbo_packed_small(&self) -> bool {
        if self.width < 128 || self.height < 128 {
            return false;
        }
        let [x0, y0, x1, y1] = self.effective_icon_bounds();
        x1.saturating_sub(x0).max(y1.saturating_sub(y0)) <= 64
    }
    pub fn max_opaque_extent(&self) -> [u32; 2] {
        let [x0, y0, x1, y1] = self.opaque_bounds();
        [x1.saturating_sub(x0), y1.saturating_sub(y0)]
    }
    /// Crop excessive transparent padding when Windows packs a smaller
    /// file/app glyph inside a 256x256 jumbo/shell canvas.
    pub fn trim_padded_icon(self) -> Self {
        let [x0, y0, x1, y1] = self.effective_icon_bounds();
        let bw = x1.saturating_sub(x0);
        let bh = y1.saturating_sub(y0);
        let min_dim = self.width.min(self.height);
        if bw == 0 || bh == 0 || min_dim == 0 || bw.max(bh) > (min_dim * 84) / 100 {
            return self;
        }
        let Some(pixels) = &self._pixels else {
            return self;
        };
        let side = ((bw.max(bh) as f32) * 1.08).ceil().max(1.0) as u32;
        let cx = (x0 + x1) as i32 / 2;
        let cy = (y0 + y1) as i32 / 2;
        let sx0 = cx - side as i32 / 2;
        let sy0 = cy - side as i32 / 2;
        let mut bgra = vec![0u8; (side as usize) * (side as usize) * 4];
        for dy in 0..side {
            let sy = sy0 + dy as i32;
            if sy < 0 || sy >= self.height as i32 {
                continue;
            }
            for dx in 0..side {
                let sx = sx0 + dx as i32;
                if sx < 0 || sx >= self.width as i32 {
                    continue;
                }
                let src = ((sy as u32 * self.width + sx as u32) as usize) * 4;
                let dst = ((dy * side + dx) as usize) * 4;
                bgra[dst..dst + 4].copy_from_slice(&pixels[src..src + 4]);
            }
        }
        Self::from_bgra(side, side, bgra).unwrap_or(self)
    }
    pub fn load(path: &Path) -> Option<Self> {
        let decoded = image::open(path).ok()?.into_rgba8();
        let (width, height) = decoded.dimensions();
        let mut pixels = decoded.into_raw();
        for p in pixels.chunks_exact_mut(4) {
            for c in 0..3 {
                p[c] = ((p[c] as u32 * p[3] as u32 + 127) / 255) as u8;
            }
        }
        Self::from_rgba(width, height, pixels)
    }
    fn refresh_tint(&mut self) {
        self.tint = self._pixels.as_ref().and_then(|pixels| {
            let pixel = pixels.chunks_exact(4).max_by_key(|p| p[3])?;
            if pixel[3] == 0 {
                return Some([0.; 3]);
            }
            let tint = [
                pixel[0] as f32 / pixel[3] as f32,
                pixel[1] as f32 / pixel[3] as f32,
                pixel[2] as f32 / pixel[3] as f32,
            ];
            pixels
                .chunks_exact(4)
                .all(|p| (0..3).all(|c| (p[c] as f32 - tint[c] * p[3] as f32).abs() <= 1.1))
                .then_some(tint)
        });
    }
    pub fn pixel_size(&self) -> [u32; 2] {
        [self.width, self.height]
    }
    pub fn identity(&self) -> usize {
        self.raw as usize
    }
    pub fn shadow(&self, blur: f32, strength: u8, pad: u32) -> Option<Self> {
        let source = self._pixels.as_ref()?;
        let width = self.width + pad * 2;
        let height = self.height + pad * 2;
        let mut alpha = vec![0.; (width * height) as usize];
        for y in 0..self.height {
            for x in 0..self.width {
                alpha[((y + pad) * width + x + pad) as usize] =
                    source[(y * self.width + x) as usize * 4 + 3] as f32;
            }
        }
        let radius = (blur * 2.5).ceil().max(1.) as i32;
        let mut weights: Vec<f32> = (-radius..=radius)
            .map(|i| (-0.5 * (i as f32 / blur.max(0.2)).powi(2)).exp())
            .collect();
        let sum: f32 = weights.iter().sum();
        for w in &mut weights {
            *w /= sum;
        }
        let mut horizontal = vec![0.; alpha.len()];
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let index = (y * width as i32 + x) as usize;
                for (i, weight) in (-radius..=radius).zip(&weights) {
                    let sx = x + i;
                    if sx >= 0 && sx < width as i32 {
                        horizontal[index] += alpha[(y * width as i32 + sx) as usize] * weight;
                    }
                }
            }
        }
        let mut pixels = vec![0; alpha.len() * 4];
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                let mut value = 0.;
                for (i, weight) in (-radius..=radius).zip(&weights) {
                    let sy = y + i;
                    if sy >= 0 && sy < height as i32 {
                        value += horizontal[(sy * width as i32 + x) as usize] * weight;
                    }
                }
                pixels[(y * width as i32 + x) as usize * 4 + 3] =
                    (value * strength as f32 / 255.).round().clamp(0., 255.) as u8;
            }
        }
        Self::from_rgba(width, height, pixels)
    }
    pub fn icon_shadow(&self) -> Option<(u32, Self)> {
        let source = self._pixels.as_ref()?;
        let coverage = source.chunks_exact(4).map(|p| p[3] as f32).sum::<f32>()
            / (self.width * self.height) as f32
            / 255.;
        let sigma = self.width.max(self.height) as f32 * 0.36;
        let reach = sigma * 3.;
        let pad = (reach - self.width.min(self.height) as f32 * 0.5)
            .ceil()
            .max(0.) as u32
            + 2;
        let width = self.width + pad * 2;
        let height = self.height + pad * 2;
        let mut pixels = vec![0; (width * height * 4) as usize];
        let strength = 70. * coverage.sqrt();
        let tail = (-4.5f32).exp();
        for y in 0..height {
            for x in 0..width {
                let dx = x as f32 + 0.5 - width as f32 * 0.5;
                let dy = y as f32 + 0.5 - height as f32 * 0.5;
                let falloff = ((-0.5 * (dx * dx + dy * dy) / (sigma * sigma)).exp() - tail).max(0.)
                    / (1. - tail);
                pixels[((y * width + x) * 4 + 3) as usize] = (strength * falloff).round() as u8;
            }
        }
        Some((pad, Self::from_rgba(width, height, pixels)?))
    }
    pub fn resized(&self, size: u32) -> Option<Self> {
        let source =
            image::RgbaImage::from_raw(self.width, self.height, self._pixels.as_ref()?.clone())?;
        let mut pixels =
            image::imageops::resize(&source, size, size, image::imageops::FilterType::Lanczos3)
                .into_raw();
        for p in pixels.chunks_exact_mut(4) {
            for c in 0..3 {
                p[c] = p[c].min(p[3]);
            }
            p.swap(0, 2);
        }
        Self::from_rgba(size, size, pixels)
    }
    pub fn from_rgba(width: u32, height: u32, mut pixels: Vec<u8>) -> Option<Self> {
        if pixels.len() != width as usize * height as usize * 4 {
            return None;
        }
        for p in pixels.chunks_exact_mut(4) {
            p.swap(0, 2);
        }
        Self::from_bgra(width, height, pixels)
    }
    pub fn from_bgra(width: u32, height: u32, mut pixels: Vec<u8>) -> Option<Self> {
        if pixels.len() != width as usize * height as usize * 4 {
            return None;
        }
        unsafe {
            let mut image = null_mut();
            if GdipCreateBitmapFromScan0(
                width as i32,
                height as i32,
                width as i32 * 4,
                0xe200b,
                pixels.as_mut_ptr(),
                &mut image,
            ) != 0
            {
                return None;
            }
            let mut result = Self {
                raw: image,
                _pixels: Some(pixels),
                width,
                height,
                tint: None,
                horizontal_parts: RefCell::new(None),
            };
            result.refresh_tint();
            Some(result)
        }
    }
}
impl Drop for Image {
    fn drop(&mut self) {
        unsafe {
            GdipDisposeImage(self.raw);
        }
    }
}
pub fn argb(color: u32, alpha: u8) -> u32 {
    ((alpha as u32) << 24) | ((color & 255) << 16) | (color & 0xff00) | ((color >> 16) & 255)
}
pub fn label_rect(image: &Image, center: f32, y: f32, dpi: f32, snap: bool) -> [f32; 4] {
    let width = image.width as f32 / dpi;
    let height = image.height as f32 / dpi;
    let mut x = center - width * 0.5;
    let mut y = y;
    if snap {
        x = (center * dpi - image.width as f32 * 0.5).round() / dpi;
        y = (y * dpi).round() / dpi;
    }
    [x, y, x + width, y + height]
}
pub fn label_clip(width: f32, center: f32, y: f32, dpi: f32, snap: bool) -> [f32; 4] {
    let pixels = (width * dpi).round();
    let width = pixels / dpi;
    let mut x = center - width * 0.5;
    let mut y = y;
    if snap {
        x = (center * dpi - pixels * 0.5).round() / dpi;
        y = (y * dpi).round() / dpi;
    }
    [x, y, x + width, y + (20. * dpi).round() / dpi]
}
thread_local! {static COST:Cell<[u64; 4]>=const{Cell::new([0;4])};}
pub fn take_cost() -> [u64; 4] {
    COST.with(|c| c.replace([0; 4]))
}
struct Cost(usize, std::time::Instant);
impl Cost {
    fn new(i: usize) -> Self {
        Self(i, std::time::Instant::now())
    }
}
impl Drop for Cost {
    fn drop(&mut self) {
        COST.with(|c| {
            let mut v = c.get();
            v[self.0] += self.1.elapsed().as_nanos() as u64;
            c.set(v);
        });
    }
}
thread_local! {static ACTIVE_BATCH:Cell<Option<(usize,usize)>>=const{Cell::new(None)};}
thread_local! {static ACTIVE_BATCH_CLIP:Cell<[i32;4]>=const{Cell::new([0;4])};}
thread_local! {static BATCH_DIRTY:Cell<bool>=const{Cell::new(true)};}
thread_local! {static ALPHA_DC:Cell<Option<usize>>=const{Cell::new(None)};}
pub fn has_alpha(dc: HDC) -> bool {
    ALPHA_DC.with(|a| a.get() == Some(dc as usize))
}
pub unsafe fn flush(dc: HDC) {
    if let Some((handle, g)) = ACTIVE_BATCH
        .with(|a| a.get())
        .filter(|(h, _)| *h == dc as usize)
    {
        let _ = handle;
        if g != 0 {
            GdipFlush(g as Ptr, 1);
        }
    }
    GdiFlush();
}
pub struct AlphaBatch {
    batch: Option<Batch>,
    bitmap: Ptr,
    previous: Option<usize>,
}
impl AlphaBatch {
    pub unsafe fn new(dc: HDC, dpi: f32) -> Option<Self> {
        let mut dib: DIBSECTION = std::mem::zeroed();
        if GetObjectW(
            GetCurrentObject(dc, OBJ_BITMAP as u32),
            std::mem::size_of::<DIBSECTION>() as i32,
            &mut dib as *mut _ as _,
        ) == 0
            || dib.dsBm.bmBits.is_null()
        {
            return None;
        }
        GdiFlush();
        let mut bitmap = null_mut();
        if GdipCreateBitmapFromScan0(
            dib.dsBm.bmWidth,
            dib.dsBm.bmHeight.abs(),
            dib.dsBm.bmWidthBytes,
            0xe200b,
            dib.dsBm.bmBits as _,
            &mut bitmap,
        ) != 0
        {
            return None;
        }
        let mut g = null_mut();
        if GdipGetImageGraphicsContext(bitmap, &mut g) != 0 {
            GdipDisposeImage(bitmap);
            return None;
        }
        GdipSetSmoothingMode(g, 4);
        GdipSetTextRenderingHint(g, 3);
        GdipSetInterpolationMode(g, 3);
        GdipSetPixelOffsetMode(g, 4);
        let previous = ALPHA_DC.with(|a| a.replace(Some(dc as usize)));
        let previous_batch = ACTIVE_BATCH.with(|a| a.replace(Some((dc as usize, g as usize))));
        let previous_clip =
            ACTIVE_BATCH_CLIP.with(|c| c.replace([i32::MIN, i32::MIN, i32::MAX, i32::MAX]));
        Some(Self {
            batch: Some(Batch {
                scale: dpi,
                target: target_for_dc(dc),
                owns_g: true,
                previous: previous_batch,
                previous_clip,
            }),
            bitmap,
            previous,
        })
    }
}
impl Drop for AlphaBatch {
    fn drop(&mut self) {
        drop(self.batch.take());
        unsafe {
            GdipDisposeImage(self.bitmap);
        }
        ALPHA_DC.with(|a| a.set(self.previous));
    }
}
unsafe fn target_for_dc(dc: HDC) -> Option<crate::sampling::Target> {
    let mut dib: DIBSECTION = std::mem::zeroed();
    let bitmap = GetCurrentObject(dc, OBJ_BITMAP as u32);
    let mut clip = std::mem::zeroed();
    GetClipBox(dc, &mut clip);
    if GetObjectW(
        bitmap,
        std::mem::size_of::<DIBSECTION>() as i32,
        &mut dib as *mut _ as _,
    ) == std::mem::size_of::<DIBSECTION>() as i32
        && !dib.dsBm.bmBits.is_null()
        && dib.dsBm.bmBitsPixel == 32
        && dib.dsBm.bmWidthBytes == dib.dsBm.bmWidth * 4
    {
        Some(crate::sampling::Target {
            bits: dib.dsBm.bmBits as _,
            width: dib.dsBm.bmWidth as usize,
            height: dib.dsBm.bmHeight.unsigned_abs() as usize,
            clip: [clip.left, clip.top, clip.right, clip.bottom],
            opaque: !ALPHA_DC.with(|a| a.get() == Some(dc as usize)),
        })
    } else {
        None
    }
}
pub struct Batch {
    scale: f32,
    target: Option<crate::sampling::Target>,
    owns_g: bool,
    previous: Option<(usize, usize)>,
    previous_clip: [i32; 4],
}
impl Batch {
    pub unsafe fn new(dc: HDC, dpi: f32) -> Self {
        GdiFlush();
        let existing = ACTIVE_BATCH
            .with(|a| a.get())
            .filter(|(handle, _)| *handle == dc as usize);
        let (owns_g, previous, previous_clip) = if let Some((_, g)) = existing {
            (
                false,
                ACTIVE_BATCH.with(|a| a.replace(Some((dc as usize, g)))),
                ACTIVE_BATCH_CLIP.with(|c| c.get()),
            )
        } else {
            BATCH_DIRTY.with(|d| d.set(false));
            (
                true,
                ACTIVE_BATCH.with(|a| a.replace(Some((dc as usize, 0)))),
                ACTIVE_BATCH_CLIP.with(|c| c.replace([0; 4])),
            )
        };
        Self {
            scale: dpi,
            target: target_for_dc(dc),
            owns_g,
            previous,
            previous_clip,
        }
    }
    pub fn blit(&self, image: &Image, r: [f32; 4], snapped: bool) -> bool {
        self.blit_alpha(image, r, 1., snapped)
    }
    pub fn blit_alpha(&self, image: &Image, r: [f32; 4], opacity: f32, snapped: bool) -> bool {
        let (Some(target), Some(pixels)) = (self.target, image._pixels.as_ref()) else {
            return false;
        };
        if opacity <= 0. {
            return true;
        }
        let _cost = Cost::new(2);
        let scale = self.scale;
        let w = image.width as f32;
        let h = image.height as f32;
        let x0 = r[0] * scale;
        let y0 = r[1] * scale;
        let x = if snapped || (x0 - x0.round()).abs() < 0.01 {
            x0.round()
        } else {
            x0
        };
        let y = if snapped { y0.round() } else { y0 };
        let r = [x, y, x + w, y + h];
        unsafe {
            if BATCH_DIRTY.with(|d| d.replace(false)) {
                if let Some((_, g)) = ACTIVE_BATCH.with(|a| a.get()).filter(|(_, g)| *g != 0) {
                    GdipFlush(g as Ptr, 1);
                }
                GdiFlush();
            }
            target.draw(
                pixels,
                [image.width as usize, image.height as usize],
                r,
                [0., 0., 1., 0.],
                opacity,
                image.tint,
            );
        }
        true
    }
    pub fn blit_scaled(&self, image: &Image, r: [f32; 4], center: [f32; 2], scale: f32) -> bool {
        let (Some(target), Some(pixels)) = (self.target, image._pixels.as_ref()) else {
            return false;
        };
        let _cost = Cost::new(2);
        let dpi = self.scale;
        let r_px = r.map(|v| v * dpi);
        let cx_px = center[0] * dpi;
        let cy_px = center[1] * dpi;
        unsafe {
            if BATCH_DIRTY.with(|d| d.replace(false)) {
                if let Some((_, g)) = ACTIVE_BATCH.with(|a| a.get()).filter(|(_, g)| *g != 0) {
                    GdipFlush(g as Ptr, 1);
                }
                GdiFlush();
            }
            target.draw(
                pixels,
                [image.width as usize, image.height as usize],
                r_px,
                [cx_px, cy_px, scale, 0.],
                1.,
                image.tint,
            );
        }
        true
    }
    pub fn hover_surface(&self, r: [f32; 4], radius: f32, amount: f32, bright: bool) -> bool {
        if self.target.is_none() {
            return false;
        }
        if amount <= 0.001 {
            return true;
        }
        let _cost = Cost::new(3);
        let w_px = stable_dim_px(r[2] - r[0], self.scale).max(1) as u32;
        let h_px = stable_dim_px(r[3] - r[1], self.scale).max(1) as u32;
        let key = (
            w_px,
            h_px,
            (radius * self.scale * 16.).round() as u32,
            (self.scale * 1000.).round() as u32,
            bright,
        );
        let pad = (15. * self.scale).ceil() as u32;
        let p = pad as f32 / self.scale;
        HOVERS.with(|cache| {
            let mut cache = cache.borrow_mut();
            if !cache.contains_key(&key) {
                if cache.len() >= 32 {
                    cache.clear();
                }
                let width = key.0 + pad * 2;
                let height = key.1 + pad * 2;
                if let Some(mut image) =
                    Image::from_rgba(width, height, vec![0; (width * height * 4) as usize])
                {
                    unsafe {
                        if let Some(canvas) = Canvas::on_image(&image, self.scale) {
                            canvas.hover_surface(
                                [p, p, p + w_px as f32 / self.scale, p + h_px as f32 / self.scale],
                                radius,
                                1.0,
                                bright,
                            );
                        }
                    }
                    image.refresh_tint();
                    cache.insert(key, image);
                }
            }
            let sx = ((r[0] - p) * self.scale).round() / self.scale;
            cache.get(&key).is_some_and(|image| {
                self.blit_alpha(
                    image,
                    [sx, r[1] - p, sx + (r[2] - r[0]) + p * 2.0, r[3] + p],
                    amount.clamp(0., 1.),
                    false,
                )
            })
        })
    }
    pub fn hover_glow(&self, r: [f32; 4], radius: f32, pointer: [f32; 2], alpha: u8) -> bool {
        let Some(target) = self.target else {
            return false;
        };
        if alpha == 0 {
            return true;
        }
        let _cost = Cost::new(2);
        let scale = self.scale;
        let rx0 = r[0] * scale;
        let ry0 = r[1] * scale;
        let rx1 = r[2] * scale;
        let ry1 = r[3] * scale;
        let rw = rx1 - rx0;
        let rh = ry1 - ry0;
        if rw <= 0. || rh <= 0. {
            return true;
        }
        let rad = (radius * scale).min(rw * 0.5).min(rh * 0.5).max(0.);
        let cx = pointer[0] * scale;
        let cy = pointer[1] * scale;
        let glow_r = 60. * scale;
        if glow_r <= 0. {
            return true;
        }
        let left = (rx0.max(cx - glow_r).floor() as i32)
            .max(target.clip[0])
            .max(0);
        let top = (ry0.max(cy - glow_r).floor() as i32)
            .max(target.clip[1])
            .max(0);
        let right = (rx1.min(cx + glow_r).ceil() as i32)
            .min(target.clip[2])
            .min(target.width as i32);
        let bottom = (ry1.min(cy + glow_r).ceil() as i32)
            .min(target.clip[3])
            .min(target.height as i32);
        if left >= right || top >= bottom {
            return true;
        }
        unsafe {
            if BATCH_DIRTY.with(|d| d.replace(false)) {
                if let Some((_, g)) = ACTIVE_BATCH.with(|a| a.get()).filter(|(_, g)| *g != 0) {
                    GdipFlush(g as Ptr, 1);
                }
                GdiFlush();
            }
        }
        GLOW_SPRITE.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                let mut pixels = vec![0u8; 128 * 128 * 4];
                for y in 0..128u32 {
                    let dy = (y as f32 + 0.5 - 64.) / 64.;
                    for x in 0..128u32 {
                        let dx = (x as f32 + 0.5 - 64.) / 64.;
                        let d = (dx * dx + dy * dy).sqrt();
                        let v = (1. - d).max(0.);
                        let a = (v * 255.).round() as u8;
                        let i = ((y * 128 + x) * 4) as usize;
                        pixels[i] = a;
                        pixels[i + 1] = a;
                        pixels[i + 2] = a;
                        pixels[i + 3] = a;
                    }
                }
                *slot = Image::from_rgba(128, 128, pixels);
            }
            let Some(sprite) = slot.as_ref() else {
                return false;
            };
            let Some(src) = sprite._pixels.as_ref() else {
                return false;
            };
            let dest = unsafe {
                std::slice::from_raw_parts_mut(target.bits, target.width * target.height * 4)
            };
            let channels = if target.opaque { 3 } else { 4 };
            let alpha_scale = alpha as f32 / 255.;
            let inv_glow_r = 64. / glow_r;
            let glow_r2 = glow_r * glow_r;
            for y in top..bottom {
                let py = y as f32 + 0.5;
                let dy = py - cy;
                let v = dy * inv_glow_r + 64. - 0.5;
                if v < -1.0 || v >= 128.0 {
                    continue;
                }
                let iy = if v < 0.0 { -1 } else { v as i32 };
                let fy = v - iy as f32;
                let wy0 = 1. - fy;
                let wy1 = fy;
                let local_y = py - ry0;
                let row_offset = y as usize * target.width * 4;
                for x in left..right {
                    let px = x as f32 + 0.5;
                    let dx = px - cx;
                    if dx * dx + dy * dy >= glow_r2 {
                        continue;
                    }
                    let dist = crate::compositor::distance(px - rx0, local_y, rw, rh, rad, 0.);
                    let coverage = (0.5 - dist).clamp(0., 1.);
                    if coverage <= 0. {
                        continue;
                    }
                    let u = dx * inv_glow_r + 64. - 0.5;
                    if u < -1.0 || u >= 128.0 {
                        continue;
                    }
                    let ix = if u < 0.0 { -1 } else { u as i32 };
                    let fx = u - ix as f32;
                    let wx0 = 1. - fx;
                    let wx1 = fx;
                    let mut sample_a = 0f32;
                    for (qx, qy, w) in [
                        (ix, iy, wx0 * wy0),
                        (ix + 1, iy, wx1 * wy0),
                        (ix, iy + 1, wx0 * wy1),
                        (ix + 1, iy + 1, wx1 * wy1),
                    ] {
                        if qx >= 0 && qy >= 0 && qx < 128 && qy < 128 {
                            sample_a += src[((qy as usize * 128 + qx as usize) * 4) + 3] as f32 * w;
                        }
                    }
                    let a = (sample_a * alpha_scale * coverage + 0.5) as u32;
                    if a == 0 {
                        continue;
                    }
                    let a = a.min(255);
                    let inv = 255 - a;
                    let dst = row_offset + x as usize * 4;
                    for c in 0..channels {
                        dest[dst + c] =
                            (a + (dest[dst + c] as u32 * inv + 127) / 255).min(255) as u8;
                    }
                }
            }
            true
        })
    }
    pub fn group_surface(&self, r: [f32; 4], radius: f32, snapped: bool) -> bool {
        let _cost = Cost::new(3);
        let key = group_surface_key(r, radius, self.scale);
        let pad = (14. * self.scale).ceil() as u32;
        let p = pad as f32 / self.scale;
        GROUPS.with(|cache| {
            let mut cache = cache.borrow_mut();
            if !cache.contains_key(&key) {
                if cache.len() >= 32 {
                    cache.clear();
                }
                let width = key.0 + pad * 2;
                let height = key.1 + pad * 2;
                if let Some(mut image) =
                    Image::from_rgba(width, height, vec![0; (width * height * 4) as usize])
                {
                    unsafe {
                        if let Some(canvas) = Canvas::on_image(&image, self.scale) {
                            canvas.draw_group_material(
                                [p, p, p + r[2] - r[0], p + r[3] - r[1]],
                                radius,
                            );
                        }
                    }
                    image.refresh_tint();
                    cache.insert(key, image);
                }
            }
            cache
                .get(&key)
                .is_some_and(|image| self.blit(image, [r[0] - p, r[1] - p, r[2] + p, r[3] + p], snapped))
        })
    }
    pub fn group_poly_surface(&self, spans: &[[f32; 4]], radius: f32, snapped: bool) -> bool {
        if spans.is_empty() {
            return true;
        }
        if spans.len() == 1 {
            return self.group_surface(spans[0], radius, snapped);
        }
        let _cost = Cost::new(3);
        with_cached_group_poly(spans, radius, self.scale, |image, dest| self.blit(image, dest, snapped))
    }
}
pub fn split_connected_group_spans(spans: &[[f32; 4]], min_overlap: f32) -> Vec<Vec<[f32; 4]>> {
    if spans.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut cur = vec![spans[0]];
    for s in &spans[1..] {
        let prev = *cur.last().unwrap();
        let overlap = prev[2].min(s[2]) - prev[0].max(s[0]);
        if overlap >= min_overlap {
            cur.push(*s);
        } else {
            out.push(cur);
            cur = vec![*s];
        }
    }
    out.push(cur);
    out
}
pub fn ortho_group_polygon(spans: &[[f32; 4]]) -> Vec<[f32; 2]> {
    if spans.is_empty() {
        return Vec::new();
    }
    let mut raw = Vec::with_capacity(spans.len() * 4);
    raw.push([spans[0][2], spans[0][1]]);
    for k in 0..spans.len() - 1 {
        let (rx, by) = (spans[k][2], spans[k][3]);
        let (nrx, nty) = (spans[k + 1][2], spans[k + 1][1]);
        if (nrx - rx).abs() >= 0.5 {
            if nrx < rx {
                raw.push([rx, by]);
                raw.push([nrx, by]);
            } else {
                raw.push([rx, nty]);
                raw.push([nrx, nty]);
            }
        }
    }
    let last = spans[spans.len() - 1];
    raw.push([last[2], last[3]]);
    raw.push([last[0], last[3]]);
    for k in (0..spans.len() - 1).rev() {
        let (nlx, nty) = (spans[k + 1][0], spans[k + 1][1]);
        let (lx, by) = (spans[k][0], spans[k][3]);
        if (nlx - lx).abs() >= 0.5 {
            if nlx < lx {
                raw.push([nlx, nty]);
                raw.push([lx, nty]);
            } else {
                raw.push([nlx, by]);
                raw.push([lx, by]);
            }
        }
    }
    raw.push([spans[0][0], spans[0][1]]);
    let mut clean: Vec<[f32; 2]> = Vec::with_capacity(raw.len());
    for pt in raw {
        if clean
            .last()
            .is_none_or(|prev| (prev[0] - pt[0]).abs() > 0.01 || (prev[1] - pt[1]).abs() > 0.01)
        {
            clean.push(pt);
        }
    }
    if clean.len() > 1 {
        let first = clean[0];
        let last_pt = *clean.last().unwrap();
        if (first[0] - last_pt[0]).abs() <= 0.01 && (first[1] - last_pt[1]).abs() <= 0.01 {
            clean.pop();
        }
    }
    let mut changed = true;
    while changed && clean.len() > 4 {
        changed = false;
        let mut i = 0;
        while i < clean.len() && clean.len() > 4 {
            let n = clean.len();
            let prev = clean[(i + n - 1) % n];
            let cur = clean[i];
            let next = clean[(i + 1) % n];
            let dx1 = cur[0] - prev[0];
            let dy1 = cur[1] - prev[1];
            let dx2 = next[0] - cur[0];
            let dy2 = next[1] - cur[1];
            if (dx1 * dy2 - dy1 * dx2).abs() <= 0.01 && (dx1 * dx2 + dy1 * dy2) >= 0.0 {
                clean.remove(i);
                changed = true;
            } else {
                i += 1;
            }
        }
    }
    clean
}
pub(crate) fn spans_bounds(spans: &[[f32; 4]]) -> [f32; 4] {
    let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
    for s in spans {
        b[0] = b[0].min(s[0]);
        b[1] = b[1].min(s[1]);
        b[2] = b[2].max(s[2]);
        b[3] = b[3].max(s[3]);
    }
    b
}
fn with_cached_group_poly(
    spans: &[[f32; 4]],
    radius: f32,
    scale: f32,
    draw: impl FnOnce(&Image, [f32; 4]) -> bool,
) -> bool {
    let bounds = spans_bounds(spans);
    let pad = (14. * scale).ceil() as u32;
    let p = pad as f32 / scale;
    let bw_dip = ((bounds[2] - bounds[0]) * 64.0).round() / 64.0;
    let bh_dip = ((bounds[3] - bounds[1]) * 64.0).round() / 64.0;
    let width = stable_dim_px(bounds[2] - bounds[0], scale).max(1) as u32 + pad * 2;
    let height = stable_dim_px(bounds[3] - bounds[1], scale).max(1) as u32 + pad * 2;
    let mut key = Vec::with_capacity(3 + spans.len() * 4);
    key.push(spans.len() as i32);
    key.push((radius * scale * 16.).round() as i32);
    key.push((scale * 1000.).round() as i32);
    let mut rel_spans = Vec::with_capacity(spans.len());
    for s in spans {
        let rx0 = ((s[0] - bounds[0]) * 64.0).round() / 64.0;
        let ry0 = ((s[1] - bounds[1]) * 64.0).round() / 64.0;
        let rx1 = ((s[2] - bounds[0]) * 64.0).round() / 64.0;
        let ry1 = ((s[3] - bounds[1]) * 64.0).round() / 64.0;
        let rel = [rx0 + p, ry0 + p, rx1 + p, ry1 + p];
        key.push(stable_dim_px(s[0] - bounds[0], scale));
        key.push(stable_dim_px(s[1] - bounds[1], scale));
        key.push(stable_dim_px(s[2] - bounds[0], scale));
        key.push(stable_dim_px(s[3] - bounds[1], scale));
        rel_spans.push(rel);
    }
    let rel_bounds = [p, p, p + bw_dip, p + bh_dip];
    let rel_verts = ortho_group_polygon(&rel_spans);
    if rel_verts.len() < 4 {
        return false;
    }
    GROUP_POLYS.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&key) {
            if cache.len() >= 48 {
                cache.clear();
            }
            if let Some(mut image) =
                Image::from_rgba(width, height, vec![0; (width * height * 4) as usize])
            {
                unsafe {
                    if let Some(canvas) = Canvas::on_image(&image, scale) {
                        canvas.draw_group_poly_material(&rel_spans, &rel_verts, rel_bounds, radius);
                    }
                }
                image.refresh_tint();
                cache.insert(key.clone(), image);
            }
        }
        cache.get(&key).is_some_and(|image| {
            draw(image, [bounds[0] - p, bounds[1] - p, bounds[2] + p, bounds[3] + p])
        })
    })
}
impl Drop for Batch {
    fn drop(&mut self) {
        let current = ACTIVE_BATCH.with(|a| a.replace(self.previous));
        ACTIVE_BATCH_CLIP.with(|c| c.set(self.previous_clip));
        if self.owns_g {
            if let Some((_, g)) = current.filter(|(_, g)| *g != 0) {
                unsafe {
                    GdipFlush(g as Ptr, 1);
                    GdipDeleteGraphics(g as Ptr);
                }
            }
        }
    }
}
pub struct Canvas {
    g: Ptr,
    scale: f32,
    target: Option<crate::sampling::Target>,
    transform: Cell<[f32; 4]>,
    owned: bool,
    state: u32,
}
impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            let _cost = Cost::new(1);
            if self.owned {
                GdipFlush(self.g, 1);
                GdipDeleteGraphics(self.g);
            } else {
                GdipRestoreGraphics(self.g, self.state);
            }
        }
    }
}
impl Canvas {
    pub unsafe fn mark_opaque(&self, r: [f32; 4]) {
        GdipFlush(self.g, 1);
        GdiFlush();
        if let Some(target) = self.target {
            let pixels =
                std::slice::from_raw_parts_mut(target.bits, target.width * target.height * 4);
            let [left, top, right, bottom] = r.map(|v| (v * self.scale).round() as i32);
            for y in top.max(0)..bottom.min(target.height as i32) {
                for x in left.max(0)..right.min(target.width as i32) {
                    pixels[(y as usize * target.width + x as usize) * 4 + 3] = 255;
                }
            }
        }
    }
    pub fn reflection_gradient(&self, r: [f32; 4], tint: f32, light: bool) {
        let key = ((tint.clamp(0., 1.) * 1000.).round() as u32, light);
        REFLECTIONS.with(|cache| {
            let mut cache = cache.borrow_mut();
            if !cache.contains_key(&key) {
                if cache.len() >= 4 {
                    cache.clear();
                }
                let side = 128;
                let mut pixels = vec![0; side * side * 4];
                let strength = 0.22 + 0.78 * tint.clamp(0., 1.);
                for y in 0..side {
                    let v = (y as f32 + 0.5) / side as f32;
                    for x in 0..side {
                        let u = (x as f32 + 0.5) / side as f32;
                        let top =
                            (-((u - 0.05) / 0.65).powi(2) - ((v + 0.08) / 0.65).powi(2)).exp();
                        let cool =
                            (-((u - 0.94) / 0.65).powi(2) - ((v - 0.08) / 0.48).powi(2)).exp();
                        let warm =
                            (-((u - 0.94) / 0.48).powi(2) - ((v - 0.91) / 0.65).powi(2)).exp();
                        let sheen = (-((u * 0.8 + v * 0.48 - 0.25) / 0.18).powi(2)).exp();
                        let mut keep = 1. - v * (if light { 0.08 } else { 0.28 }) * strength;
                        let mut c = [0.; 3];
                        for (alpha, color) in [
                            (top * 0.20 * strength, [220., 202., 197.]),
                            (cool * 0.11 * strength, [235., 180., 159.]),
                            (warm * 0.075 * strength, [176., 190., 226.]),
                            (sheen * 0.045 * strength, [255.; 3]),
                        ] {
                            keep *= 1. - alpha;
                            for channel in 0..3 {
                                c[channel] = c[channel] * (1. - alpha) + color[channel] * alpha;
                            }
                        }
                        let alpha = ((1. - keep) * 255.).round().clamp(0., 255.);
                        let i = (y * side + x) * 4;
                        pixels[i] = c[2].round().clamp(0., alpha) as u8;
                        pixels[i + 1] = c[1].round().clamp(0., alpha) as u8;
                        pixels[i + 2] = c[0].round().clamp(0., alpha) as u8;
                        pixels[i + 3] = alpha as u8;
                    }
                }
                if let Some(image) = Image::from_rgba(side as u32, side as u32, pixels) {
                    cache.insert(key, image);
                }
            }
            if let Some(image) = cache.get(&key) {
                self.image(image, r);
            }
        });
    }
    pub fn soft_shadow(&self, r: [f32; 4], radius: f32, amount: f32, blur: f32, dy: f32) {
        if amount < 0.01 || self.target.is_none() {
            return;
        }
        let w = ((r[2] - r[0]) * self.scale).round().max(1.) as u32;
        let h = ((r[3] - r[1]) * self.scale).round().max(1.) as u32;
        let key = (
            w,
            h,
            (radius * self.scale * 16.).round() as u32,
            (blur * self.scale * 16.).round() as u32,
            (dy * self.scale * 16.).round() as u32,
        );
        let sigma = (blur * self.scale * 0.65).max(0.6);
        let dx = dy * self.scale * 0.9;
        let dy = dy * self.scale;
        let pad = (sigma * 3. + dy.abs() + self.scale * 2.).ceil() as u32;
        SHADOWS.with(|cache| {
            let mut cache = cache.borrow_mut();
            if !cache.contains_key(&key) {
                if cache.len() >= 24 {
                    cache.clear();
                }
                let width = w + pad * 2;
                let height = h + pad * 2;
                let mut pixels = vec![0; (width * height) as usize * 4];
                for y in 0..height {
                    for x in 0..width {
                        let px = x as f32 + 0.5 - pad as f32;
                        let py = y as f32 + 0.5 - pad as f32;
                        let original = crate::compositor::distance(
                            px,
                            py,
                            w as f32,
                            h as f32,
                            radius * self.scale,
                            0.,
                        );
                        let shifted = crate::compositor::distance(
                            px - dx,
                            py - dy,
                            w as f32,
                            h as f32,
                            radius * self.scale,
                            0.,
                        );
                        let outside = (original / (1.5 * self.scale) + 0.5).clamp(0., 1.);
                        let alpha = 0.5 * (1. - (shifted / (sigma * 1.55)).tanh()) * outside;
                        pixels[((y * width + x) as usize) * 4 + 3] =
                            (alpha * 255.).round().clamp(0., 255.) as u8;
                    }
                }
                cache.insert(
                    key,
                    ShadowSurface {
                        image: Image::from_rgba(width, height, pixels),
                    },
                );
            }
            if let Some(surface) = cache.get(&key) {
                if let Some(image) = &surface.image {
                    let pad = pad as f32 / self.scale;
                    self.image_alpha(
                        image,
                        [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad],
                        (amount / 255.).clamp(0., 1.),
                    );
                }
            }
        });
    }
    unsafe fn on_image(image: &Image, scale: f32) -> Option<Self> {
        let mut g = null_mut();
        if GdipGetImageGraphicsContext(image.raw, &mut g) != 0 {
            return None;
        }
        GdipSetTextRenderingHint(g, 4);
        GdipSetSmoothingMode(g, 4);
        GdipSetInterpolationMode(g, 3);
        GdipSetPixelOffsetMode(g, 4);
        Some(Self {
            g,
            scale,
            owned: true,
            state: 0,
            target: image._pixels.as_ref().map(|p| crate::sampling::Target {
                bits: p.as_ptr() as *mut u8,
                width: image.width as usize,
                height: image.height as usize,
                clip: [0, 0, image.width as i32, image.height as i32],
                opaque: false,
            }),
            transform: Cell::new([0., 0., 1., 0.]),
        })
    }
    pub fn render_sprite(
        width: u32,
        height: u32,
        scale: f32,
        draw: impl FnOnce(&Canvas),
    ) -> Option<Image> {
        if width == 0 || height == 0 {
            return None;
        }
        let mut image = Image::from_rgba(width, height, vec![0; (width * height * 4) as usize])?;
        unsafe {
            if let Some(canvas) = Self::on_image(&image, scale) {
                draw(&canvas);
            }
        }
        image.refresh_tint();
        Some(image)
    }
    pub fn pane_overlay(size: [u32; 2], dpi: f32, tint: f32, light: bool) -> Option<std::rc::Rc<Image>> {
        let key = (size, dpi.to_bits(), (tint * 1000.).round() as i32, light);
        if let Some(image) = OVERLAYS.with(|c| c.borrow().get(&key).cloned()) {
            return Some(image);
        }
        let mut image =
            Image::from_rgba(size[0], size[1], vec![0; (size[0] * size[1] * 4) as usize])?;
        unsafe {
            let c = Self::on_image(&image, dpi)?;
            let rect = [0., 0., size[0] as f32 / dpi, size[1] as f32 / dpi];
            c.reflection_gradient(rect, tint, light);
        }
        if let Some(pixels) = image._pixels.as_mut() {
            for y in 0..size[1] {
                for x in 0..size[0] {
                    let mut hash = ((x / 2) % 64).wrapping_mul(374761393)
                        ^ ((y / 2) % 64).wrapping_mul(668265263);
                    hash = (hash ^ (hash >> 13)).wrapping_mul(1274126177);
                    let value = ((hash ^ (hash >> 16)) & 255) as i32 - 128;
                    // Two-pixel grain, at 36% of the original opacity. Baked
                    // into the cached material rather than animated.
                    let alpha = (value.abs() * 396 / 12800) as u32;
                    let i = ((y * size[0] + x) * 4) as usize;
                    for c in 0..4 {
                        let src = if c == 3 || value > 0 { alpha } else { 0 };
                        pixels[i + c] = (src + (pixels[i + c] as u32 * (255 - alpha) + 127) / 255)
                            .min(255) as u8;
                    }
                }
            }
        }
        image.refresh_tint();
        let image = std::rc::Rc::new(image);
        OVERLAYS.with(|c| {
            let mut cache = c.borrow_mut();
            let bytes: u64 = cache.values().map(|i| i.width as u64 * i.height as u64 * 4).sum();
            let added = size[0] as u64 * size[1] as u64 * 4;
            if cache.len() >= 12 || bytes + added > 32 * 1024 * 1024 {
                cache.clear();
            }
            cache.insert(key, image.clone());
        });
        Some(image)
    }
    fn draw_grain(&self, rect: [f32; 4], dpi: f32) {
        let step = 128. / dpi.max(1.);
        GRAIN_TILE.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                let mut pixels = vec![0u8; 128 * 128 * 4];
                for y in 0..128u32 {
                    for x in 0..128u32 {
                        let mut hash = ((x / 2) % 64).wrapping_mul(374761393)
                            ^ ((y / 2) % 64).wrapping_mul(668265263);
                        hash = (hash ^ (hash >> 13)).wrapping_mul(1274126177);
                        let value = ((hash ^ (hash >> 16)) & 255) as i32 - 128;
                        let alpha = (value.abs() * 396 / 12800) as u8;
                        let rgb = if value > 0 { alpha } else { 0 };
                        let i = ((y * 128 + x) * 4) as usize;
                        pixels[i] = rgb;
                        pixels[i + 1] = rgb;
                        pixels[i + 2] = rgb;
                        pixels[i + 3] = alpha;
                    }
                }
                *slot = Image::from_rgba(128, 128, pixels);
            }
            if let Some(tile) = slot.as_ref() {
                let mut y = rect[1];
                while y < rect[3] {
                    let mut x = rect[0];
                    while x < rect[2] {
                        self.clipped_image(tile, [x, y, x + step, y + step], rect);
                        x += step;
                    }
                    y += step;
                }
            }
        });
    }
    pub fn draw_pane_overlay(
        &self,
        _size: [u32; 2],
        dpi: f32,
        tint: f32,
        light: bool,
        rect: [f32; 4],
    ) {
        let width = (rect[2] - rect[0]) * self.scale;
        let height = (rect[3] - rect[1]) * self.scale;
        // These layers depend on material/geometry, not desktop position.
        // Reuse device-sized pixels while moving; the wallpaper beneath them
        // is still rendered from the current window position every frame.
        if (dpi - self.scale).abs() < 0.001
            && (width - width.round()).abs() < 0.01
            && (height - height.round()).abs() < 0.01
            && width >= 1. && height >= 1. && width * height <= 8_000_000.
        {
            if let Some(image) = Self::pane_overlay([width.round() as u32, height.round() as u32], dpi, tint, light) {
                self.image(&image, rect);
                return;
            }
        }
        self.reflection_gradient(rect, tint, light);
        self.draw_grain(rect, dpi);
    }
    pub fn tile(
        icon: &Image,
        icon_shadow: Option<(u32, &Image)>,
        label: Option<(&Image, Option<(u32, &Image)>)>,
        size: [f32; 2],
        icon_size: f32,
        dpi: f32,
    ) -> Option<(f32, Image)> {
        let mut margin = 7f32;
        if let Some((p, _)) = icon_shadow {
            let spread = p as f32 / (icon_size * dpi * 1.12).ceil() * icon_size;
            margin = margin
                .max(spread - 8. - 1.2)
                .max(spread - (size[0] - icon_size) * 0.5 + 0.65)
                .max(8. + icon_size + spread + 1.2 - size[1])
                + 2.;
        }
        let pad_px = (margin * dpi).ceil() as u32;
        let pad = pad_px as f32 / dpi;
        let width = (size[0] * dpi).round() as u32 + pad_px * 2;
        let height = (size[1] * dpi).round() as u32 + pad_px * 2;
        let mut image = Image::from_rgba(width, height, vec![0; (width * height * 4) as usize])?;
        unsafe {
            let canvas = Canvas::on_image(&image, dpi)?;
            let x = pad + size[0] * 0.5 - icon_size * 0.5;
            let y = pad + 8.;
            if let Some((p, shadow)) = icon_shadow {
                let px = (icon_size * dpi * 1.12).ceil();
                let spread = p as f32 / px * icon_size;
                canvas.image(
                    shadow,
                    [
                        x - spread + 0.65,
                        y - spread + 1.2,
                        x + icon_size + spread + 0.65,
                        y + icon_size + spread + 1.2,
                    ],
                );
            }
            canvas.image(icon, [x, y, x + icon_size, y + icon_size]);
            if let Some((label, shadow)) = label {
                let y = pad + icon_size + 14.;
                let rect = label_rect(label, pad + size[0] * 0.5, y, dpi, true);
                let clip = label_clip(size[0] - 6., pad + size[0] * 0.5, y, dpi, true);
                if let Some((p, shadow)) = shadow {
                    let spread = p as f32 / dpi;
                    canvas.clipped_image(
                        shadow,
                        [
                            rect[0] - spread + 0.2,
                            y - spread + 0.65,
                            rect[2] + spread + 0.2,
                            rect[3] + spread + 0.65,
                        ],
                        [
                            clip[0] - spread,
                            y - spread,
                            clip[2] + spread,
                            clip[3] + spread + 1.,
                        ],
                    );
                }
                canvas.clipped_image(label, rect, clip);
            }
        }
        image.refresh_tint();
        Some((pad, image))
    }
    pub fn label(text: &str, width: f32, height: f32, dpi: f32, color: u32) -> Option<Image> {
        let width_px = (width * dpi).round().max(1.) as u32;
        let height_px = (height * dpi).round().max(1.) as u32;
        let mut image = Image::from_rgba(
            width_px,
            height_px,
            vec![0; (width_px * height_px * 4) as usize],
        )?;
        unsafe {
            let canvas = Self::on_image(&image, dpi)?;
            // Grayscale grid-fitted glyphs at the final device resolution.
            // Supersampling/downscaling and subsequent fractional placement
            // both softened the thin Regular strokes.
            GdipSetTextRenderingHint(canvas.g, 3);
            GdipSetPixelOffsetMode(canvas.g, 3);
            canvas.text(
                text,
                [0., 0., width_px as f32 / dpi, height_px as f32 / dpi],
                11.,
                true,
                color,
                1,
            );
        }
        image.refresh_tint();
        Some(image)
    }
    pub fn group_surface_live(&self, r: [f32; 4], radius: f32, _live: bool) {
        let _cost = Cost::new(3);
        let key = group_surface_key(r, radius, self.scale);
        let pad = (14. * self.scale).ceil() as u32;
        let p = pad as f32 / self.scale;
        GROUPS.with(|cache| {
            let mut cache = cache.borrow_mut();
            if !cache.contains_key(&key) {
                let nearest = cache
                    .keys()
                    .filter(|k| k.1 == key.1 && k.2 == key.2 && k.3 == key.3)
                    .min_by_key(|k| k.0.abs_diff(key.0))
                    .copied();
                if let Some(old) = nearest {
                    if let Some(image) = cache.get(&old) {
                        self.image_horizontal(
                            image,
                            [r[0] - p, r[1] - p, r[2] + p, r[3] + p],
                            pad + (radius * self.scale).ceil() as u32,
                        );
                        return;
                    }
                }
            }
            if !cache.contains_key(&key) {
                if cache.len() >= 32 {
                    cache.clear();
                }
                let width = key.0 + pad * 2;
                let height = key.1 + pad * 2;
                if let Some(mut image) =
                    Image::from_rgba(width, height, vec![0; (width * height * 4) as usize])
                {
                    unsafe {
                        if let Some(canvas) = Canvas::on_image(&image, self.scale) {
                            canvas.draw_group_material(
                                [p, p, p + r[2] - r[0], p + r[3] - r[1]],
                                radius,
                            );
                        }
                    }
                    image.refresh_tint();
                    cache.insert(key, image);
                }
            }
            if let Some(image) = cache.get(&key) {
                self.image(image, [r[0] - p, r[1] - p, r[2] + p, r[3] + p]);
            }
        });
    }
    fn draw_group_material(&self, r: [f32; 4], radius: f32) {
        self.soft_shadow(r, radius, 20., 4., 2.);
        self.round(r, radius, 0xffffff, 15, false, false);
        self.coating(r, radius);
        self.reflection_edge(r, radius, 0.45);
    }
    pub fn group_poly_surface_live(&self, spans: &[[f32; 4]], radius: f32) {
        if spans.is_empty() {
            return;
        }
        if spans.len() == 1 {
            self.group_surface_live(spans[0], radius, true);
            return;
        }
        let _cost = Cost::new(3);
        with_cached_group_poly(spans, radius, self.scale, |image, dest| {
            self.image(image, dest);
            true
        });
    }
    fn soft_shadow_poly(
        &self,
        verts: &[[f32; 2]],
        bounds: [f32; 4],
        radius: f32,
        amount: f32,
        blur: f32,
        dy: f32,
    ) {
        if amount < 0.01 || self.target.is_none() || verts.len() < 4 {
            return;
        }
        let bw = ((bounds[2] - bounds[0]) * self.scale).round().max(1.) as u32;
        let bh = ((bounds[3] - bounds[1]) * self.scale).round().max(1.) as u32;
        let sigma = (blur * self.scale * 0.65).max(0.6);
        let dx = dy * self.scale * 0.9;
        let dy_px = dy * self.scale;
        let pad = (sigma * 3. + dy_px.abs() + self.scale * 2.).ceil() as u32;
        let pad_dip = pad as f32 / self.scale;
        let width = bw + pad * 2;
        let height = bh + pad * 2;
        let scaled_verts: Vec<[f32; 2]> = verts
            .iter()
            .map(|v| [
                (v[0] - bounds[0]) * self.scale + pad as f32,
                (v[1] - bounds[1]) * self.scale + pad as f32,
            ])
            .collect();
        let sdf = crate::compositor::PolySdf::new(&scaled_verts, radius * self.scale);
        let mut pixels = vec![0u8; (width * height) as usize * 4];
        for y in 0..height {
            let py = y as f32 + 0.5;
            for x in 0..width {
                let px = x as f32 + 0.5;
                let (original, _, _) = sdf.signed_distance(px, py);
                if original <= -1.5 * self.scale {
                    continue;
                }
                let (shifted, _, _) = sdf.signed_distance(px - dx, py - dy_px);
                let outside = (original / (1.5 * self.scale) + 0.5).clamp(0., 1.);
                let alpha = 0.5 * (1. - (shifted / (sigma * 1.55)).tanh()) * outside;
                pixels[((y * width + x) as usize) * 4 + 3] =
                    (alpha * 255.).round().clamp(0., 255.) as u8;
            }
        }
        if let Some(image) = Image::from_bgra(width, height, pixels) {
            self.image_alpha(
                &image,
                [
                    bounds[0] - pad_dip,
                    bounds[1] - pad_dip,
                    bounds[2] + pad_dip,
                    bounds[3] + pad_dip,
                ],
                (amount / 255.).clamp(0., 1.),
            );
        }
    }
    fn draw_group_poly_material(
        &self,
        spans: &[[f32; 4]],
        verts: &[[f32; 2]],
        bounds: [f32; 4],
        radius: f32,
    ) {
        self.soft_shadow_poly(verts, bounds, radius, 20., 4., 2.);
        self.poly_round(verts, radius, 0xffffff, 15, false);
        self.poly_coating(verts, bounds, radius);
        self.poly_reflection_edge(spans, verts, bounds, radius, 0.45);
    }
    unsafe fn poly_path(&self, verts: &[[f32; 2]], radius: f32) -> Ptr {
        let mut p = null_mut();
        GdipCreatePath(0, &mut p);
        let n = verts.len();
        if n < 3 {
            return p;
        }
        for i in 0..n {
            let prev = verts[(i + n - 1) % n];
            let v = verts[i];
            let next = verts[(i + 1) % n];
            let l_in = (v[0] - prev[0]).hypot(v[1] - prev[1]).max(1e-4);
            let l_out = (next[0] - v[0]).hypot(next[1] - v[1]).max(1e-4);
            let d_in = [(v[0] - prev[0]) / l_in, (v[1] - prev[1]) / l_in];
            let d_out = [(next[0] - v[0]) / l_out, (next[1] - v[1]) / l_out];
            let r_i = radius.min(l_in * 0.5).min(l_out * 0.5).max(0.1);
            let cross = d_in[0] * d_out[1] - d_in[1] * d_out[0];
            let cx = v[0] - d_in[0] * r_i + d_out[0] * r_i;
            let cy = v[1] - d_in[1] * r_i + d_out[1] * r_i;
            let start = (-d_out[1]).atan2(-d_out[0]).to_degrees();
            let sweep = if cross > 0.0 { 90.0 } else { -90.0 };
            let dia = 2.0 * r_i * self.scale;
            GdipAddPathArc(
                p,
                (cx - r_i) * self.scale,
                (cy - r_i) * self.scale,
                dia,
                dia,
                start,
                sweep,
            );
        }
        GdipClosePathFigure(p);
        p
    }
    pub fn poly_round(
        &self,
        verts: &[[f32; 2]],
        radius: f32,
        color: u32,
        alpha: u8,
        outline: bool,
    ) {
        if verts.len() < 3 {
            return;
        }
        unsafe {
            let p = self.poly_path(verts, radius);
            if outline {
                let pen = self.pen(color, alpha, 0.65);
                GdipDrawPath(self.g, pen, p);
                GdipDeletePen(pen);
            } else {
                let mut brush = null_mut();
                GdipCreateSolidFill(argb(color, alpha), &mut brush);
                GdipFillPath(self.g, brush, p);
                GdipDeleteBrush(brush);
            }
            GdipDeletePath(p);
        }
    }
    fn poly_coating(&self, verts: &[[f32; 2]], bounds: [f32; 4], radius: f32) {
        if verts.len() < 3 {
            return;
        }
        unsafe {
            let path = self.poly_path(verts, radius);
            let rect = Rect {
                x: bounds[0] * self.scale,
                y: bounds[1] * self.scale,
                w: (bounds[2] - bounds[0]) * self.scale,
                h: (bounds[3] - bounds[1]) * self.scale,
            };
            let mut brush = null_mut();
            GdipCreateLineBrushFromRectWithAngle(
                &rect, 0x0cffffff, 0x00ffffff, 55., 0, 0, &mut brush,
            );
            GdipFillPath(self.g, brush, path);
            GdipDeleteBrush(brush);
            GdipDeletePath(path);
        }
    }
    fn poly_reflection_edge(
        &self,
        spans: &[[f32; 4]],
        verts: &[[f32; 2]],
        bounds: [f32; 4],
        radius: f32,
        amount: f32,
    ) {
        if spans.is_empty() || verts.len() < 3 {
            return;
        }
        unsafe {
            let edge = self.poly_path(verts, radius);
            let tl = [spans[0][0] + radius * 0.35, spans[0][1] + radius * 0.35];
            let last = spans[spans.len() - 1];
            let br = [last[2] - radius * 0.25, last[3] - radius * 0.25];
            let max_span = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]);
            for (point, strength, span) in [(tl, 115., 0.85), (br, 55., 0.55)] {
                let reach = (max_span * span).max(35.);
                let circle = self.path(
                    [
                        point[0] - reach,
                        point[1] - reach,
                        point[0] + reach,
                        point[1] + reach,
                    ],
                    reach,
                );
                let mut brush = null_mut();
                GdipCreatePathGradientFromPath(circle, &mut brush);
                GdipSetPathGradientCenterColor(brush, argb(0xffffff, (strength * amount) as u8));
                let transparent = 0x00ffffff;
                let mut count = 1;
                GdipSetPathGradientSurroundColorsWithCount(brush, &transparent, &mut count);
                let center = Point {
                    x: point[0] * self.scale,
                    y: point[1] * self.scale,
                };
                GdipSetPathGradientCenterPoint(brush, &center);
                let mut pen = null_mut();
                GdipCreatePen2(brush, 0.8 * self.scale, 2, &mut pen);
                GdipDrawPath(self.g, pen, edge);
                GdipDeletePen(pen);
                GdipDeleteBrush(brush);
                GdipDeletePath(circle);
            }
            GdipDeletePath(edge);
        }
    }
    pub fn focus_shadow(&self, r: [f32; 4], radius: f32, amount: f32) {
        for i in (1..=6).rev() {
            let spread = i as f32 * 0.9;
            self.round(
                [
                    r[0] - spread,
                    r[1] + 3. - spread,
                    r[2] + spread,
                    r[3] + 3. + spread,
                ],
                radius + spread,
                0,
                (amount * 5.) as u8,
                false,
                false,
            );
        }
        self.reflection_edge(r, radius, amount * 0.45);
    }
    pub fn transform_about(&self, p: [f32; 2], scale: f32, rotation: f32) {
        self.transform
            .set([p[0] * self.scale, p[1] * self.scale, scale, rotation]);
        unsafe {
            GdipTranslateWorldTransform(self.g, -p[0] * self.scale, -p[1] * self.scale, 1);
            GdipScaleWorldTransform(self.g, scale, scale, 1);
            GdipRotateWorldTransform(self.g, rotation, 1);
            GdipTranslateWorldTransform(self.g, p[0] * self.scale, p[1] * self.scale, 1);
        }
    }
    pub fn coating(&self, r: [f32; 4], radius: f32) {
        unsafe {
            let path = self.path(r, radius);
            let rect = Rect {
                x: r[0] * self.scale,
                y: r[1] * self.scale,
                w: (r[2] - r[0]) * self.scale,
                h: (r[3] - r[1]) * self.scale,
            };
            let mut brush = null_mut();
            GdipCreateLineBrushFromRectWithAngle(
                &rect, 0x0cffffff, 0x00ffffff, 55., 0, 0, &mut brush,
            );
            GdipFillPath(self.g, brush, path);
            GdipDeleteBrush(brush);
            GdipDeletePath(path);
        }
    }
    pub fn widget_corner_reflection(&self, r: [f32; 4], radius: f32, light: bool) {
        let x = r[0] + radius * 0.65;
        let y = r[1] + 4.;
        let reach = ((r[2] - r[0]) * 0.40).clamp(160., 270.);
        self.draw_corner_reflection_at(r, radius, x, y, reach, light);
    }
    pub unsafe fn widget_corner_reflection_for(
        &self,
        hwnd: windows_sys::Win32::Foundation::HWND,
        r: [f32; 4],
        radius: f32,
        light: bool,
    ) {
        let Some(panes) = crate::fusion::group(hwnd) else {
            self.widget_corner_reflection(r, radius, light);
            return;
        };
        let mut wr: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut wr);
        for p in &panes {
            let scale = p.dpi;
            let left_covered = panes.iter().any(|q| {
                q.id != p.id
                    && q.rect[0] < p.rect[0] + 6. * scale
                    && q.rect[2] >= p.rect[0] - 6. * scale
                    && q.rect[1] <= p.rect[1] + p.radius * 0.5
                    && q.rect[3] >= p.rect[1] + 6. * scale
            });
            let top_covered = panes.iter().any(|q| {
                q.id != p.id
                    && q.rect[1] < p.rect[1] + 6. * scale
                    && q.rect[3] >= p.rect[1] - 6. * scale
                    && q.rect[0] <= p.rect[0] + p.radius * 0.5
                    && q.rect[2] >= p.rect[0] + 6. * scale
            });
            if left_covered || top_covered {
                continue;
            }
            let p_l = (p.rect[0] - wr.left as f32) / self.scale;
            let p_t = (p.rect[1] - wr.top as f32) / self.scale;
            let p_w = (p.rect[2] - p.rect[0]) / self.scale;
            let p_rad = p.radius / p.dpi;
            let x = p_l + p_rad * 0.65;
            let y = p_t + 4.;
            let reach = (p_w * 0.40).clamp(160., 270.);
            if x + reach < r[0] || x - reach > r[2] || y + 160. < r[1] || y - 160. > r[3] {
                continue;
            }
            self.draw_corner_reflection_at(r, radius, x, y, reach, light);
        }
    }
    fn draw_corner_reflection_at(
        &self,
        r: [f32; 4],
        radius: f32,
        x: f32,
        y: f32,
        reach: f32,
        light: bool,
    ) {
        // Localized reflection belongs to the enclosing glass, not the search
        // pane. Its short falloff leaves the rest of the wallpaper transparent.
        unsafe {
            let clip = self.path(r, radius);
            let reflection = self.path([x - reach, y - 160., x + reach, y + 160.], 160.);
            let mut brush = null_mut();
            GdipCreatePathGradientFromPath(reflection, &mut brush);
            GdipSetPathGradientCenterColor(brush, argb(0xffffff, if light { 36 } else { 48 }));
            let transparent = 0x00ffffff;
            let mut count = 1;
            GdipSetPathGradientSurroundColorsWithCount(brush, &transparent, &mut count);
            let center = Point {
                x: x * self.scale,
                y: y * self.scale,
            };
            GdipSetPathGradientCenterPoint(brush, &center);
            // Fill the antialiased pane path directly. A GDI+ clip path is a
            // binary edge and turns into visible steps when cached/scaled.
            GdipFillPath(self.g, brush, clip);
            GdipDeleteBrush(brush);
            GdipDeletePath(reflection);
            GdipDeletePath(clip);
        }
    }
    pub fn search_surface(&self, r: [f32; 4], radius: f32, color: u32, light: bool) {
        // A recessed pane retains the palette hue without the raised outer
        // shadow or corner highlight of the enclosing widget glass.
        self.round(r, radius, color, 5, false, false);
        self.round(
            r,
            radius,
            if light { 0xffffff } else { 0 },
            if light { 6 } else { 0 },
            false,
            false,
        );
        unsafe {
            let path = self.path(r, radius);
            let mut brush = null_mut();
            let rect = Rect {
                x: r[0] * self.scale,
                y: r[1] * self.scale,
                w: (r[2] - r[0]) * self.scale,
                h: (r[3] - r[1]) * self.scale,
            };
            GdipCreateLineBrushFromRectWithAngle(
                &rect,
                argb(0xffffff, if light { 4 } else { 5 }),
                0x00ffffff,
                55.,
                0,
                0,
                &mut brush,
            );
            GdipFillPath(self.g, brush, path);
            GdipDeleteBrush(brush);
            GdipDeletePath(path);
        }
        self.reflection_edge(r, radius, 0.175);
    }
    pub fn glass_surface(&self, r: [f32; 4], radius: f32, shade: u8, light: bool) {
        if light {
            self.soft_shadow(r, radius, 12., 5., 2.);
            self.round(r, radius, 0xffffff, shade.saturating_add(22), false, false);
            self.coating(r, radius);
            self.reflection_edge(r, radius, 0.75);
            return;
        }
        self.round(
            [r[0], r[1] + 2., r[2], r[3] + 3.],
            radius,
            0,
            14,
            false,
            false,
        );
        self.round(r, radius, 0, shade, false, false);
        self.coating(r, radius);
        self.reflection_edge(r, radius, 0.55);
    }
    pub fn hover_surface(&self, r: [f32; 4], radius: f32, amount: f32, light: bool) {
        self.soft_shadow(r, radius, amount * if light { 18. } else { 28. }, 5., 2.5);
        unsafe {
            let path = self.path(r, radius);
            let rect = Rect {
                x: r[0] * self.scale,
                y: r[1] * self.scale,
                w: (r[2] - r[0]) * self.scale,
                h: (r[3] - r[1]) * self.scale,
            };
            let mut brush = null_mut();
            GdipCreateLineBrushFromRectWithAngle(
                &rect,
                argb(0xffffff, (amount * if light { 100. } else { 28. }) as u8),
                if light {
                    argb(0xffffff, (amount * 36.) as u8)
                } else {
                    argb(0xffffff, (amount * 6.) as u8)
                },
                80.,
                0,
                0,
                &mut brush,
            );
            GdipFillPath(self.g, brush, path);
            GdipDeleteBrush(brush);
            GdipDeletePath(path);
        }
        self.reflection_edge(r, radius, amount);
        unsafe {
            let path = self.path(r, radius);
            let mut brush = null_mut();
            let rect = Rect {
                x: r[0] * self.scale,
                y: r[1] * self.scale,
                w: (r[2] - r[0]) * self.scale,
                h: (r[3] - r[1]) * self.scale,
            };
            GdipCreateLineBrushFromRectWithAngle(
                &rect,
                argb(0xffffff, (amount * 140.) as u8),
                0x00ffffff,
                82.,
                0,
                0,
                &mut brush,
            );
            let factors = [0., 0.55, 1., 1.];
            let positions = [0., 0.09, 0.35, 1.];
            GdipSetLineBlend(brush, factors.as_ptr(), positions.as_ptr(), 4);
            let mut pen = null_mut();
            GdipCreatePen2(brush, 2.1 * self.scale, 2, &mut pen);
            GdipDrawPath(self.g, pen, path);
            GdipDeletePen(pen);
            GdipDeleteBrush(brush);
            GdipDeletePath(path);
        }
    }
    pub fn reflection_edge(&self, r: [f32; 4], radius: f32, amount: f32) {
        unsafe {
            let edge = self.path(r, radius);
            for (point, strength, span) in [
                ([r[0] + radius * 0.35, r[1] + radius * 0.35], 115., 0.85),
                ([r[2] - radius * 0.25, r[3] - radius * 0.25], 55., 0.55),
            ] {
                let reach = ((r[2] - r[0]).max(r[3] - r[1]) * span).max(35.);
                let circle = self.path(
                    [
                        point[0] - reach,
                        point[1] - reach,
                        point[0] + reach,
                        point[1] + reach,
                    ],
                    reach,
                );
                let mut brush = null_mut();
                GdipCreatePathGradientFromPath(circle, &mut brush);
                GdipSetPathGradientCenterColor(brush, argb(0xffffff, (strength * amount) as u8));
                let transparent = 0x00ffffff;
                let mut count = 1;
                GdipSetPathGradientSurroundColorsWithCount(brush, &transparent, &mut count);
                let center = Point {
                    x: point[0] * self.scale,
                    y: point[1] * self.scale,
                };
                GdipSetPathGradientCenterPoint(brush, &center);
                let mut pen = null_mut();
                GdipCreatePen2(brush, 0.8 * self.scale, 2, &mut pen);
                GdipDrawPath(self.g, pen, edge);
                GdipDeletePen(pen);
                GdipDeleteBrush(brush);
                GdipDeletePath(circle);
            }
            GdipDeletePath(edge);
        }
    }
    pub fn glow(&self, r: [f32; 4], radius: f32, point: [f32; 2], color: u32, alpha: u8) {
        unsafe {
            let clip = self.path(r, radius);
            GdipSetClipPath(self.g, clip, 0);
            let circle = self.path(
                [
                    point[0] - 60.,
                    point[1] - 60.,
                    point[0] + 60.,
                    point[1] + 60.,
                ],
                60.,
            );
            let mut brush = null_mut();
            GdipCreatePathGradientFromPath(circle, &mut brush);
            GdipSetPathGradientCenterColor(brush, argb(color, alpha));
            let edge = argb(color, 0);
            let mut count = 1;
            GdipSetPathGradientSurroundColorsWithCount(brush, &edge, &mut count);
            let p = Point {
                x: point[0] * self.scale,
                y: point[1] * self.scale,
            };
            GdipSetPathGradientCenterPoint(brush, &p);
            GdipFillPath(self.g, brush, circle);
            GdipResetClip(self.g);
            GdipDeleteBrush(brush);
            GdipDeletePath(circle);
            GdipDeletePath(clip);
        }
    }
    pub unsafe fn new(dc: HDC, scale: f32) -> Self {
        let _cost = Cost::new(0);
        BATCH_DIRTY.with(|d| d.set(true));
        // Complete queued GDI copies before GDI+ blends into the same DIB.
        GdiFlush();
        let mut clip = std::mem::zeroed();
        GetClipBox(dc, &mut clip);
        let shared = ACTIVE_BATCH
            .with(|a| a.get())
            .filter(|(handle, _)| *handle == dc as usize);
        let mut owned = true;
        let mut state = 0;
        let mut g = null_mut();
        if let Some((handle, pointer)) = shared {
            if pointer == 0 {
                GdipCreateFromHDC(dc, &mut g);
                GdipSetSmoothingMode(g, 4);
                GdipSetTextRenderingHint(g, 4);
                GdipSetInterpolationMode(g, 3);
                GdipSetPixelOffsetMode(g, 4);
                ACTIVE_BATCH.with(|a| a.set(Some((handle, g as usize))));
                ACTIVE_BATCH_CLIP
                    .with(|c| c.set([clip.left, clip.top, clip.right, clip.bottom]));
                owned = false;
            } else {
                let g_clip = ACTIVE_BATCH_CLIP.with(|c| c.get());
                if clip.left >= g_clip[0]
                    && clip.top >= g_clip[1]
                    && clip.right <= g_clip[2]
                    && clip.bottom <= g_clip[3]
                {
                    g = pointer as Ptr;
                    owned = false;
                }
            }
        }
        if owned {
            GdipCreateFromHDC(dc, &mut g);
            GdipSetSmoothingMode(g, 4);
            GdipSetTextRenderingHint(g, 4);
            GdipSetInterpolationMode(g, 3);
            GdipSetPixelOffsetMode(g, 4);
        } else {
            GdipSaveGraphics(g, &mut state);
            GdipResetWorldTransform(g);
            GdipSetClipRect(
                g,
                clip.left as f32,
                clip.top as f32,
                (clip.right - clip.left) as f32,
                (clip.bottom - clip.top) as f32,
                0,
            );
        }
        let mut dib: DIBSECTION = std::mem::zeroed();
        let bitmap = GetCurrentObject(dc, OBJ_BITMAP as u32);
        let target = if GetObjectW(
            bitmap,
            std::mem::size_of::<DIBSECTION>() as i32,
            &mut dib as *mut _ as _,
        ) == std::mem::size_of::<DIBSECTION>() as i32
            && !dib.dsBm.bmBits.is_null()
            && dib.dsBm.bmBitsPixel == 32
            && dib.dsBm.bmWidthBytes == dib.dsBm.bmWidth * 4
        {
            Some(crate::sampling::Target {
                bits: dib.dsBm.bmBits as _,
                width: dib.dsBm.bmWidth as usize,
                height: dib.dsBm.bmHeight.unsigned_abs() as usize,
                clip: [clip.left, clip.top, clip.right, clip.bottom],
                opaque: !ALPHA_DC.with(|a| a.get() == Some(dc as usize)),
            })
        } else {
            None
        };
        Self {
            g,
            scale,
            owned,
            state,
            target,
            transform: Cell::new([0., 0., 1., 0.]),
        }
    }
    unsafe fn path(&self, r: [f32; 4], radius: f32) -> Ptr {
        let mut p = null_mut();
        GdipCreatePath(0, &mut p);
        let x = r[0] * self.scale;
        let y = r[1] * self.scale;
        let w = (r[2] - r[0]) * self.scale;
        let h = (r[3] - r[1]) * self.scale;
        let d = (radius * 2.0 * self.scale).min(w).min(h).max(0.1);
        for (ax, ay, start) in [
            (x, y, 180.0),
            (x + w - d, y, 270.0),
            (x + w - d, y + h - d, 0.0),
            (x, y + h - d, 90.0),
        ] {
            GdipAddPathArc(p, ax, ay, d, d, start, 90.0);
        }
        GdipClosePathFigure(p);
        p
    }
    pub fn round(
        &self,
        r: [f32; 4],
        radius: f32,
        color: u32,
        alpha: u8,
        outline: bool,
        dashed: bool,
    ) {
        unsafe {
            let p = self.path(r, radius);
            if outline {
                let pen = self.pen(color, alpha, if dashed { 1.0 } else { 0.65 });
                if dashed {
                    GdipSetPenDashStyle(pen, 1);
                }
                GdipDrawPath(self.g, pen, p);
                GdipDeletePen(pen);
            } else {
                let mut brush = null_mut();
                GdipCreateSolidFill(argb(color, alpha), &mut brush);
                GdipFillPath(self.g, brush, p);
                GdipDeleteBrush(brush);
            }
            GdipDeletePath(p);
        }
    }
    unsafe fn pen(&self, color: u32, alpha: u8, width: f32) -> Ptr {
        let mut p = null_mut();
        GdipCreatePen1(argb(color, alpha), width * self.scale, 2, &mut p);
        GdipSetPenStartCap(p, 2);
        GdipSetPenEndCap(p, 2);
        p
    }
    pub fn line(&self, points: &[(f32, f32)], color: u32, alpha: u8, width: f32) {
        unsafe {
            let pen = self.pen(color, alpha, width);
            let p: Vec<Point> = points
                .iter()
                .map(|(x, y)| Point {
                    x: x * self.scale,
                    y: y * self.scale,
                })
                .collect();
            GdipDrawLines(self.g, pen, p.as_ptr(), p.len() as i32);
            GdipDeletePen(pen);
        }
    }
    pub fn clipped_image(&self, image: &Image, r: [f32; 4], clip: [f32; 4]) {
        self.clipped_image_alpha(image, r, clip, 1.);
    }
    pub fn clipped_image_alpha(&self, image: &Image, r: [f32; 4], clip: [f32; 4], opacity: f32) {
        if let (Some(mut target), Some(pixels)) = (self.target, image._pixels.as_ref()) {
            let bounds = clip.map(|v| (v * self.scale).round() as i32);
            target.clip = [
                target.clip[0].max(bounds[0]),
                target.clip[1].max(bounds[1]),
                target.clip[2].min(bounds[2]),
                target.clip[3].min(bounds[3]),
            ];
            let w = ((r[2] - r[0]) * self.scale).round();
            let h = ((r[3] - r[1]) * self.scale).round();
            let cx = (r[0] + r[2]) * self.scale * 0.5;
            let cy = (r[1] + r[3]) * self.scale * 0.5;
            unsafe {
                GdipFlush(self.g, 1);
                GdiFlush();
                target.draw(
                    pixels,
                    [image.width as usize, image.height as usize],
                    [cx - w * 0.5, cy - h * 0.5, cx + w * 0.5, cy + h * 0.5],
                    self.transform.get(),
                    opacity,
                    image.tint,
                );
            }
            return;
        }
        unsafe {
            GdipSetClipRect(
                self.g,
                clip[0] * self.scale,
                clip[1] * self.scale,
                (clip[2] - clip[0]) * self.scale,
                (clip[3] - clip[1]) * self.scale,
                1,
            );
        }
        self.image_alpha(image, r, opacity);
    }
    pub fn image_horizontal(&self, image: &Image, r: [f32; 4], edge: u32) {
        self.image_horizontal_alpha(image, r, edge, 1.);
    }
    pub fn image_horizontal_alpha(&self, image: &Image, r: [f32; 4], edge: u32, opacity: f32) {
        let width = ((r[2] - r[0]) * self.scale).round();
        let edge = edge
            .min(image.width / 2 - 1)
            .min((width / 2. - 1.).max(1.) as u32)
            .max(1);
        let mut parts = image.horizontal_parts.borrow_mut();
        if parts.as_ref().is_none_or(|(old, _)| *old != edge) {
            if let Some(source) = image._pixels.as_ref() {
                let mut images = Vec::new();
                for (start, width) in [
                    (0, edge + 1),
                    (edge - 1, image.width - 2 * edge + 2),
                    (image.width - edge - 1, edge + 1),
                ] {
                    let mut pixels = Vec::with_capacity((width * image.height * 4) as usize);
                    for y in 0..image.height {
                        let begin = ((y * image.width + start) * 4) as usize;
                        pixels.extend_from_slice(&source[begin..begin + (width * 4) as usize]);
                    }
                    for p in pixels.chunks_exact_mut(4) {
                        p.swap(0, 2);
                    }
                    if let Some(part) = Image::from_rgba(width, image.height, pixels) {
                        images.push(part);
                    }
                }
                if images.len() == 3 {
                    *parts = Some((edge, images));
                }
            }
        }
        if let Some((_, parts)) = parts.as_ref() {
            let e = edge as f32 / self.scale;
            let p = 1. / self.scale;
            let left = (r[0] * self.scale).round() / self.scale;
            let right = left + width / self.scale;
            let clips = [
                [left, r[1], left + e, r[3]],
                [left + e, r[1], right - e, r[3]],
                [right - e, r[1], right, r[3]],
            ];
            let m = (right - left - e * 2.) / (image.width - edge * 2) as f32;
            let rects = [
                [left, r[1], left + e + p, r[3]],
                [left + e - m, r[1], right - e + m, r[3]],
                [right - e - p, r[1], right, r[3]],
            ];
            for ((part, rect), clip) in parts.iter().zip(rects).zip(clips) {
                self.clipped_image_alpha(part, rect, clip, opacity);
            }
        }
    }
    pub fn image_snapped(&self, image: &Image, r: [f32; 4]) {
        let width = ((r[2] - r[0]) * self.scale).round();
        let height = ((r[3] - r[1]) * self.scale).round();
        let x = (((r[0] + r[2]) * self.scale - width) * 0.5).round() / self.scale;
        let y = (r[1] * self.scale).round() / self.scale;
        self.image(
            image,
            [x, y, x + width / self.scale, y + height / self.scale],
        );
    }
    pub fn image(&self, image: &Image, r: [f32; 4]) {
        self.image_alpha(image, r, 1.);
    }
    pub fn image_cpu(&self, image: &Image, r: [f32; 4]) {
        self.image_cpu_alpha(image, r, 1.);
    }
    pub fn image_cpu_alpha(&self, image: &Image, r: [f32; 4], opacity: f32) {
        unsafe {
            if let (Some(target), Some(pixels)) = (self.target, image._pixels.as_ref()) {
                GdipFlush(self.g, 1);
                GdiFlush();
                target.draw(
                    pixels,
                    [image.width as usize, image.height as usize],
                    r.map(|v| v * self.scale),
                    self.transform.get(),
                    opacity,
                    image.tint,
                );
            } else {
                self.image_alpha(image, r, opacity);
            }
        }
    }
    pub fn image_alpha(&self, image: &Image, r: [f32; 4], opacity: f32) {
        let _cost = Cost::new(2);
        let w = ((r[2] - r[0]) * self.scale).round().max(1.) / self.scale;
        let h = ((r[3] - r[1]) * self.scale).round().max(1.) / self.scale;
        let cx = (r[0] + r[2]) * 0.5;
        let cy = (r[1] + r[3]) * 0.5;
        let r = [cx - w * 0.5, cy - h * 0.5, cx + w * 0.5, cy + h * 0.5];
        {
            if let (Some(target), Some(pixels)) = (self.target, image._pixels.as_ref()) {
                unsafe {
                    GdipFlush(self.g, 1);
                    GdiFlush();
                    target.draw(
                        pixels,
                        [image.width as usize, image.height as usize],
                        r.map(|v| v * self.scale),
                        self.transform.get(),
                        opacity,
                        image.tint,
                    );
                }
                return;
            }
        }
        unsafe {
            // Static sprite dimensions snap once to device pixels; the center
            // retains continuous subpixel translation. Animated scaling uses
            // image_cpu instead, avoiding GDI+'s fractional-size wobble.
            let width = ((r[2] - r[0]) * self.scale).round().max(1.);
            let height = ((r[3] - r[1]) * self.scale).round().max(1.);
            let x = (r[0] + r[2]) * self.scale * 0.5 - width * 0.5;
            let y = (r[1] + r[3]) * self.scale * 0.5 - height * 0.5;
            let mut attributes = null_mut();
            if opacity < 0.9999 {
                let mut matrix = ColorMatrix([0.; 25]);
                for i in 0..5 {
                    matrix.0[i * 5 + i] = 1.;
                }
                matrix.0[18] = opacity;
                GdipCreateImageAttributes(&mut attributes);
                GdipSetImageAttributesColorMatrix(attributes, 1, 1, &matrix, std::ptr::null(), 0);
            }
            GdipDrawImageRectRect(
                self.g,
                image.raw,
                x,
                y,
                width,
                height,
                0.,
                0.,
                image.width as f32,
                image.height as f32,
                2,
                attributes,
                null_mut(),
                null_mut(),
            );
            if !attributes.is_null() {
                GdipDisposeImageAttributes(attributes);
            }
        }
    }
    pub fn text(&self, text: &str, r: [f32; 4], size: f32, bold: bool, color: u32, align: i32) {
        self.text_alpha(text, r, size, bold, color, align, 255);
    }
    pub fn text_alpha(
        &self,
        text: &str,
        r: [f32; 4],
        size: f32,
        bold: bool,
        color: u32,
        align: i32,
        opacity: u8,
    ) {
        self.text_weight(
            text,
            r,
            size,
            if bold { 700 } else { 400 },
            color,
            align,
            opacity,
        );
    }
    /// Monospaced content typography; uses the same DPI-aware font cache.
    pub fn text_monospace(
        &self,
        text: &str,
        r: [f32; 4],
        size: f32,
        bold: bool,
        color: u32,
        align: i32,
    ) {
        self.text_weight(
            text,
            r,
            size,
            if bold { 1700 } else { 1400 },
            color,
            align,
            255,
        );
    }
    /// Native display typography for legacy Clock content (no bitmap stretching).
    pub fn text_clock_display(
        &self,
        text: &str,
        r: [f32; 4],
        size: f32,
        color: u32,
        align: i32,
        impact: bool,
    ) {
        self.text_weight(
            text,
            r,
            size,
            if impact { 2400 } else { 3400 },
            color,
            align,
            255,
        );
    }
    /// Opt-in DirectWrite measurement helper using `crate::d2d` (no GDI+ fallback alteration).
    #[allow(dead_code)]
    pub fn measure_text_dwrite(&self, text: &str, size: f32, bold: bool) -> Option<[f32; 2]> {
        crate::d2d::measure_text(
            text,
            &crate::d2d::TextStyle {
                size,
                weight: if bold { 700 } else { 400 },
                ..Default::default()
            },
            10_000.0,
            1_000.0,
        )
        .map(|m| [m.width, m.height])
    }
    /// Measure with the very same font/format used to draw Clock's live time.
    pub fn measure_clock_display(&self,text:&str,size:f32)->f32 {
        unsafe {let font=font_at(size*self.scale,3400);let mut format=null_mut();GdipCreateStringFormat(0x1000,0,&mut format);
        let layout=Rect{x:0.,y:0.,w:10000.,h:1000.};let mut bounds=Rect{x:0.,y:0.,w:0.,h:0.};let text=wide(text);
        let result=GdipMeasureString(self.g,text.as_ptr(),text.len()as i32-1,font,&layout,format,&mut bounds,null_mut(),null_mut());GdipDeleteStringFormat(format);
        if result==0 {bounds.w/self.scale}else{size*3.2}}
    }
    pub fn measure_clock_label(&self,text:&str,size:f32,bold:bool)->f32 {
        unsafe {let font=font_at(size*self.scale,if bold{700}else{400});let mut format=null_mut();GdipCreateStringFormat(0x1000,0,&mut format);
        let layout=Rect{x:0.,y:0.,w:10000.,h:1000.};let mut bounds=Rect{x:0.,y:0.,w:0.,h:0.};let text=wide(text);
        let result=GdipMeasureString(self.g,text.as_ptr(),text.len()as i32-1,font,&layout,format,&mut bounds,null_mut(),null_mut());GdipDeleteStringFormat(format);
        if result==0 {bounds.w/self.scale}else{size}}
    }
    pub fn measure_clock_monospace(&self,text:&str,size:f32)->f32 {
        unsafe {let font=font_at(size*self.scale,1400);let mut format=null_mut();GdipCreateStringFormat(0x1000,0,&mut format);
        let layout=Rect{x:0.,y:0.,w:10000.,h:1000.};let mut bounds=Rect{x:0.,y:0.,w:0.,h:0.};let text=wide(text);
        let result=GdipMeasureString(self.g,text.as_ptr(),text.len()as i32-1,font,&layout,format,&mut bounds,null_mut(),null_mut());GdipDeleteStringFormat(format);
        if result==0 {bounds.w/self.scale}else{size*3.75}}
    }
    pub fn text_light(&self, text: &str, r: [f32; 4], size: f32, color: u32, align: i32) {
        let r = r.map(|v| (v * self.scale).round() / self.scale);
        self.text_weight(
            text,
            r,
            (size * self.scale).round() / self.scale,
            3400,
            color,
            align,
            255,
        );
    }
    pub fn text_medium(&self, text: &str, r: [f32; 4], size: f32, color: u32, align: i32) {
        let r = r.map(|v| (v * self.scale).round() / self.scale);
        self.text_weight(
            text,
            r,
            (size * self.scale).round() / self.scale,
            600,
            color,
            align,
            255,
        );
    }
    pub fn text_styled(
        &self,
        text: &str,
        r: [f32; 4],
        size: f32,
        style: u8,
        color: u32,
        align: i32,
        opacity: u8,
    ) {
        self.text_weight(
            text,
            r,
            size,
            400 | (((style & 0x0f) as u16) << 12),
            color,
            align,
            opacity,
        );
    }
    fn text_weight(
        &self,
        text: &str,
        r: [f32; 4],
        size: f32,
        weight: u16,
        color: u32,
        align: i32,
        opacity: u8,
    ) {
        unsafe {
            GdipSetTextRenderingHint(
                self.g,
                if self.transform.get()[3].abs() < 0.001 {
                    3
                } else {
                    4
                },
            );
            let font = font_at(size * self.scale, weight);
            let mut format = null_mut();
            GdipCreateStringFormat(0x1000, 0, &mut format);
            GdipSetStringFormatAlign(format, align);
            GdipSetStringFormatLineAlign(format, 1);
            GdipSetStringFormatTrimming(format, 3);
            let mut brush = null_mut();
            GdipCreateSolidFill(argb(color, opacity), &mut brush);
            let r = Rect {
                x: r[0] * self.scale,
                y: r[1] * self.scale,
                w: (r[2] - r[0]) * self.scale,
                h: (r[3] - r[1]) * self.scale,
            };
            let text = wide(text);
            GdipDrawString(
                self.g,
                text.as_ptr(),
                text.len() as i32 - 1,
                font,
                &r,
                format,
                brush,
            );
            GdipDeleteBrush(brush);
            GdipDeleteStringFormat(format);
        }
    }
    /// Measure an inline rich-text run using typographic GDI+ metrics (including trailing spaces).
    pub fn measure_rich_run(&self, text: &str, family: Option<&str>, size: f32, style: u8) -> f32 {
        if text.is_empty() {
            return 0.;
        }
        unsafe {
            let font = styled_font_at(family, size * self.scale, style);
            let format = rich_string_format();
            let layout = Rect {
                x: 0.,
                y: 0.,
                w: 10000.,
                h: 1000.,
            };
            let mut bounds = Rect {
                x: 0.,
                y: 0.,
                w: 0.,
                h: 0.,
            };
            let wtext = wide(text);
            let result = GdipMeasureString(
                self.g,
                wtext.as_ptr(),
                wtext.len() as i32 - 1,
                font,
                &layout,
                format,
                &mut bounds,
                null_mut(),
                null_mut(),
            );
            GdipDeleteStringFormat(format);
            if result == 0 {
                bounds.w / self.scale
            } else {
                text.chars().count() as f32 * size * 0.55
            }
        }
    }
    /// Draw an inline rich-text run with custom font family, size, and style bits (1=B, 2=I, 4=U, 8=S).
    pub fn text_rich_run(
        &self,
        text: &str,
        r: [f32; 4],
        family: Option<&str>,
        size: f32,
        style: u8,
        color: u32,
        opacity: u8,
    ) {
        if text.is_empty() {
            return;
        }
        unsafe {
            GdipSetTextRenderingHint(
                self.g,
                if self.transform.get()[3].abs() < 0.001 {
                    3
                } else {
                    4
                },
            );
            let font = styled_font_at(family, size * self.scale, style);
            let format = rich_string_format();
            let mut brush = null_mut();
            GdipCreateSolidFill(argb(color, opacity), &mut brush);
            let r = Rect {
                x: r[0] * self.scale,
                y: r[1] * self.scale,
                w: (r[2] - r[0]).max(1.) * self.scale,
                h: (r[3] - r[1]).max(1.) * self.scale,
            };
            let wtext = wide(text);
            GdipDrawString(
                self.g,
                wtext.as_ptr(),
                wtext.len() as i32 - 1,
                font,
                &r,
                format,
                brush,
            );
            GdipDeleteBrush(brush);
            GdipDeleteStringFormat(format);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_pane_material_matches_direct_layers_and_reuses_the_image() {
        let _ui = crate::native_ui_test_lock();
        let _runtime = Runtime::start().unwrap();
        unsafe {
            for dpi in [1., 1.25, 1.75, 2.] {
                let (w, h) = ((320. * dpi) as i32, (180. * dpi) as i32);
                let rect = [0., 0., w as f32 / dpi, h as f32 / dpi];
                for (tint, light) in [(0., false), (0.6, true)] {
                    let mut direct = crate::compositor::Buffer::plain(w, h, dpi).unwrap();
                    let mut cached = crate::compositor::Buffer::plain(w, h, dpi).unwrap();
                    let initial = [30u8, 80, 120, 255].repeat((w * h) as usize);
                    direct.pixels_mut().copy_from_slice(&initial);
                    cached.pixels_mut().copy_from_slice(&initial);
                    {
                        let _batch = AlphaBatch::new(direct.dc, dpi);
                        let c = Canvas::new(direct.dc, dpi);
                        c.reflection_gradient(rect, tint, light);
                        c.draw_grain(rect, dpi);
                    }
                    {
                        let _batch = AlphaBatch::new(cached.dc, dpi);
                        Canvas::new(cached.dc, dpi).draw_pane_overlay([w as u32, h as u32], dpi, tint, light, rect);
                    }
                    let expected = direct.snapshot();
                    let actual = cached.snapshot();
                    let delta = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                    assert!(delta <= 2, "cached material changed colors/alpha: max delta={delta}, dpi={dpi}");
                    let a = Canvas::pane_overlay([w as u32, h as u32], dpi, tint, light).unwrap();
                    let b = Canvas::pane_overlay([w as u32, h as u32], dpi, tint, light).unwrap();
                    assert!(std::rc::Rc::ptr_eq(&a, &b), "warm material must reuse its pixel buffer and GDI+ image");
                }
            }
        }
    }

    #[test]
    fn reused_labels_never_stretch_when_icon_size_changes() {
        let _runtime = Runtime::start().unwrap();
        let icon = Image::from_rgba(1, 1, vec![0; 4]).unwrap();
        for dpi in [1., 1.25, 1.75, 2.] {
            let label = Canvas::label("Recordly", 108., 20., dpi, 0xffffff).unwrap();
            let source = label._pixels.as_ref().unwrap();
            for icon_size in [36., 48., 60.] {
                let size = [icon_size + 42., icon_size + 42.];
                let (pad, tile) =
                    Canvas::tile(&icon, None, Some((&label, None)), size, icon_size, dpi).unwrap();
                let rect = label_rect(
                    &label,
                    pad + size[0] * 0.5,
                    pad + icon_size + 14.,
                    dpi,
                    true,
                );
                let x = (rect[0] * dpi).round() as i32;
                let y = (rect[1] * dpi).round() as i32;
                let dest = tile._pixels.as_ref().unwrap();
                for sy in 0..label.height {
                    for sx in 0..label.width {
                        let i = ((sy * label.width + sx) * 4) as usize;
                        if source[i + 3] == 0 {
                            continue;
                        }
                        let dx = x + sx as i32;
                        let dy = y + sy as i32;
                        assert!(dx >= 0 && dx < tile.width as i32);
                        let d = ((dy as u32 * tile.width + dx as u32) * 4) as usize;
                        assert_eq!(&dest[d..d + 4], &source[i..i + 4]);
                    }
                }
            }
        }
    }
    #[test]
    fn translucent_background_preserves_opaque_content_and_alpha_during_fade() {
        let _runtime = Runtime::start().unwrap();
        unsafe {
            for opacity in [0, 64, 128] {
                let mut buffer = crate::compositor::Buffer::plain(100, 100, 1.).unwrap();
                let brush = CreateSolidBrush(0x808080);
                FillRect(
                    buffer.dc,
                    &windows_sys::Win32::Foundation::RECT {
                        left: 0,
                        top: 0,
                        right: 100,
                        bottom: 100,
                    },
                    brush,
                );
                DeleteObject(brush);
                buffer.background_alpha(opacity);
                let base = crate::compositor::Buffer::plain(100, 100, 1.).unwrap();
                base.copy_from(buffer.dc);
                {
                    let _batch = AlphaBatch::new(buffer.dc, 1.).unwrap();
                    let c = Canvas::new(buffer.dc, 1.);
                    c.round([20., 20., 50., 50.], 5., 0xffffff, 255, false, false);
                    c.text("Text", [10., 55., 90., 85.], 13., true, 0xffffff, 0);
                    flush(buffer.dc);
                    let image = Image::from_rgba(8, 8, [0, 0, 255, 255].repeat(64)).unwrap();
                    c.image(&image, [60., 20., 68., 28.]);
                }
                let pixels = buffer.snapshot();
                assert_eq!(pixels[3], opacity);
                assert_eq!(pixels[(30 * 100 + 30) * 4 + 3], 255);
                assert_eq!(pixels[(23 * 100 + 63) * 4 + 3], 255);
                assert!(pixels
                    .chunks_exact(4)
                    .all(|p| p[..3].iter().all(|c| *c <= p[3])));
                assert!(pixels[(55 * 100 * 4)..(85 * 100 * 4)]
                    .chunks_exact(4)
                    .any(|p| p[3] > opacity));
                buffer.fade_list(&base, 35., 80.);
                let faded = buffer.snapshot();
                assert_eq!(faded[(30 * 100 + 30) * 4 + 3], opacity);
                assert!(faded
                    .chunks_exact(4)
                    .all(|p| p[..3].iter().all(|c| *c <= p[3])));
            }
        }
    }
    #[test]
    fn widget_reflection_keeps_native_corner_geometry_when_panel_size_changes() {
        let _runtime = Runtime::start().unwrap();
        for dpi in [1., 1.75] {
            let mut corners = Vec::new();
            for [w, h] in [[700., 240.], [1000., 900.]] {
                let image = Image::from_rgba(
                    (w * dpi) as u32,
                    (h * dpi) as u32,
                    vec![0; ((w * dpi) as u32 * (h * dpi) as u32 * 4) as usize],
                )
                .unwrap();
                unsafe {
                    let c = Canvas::on_image(&image, dpi).unwrap();
                    c.widget_corner_reflection([4., 4., w - 4., h - 4.], 34., false);
                }
                let pixels = image._pixels.as_ref().unwrap();
                let mut corner = Vec::new();
                for y in 0..(70. * dpi) as u32 {
                    let start = (y * image.width * 4) as usize;
                    corner.extend_from_slice(&pixels[start..start + (70. * dpi) as usize * 4]);
                }
                assert!(corner.chunks_exact(4).any(|p| p[3] > 25));
                corners.push(corner);
            }
            assert_eq!(corners[0], corners[1]);
        }
    }
    #[test]
    fn native_dpi_labels_survive_tile_composition_without_resampling() {
        let _runtime = Runtime::start().unwrap();
        let icon = Image::from_rgba(1, 1, vec![0; 4]).unwrap();
        for dpi in [1., 1.25, 1.5, 1.75, 2.] {
            for color in [0x17200c, 0xfdfaf8] {
                let size = [108., 86.];
                let label =
                    Canvas::label("Client Riot · Affinity", size[0] - 6., 20., dpi, color).unwrap();
                let (pad, tile) =
                    Canvas::tile(&icon, None, Some((&label, None)), size, 48., dpi).unwrap();
                assert_eq!(label.width, ((size[0] - 6.) * dpi).round() as u32);
                let x = ((pad + size[0] * 0.5) * dpi - label.width as f32 * 0.5).round() as usize;
                let y = ((pad + 48. + 14.) * dpi).round() as usize;
                let source = label._pixels.as_ref().unwrap();
                let dest = tile._pixels.as_ref().unwrap();
                assert!(source.chunks_exact(4).any(|p| p[3] > 200));
                for row in 0..label.height as usize {
                    let begin = ((y + row) * tile.width as usize + x) * 4;
                    let source_begin = row * label.width as usize * 4;
                    for byte in 0..label.width as usize * 4 {
                        assert_eq!(
                            dest[begin + byte],
                            source[source_begin + byte],
                            "dpi={dpi} row={row} byte={byte} origin={x},{y}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn recessed_search_pane_transmits_light_without_a_raised_highlight() {
        let _runtime = Runtime::start().unwrap();
        unsafe {
            let buffer = crate::compositor::Buffer::plain(360, 100, 1.).unwrap();
            let brush = CreateSolidBrush(0xb4b4b4);
            FillRect(
                buffer.dc,
                &windows_sys::Win32::Foundation::RECT {
                    left: 0,
                    top: 0,
                    right: 360,
                    bottom: 100,
                },
                brush,
            );
            DeleteObject(brush);
            {
                let c = Canvas::new(buffer.dc, 1.);
                c.search_surface([10., 20., 350., 80.], 30., 0x00563621, false);
            }
            let pixels = buffer.snapshot();
            let at = |x: usize, y: usize| {
                pixels[(y * 360 + x) * 4] as i32
                    + pixels[(y * 360 + x) * 4 + 1] as i32
                    + pixels[(y * 360 + x) * 4 + 2] as i32
            };
            assert!((490..550).contains(&at(180, 50)));
            assert!(at(45, 38) > at(315, 62));
        }
    }
    #[test]
    fn search_glass_keeps_the_selected_hue_without_panel_tint() {
        let _runtime = Runtime::start().unwrap();
        let mut samples = Vec::new();
        unsafe {
            for color in [0x00304090, 0x00904030] {
                let buffer = crate::compositor::Buffer::plain(160, 80, 1.).unwrap();
                let brush = CreateSolidBrush(0xb4b4b4);
                FillRect(
                    buffer.dc,
                    &windows_sys::Win32::Foundation::RECT {
                        left: 0,
                        top: 0,
                        right: 160,
                        bottom: 80,
                    },
                    brush,
                );
                DeleteObject(brush);
                {
                    let c = Canvas::new(buffer.dc, 1.);
                    c.search_surface([10., 10., 150., 70.], 20., color, false);
                }
                samples
                    .push(buffer.snapshot()[(40 * 160 + 80) * 4..(40 * 160 + 80) * 4 + 3].to_vec());
            }
        }
        assert!(samples[0][2] > samples[1][2]);
        assert!(samples[0][0] < samples[1][0]);
    }
    #[test]
    fn search_material_tone_tracks_widget_theme() {
        let _runtime = Runtime::start().unwrap();
        unsafe {
            for light in [false, true] {
                let buffer = crate::compositor::Buffer::plain(160, 80, 1.).unwrap();
                let brush = CreateSolidBrush(0xb4b4b4);
                FillRect(
                    buffer.dc,
                    &windows_sys::Win32::Foundation::RECT {
                        left: 0,
                        top: 0,
                        right: 160,
                        bottom: 80,
                    },
                    brush,
                );
                DeleteObject(brush);
                {
                    let c = Canvas::new(buffer.dc, 1.);
                    c.glass_surface([10., 10., 150., 70.], 20., 38, light);
                }
                let pixels = buffer.snapshot();
                let value = pixels[(40 * 160 + 80) * 4];
                if light {
                    assert!(value > 180);
                } else {
                    assert!(value < 180);
                }
            }
        }
    }
    #[test]
    fn horizontal_resize_has_no_transparent_seams() {
        let _runtime = Runtime::start().unwrap();
        unsafe {
            let buffer = crate::compositor::Buffer::plain(160, 64, 1.).unwrap();
            PatBlt(buffer.dc, 0, 0, 160, 64, BLACKNESS);
            let image = Image::from_rgba(24, 32, [80, 60, 20, 128].repeat(24 * 32)).unwrap();
            {
                let canvas = Canvas::new(buffer.dc, 1.);
                canvas.image_horizontal(&image, [8.25, 8., 150.25, 40.], 5);
            }
            let pixels = buffer.snapshot();
            for x in 9..149 {
                assert_eq!(
                    &pixels[(24 * 160 + x) * 4..(24 * 160 + x) * 4 + 3],
                    &[20, 60, 80]
                );
            }
        }
    }
    #[test]
    fn batching_preserves_native_cpu_and_following_native_draws() {
        let _runtime = Runtime::start().unwrap();
        unsafe {
            let buffer = crate::compositor::Buffer::plain(64, 64, 1.).unwrap();
            PatBlt(buffer.dc, 0, 0, 64, 64, BLACKNESS);
            let green = Image::from_rgba(16, 16, [0, 255, 0, 255].repeat(16 * 16)).unwrap();
            {
                let _batch = Batch::new(buffer.dc, 1.);
                {
                    let c = Canvas::new(buffer.dc, 1.);
                    c.round([2., 2., 14., 14.], 1., 0x0000ff, 255, false, false);
                }
                {
                    let c = Canvas::new(buffer.dc, 1.);
                    c.image_cpu(&green, [16., 16., 32., 32.]);
                }
                {
                    let c = Canvas::new(buffer.dc, 1.);
                    c.round([40., 40., 50., 50.], 1., 0xff0000, 255, false, false);
                }
            }
            let pixels = buffer.snapshot();
            assert_eq!(
                &pixels[(5 * 64 + 5) * 4..(5 * 64 + 5) * 4 + 3],
                &[0, 0, 255]
            );
            assert_eq!(
                &pixels[(20 * 64 + 20) * 4..(20 * 64 + 20) * 4 + 3],
                &[0, 255, 0]
            );
            assert_eq!(
                &pixels[(45 * 64 + 45) * 4..(45 * 64 + 45) * 4 + 3],
                &[255, 0, 0]
            );
        }
    }
    #[test]
    #[ignore = "Rendering performance diagnostic"]
    fn image_batch_benchmark() {
        let _runtime = Runtime::start().unwrap();
        unsafe {
            let buffer = crate::compositor::Buffer::plain(1024, 768, 1.).unwrap();
            let mut pixels = vec![0; 160 * 160 * 4];
            for y in 16..144 {
                for x in 16..144 {
                    let i = (y * 160 + x) * 4;
                    pixels[i..i + 4].copy_from_slice(&[120, 80, 220, 255]);
                }
            }
            let image = Image::from_rgba(160, 160, pixels).unwrap();
            for batch in [false, true] {
                let start = std::time::Instant::now();
                for _ in 0..30 {
                    PatBlt(buffer.dc, 0, 0, 1024, 768, BLACKNESS);
                    let _batch = if batch {
                        Some(Batch::new(buffer.dc, 1.))
                    } else {
                        None
                    };
                    for i in 0..24 {
                        let c = Canvas::new(buffer.dc, 1.);
                        let x = (i % 6) * 160;
                        let y = (i / 6) * 170;
                        c.image(
                            &image,
                            [x as f32, y as f32, x as f32 + 160., y as f32 + 160.],
                        );
                    }
                }
                println!(
                    "Batch {batch}: {:.2} ms/frame",
                    start.elapsed().as_secs_f64() * 1000. / 30.
                );
            }
        }
    }
    #[test]
    fn cached_shadow_fades_without_opacity_steps_or_reversals() {
        let _runtime = Runtime::start().unwrap();
        let mut previous = 0u8;
        for amount in 0..=32 {
            let target = Image::from_rgba(96, 96, vec![0; 96 * 96 * 4]).unwrap();
            unsafe {
                let canvas = Canvas::on_image(&target, 1.).unwrap();
                canvas.soft_shadow([24., 24., 64., 64.], 10., amount as f32, 4., 2.);
            }
            let alpha = target._pixels.as_ref().unwrap()[(45 * 96 + 65) * 4 + 3];
            assert!(alpha >= previous && alpha - previous <= 2);
            previous = alpha;
        }
        assert!(previous > 0);
    }
    #[test]
    fn transparent_icons_produce_transparent_shadows() {
        let _runtime = Runtime::start().unwrap();
        let source = Image::from_rgba(16, 16, vec![0; 16 * 16 * 4]).unwrap();
        let shadow = source.shadow(1.5, 96, 4).unwrap();
        assert!(shadow._pixels.as_ref().unwrap().iter().all(|v| *v == 0));
        let (_, shadow) = source.icon_shadow().unwrap();
        assert!(shadow._pixels.as_ref().unwrap().iter().all(|v| *v == 0));
    }
    #[test]
    fn round_icon_shadow_and_cached_tile_have_clear_uncut_edges() {
        let _runtime = Runtime::start().unwrap();
        for dpi in [1., 1.75, 2.] {
            let side = (48f32 * dpi * 1.12).ceil() as u32;
            let icon =
                Image::from_rgba(side, side, [255; 4].repeat((side * side) as usize)).unwrap();
            let (pad, shadow) = icon.icon_shadow().unwrap();
            let pixels = shadow._pixels.as_ref().unwrap();
            let center = shadow.width / 2;
            let at = |x: u32, y: u32| pixels[((y * shadow.width + x) * 4 + 3) as usize];
            assert!(at(center, center) > at(center, center + side / 2));
            assert_eq!(at(center + side / 2, center), at(center, center + side / 2));
            let (_, tile) =
                Canvas::tile(&icon, Some((pad, &shadow)), None, [86., 82.], 48., dpi).unwrap();
            for image in [&shadow, &tile] {
                let pixels = image._pixels.as_ref().unwrap();
                for y in 0..image.height {
                    for x in 0..image.width {
                        if x == 0 || y == 0 || x + 1 == image.width || y + 1 == image.height {
                            assert_eq!(pixels[((y * image.width + x) * 4 + 3) as usize], 0);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn windows_mask_transparency_does_not_turn_black_and_real_black_is_preserved() {
        assert_eq!(
            recover_icon_alpha(&[0, 0, 0, 0], &[255, 255, 255, 0]),
            vec![0, 0, 0, 0]
        );
        assert_eq!(
            recover_icon_alpha(&[0, 0, 0, 0], &[0, 0, 0, 0]),
            vec![0, 0, 0, 255]
        );
        assert_eq!(
            recover_icon_alpha(&[128, 128, 128, 0], &[255, 255, 255, 0]),
            vec![128, 128, 128, 128]
        );
    }
    #[test]
    fn moving_image_preserves_fractional_pixel_positions() {
        let _runtime = Runtime::start().unwrap();
        let source = Image::from_rgba(16, 16, vec![255; 16 * 16 * 4]).unwrap();
        for width in [16., 16.6, 47.3] {
            let mut previous = None;
            for i in 0..5 {
                let target = Image::from_rgba(96, 48, vec![0; 96 * 48 * 4]).unwrap();
                let x = 12. + i as f32 * 0.25;
                unsafe {
                    let canvas = Canvas::on_image(&target, 1.).unwrap();
                    canvas.image(&source, [x, 12., x + width, 28.]);
                }
                let pixels = target._pixels.as_ref().unwrap();
                let mut weight = 0.;
                let mut sum = 0.;
                for (index, p) in pixels.chunks_exact(4).enumerate() {
                    weight += p[3] as f32;
                    sum += (index % 96) as f32 * p[3] as f32;
                }
                let center = sum / weight;
                if let Some(old) = previous {
                    assert!(
                        center - old > 0.10,
                        "subpixel motion snapped at width {width}: {old} -> {center}"
                    );
                }
                previous = Some(center);
            }
        }
    }
    #[test]
    fn hover_scale_keeps_the_center_fixed_and_grows_continuously() {
        let _runtime = Runtime::start().unwrap();
        let source = Image::from_rgba(16, 16, vec![255; 16 * 16 * 4]).unwrap();
        let mut previous = 0.;
        for i in 0..12 {
            let target = Image::from_rgba(48, 48, vec![0; 48 * 48 * 4]).unwrap();
            let extra = i as f32 * 0.10;
            unsafe {
                let canvas = Canvas::on_image(&target, 1.).unwrap();
                canvas.image_cpu(
                    &source,
                    [12. - extra, 12. - extra, 28. + extra, 28. + extra],
                );
            }
            let pixels = target._pixels.as_ref().unwrap();
            let mut weight = 0.;
            let mut sum = 0.;
            for (index, p) in pixels.chunks_exact(4).enumerate() {
                weight += p[3] as f32;
                sum += (index % 48) as f32 * p[3] as f32;
            }
            assert!(weight > previous, "scale reversed at frame {i}");
            assert!(
                (sum / weight - 19.5).abs() < 0.06,
                "scale center wobbled at frame {i}: {}",
                sum / weight
            );
            previous = weight;
        }
    }
    #[test]
    fn batch_hover_surface_and_glow_reuse_caches_without_gdi_graphics() {
        let _runtime = Runtime::start().unwrap();
        clear_caches();
        unsafe {
            let rect = [20., 20., 120., 100.];
            let buffer = crate::compositor::Buffer::plain(160, 120, 1.).unwrap();
            PatBlt(buffer.dc, 0, 0, 160, 120, BLACKNESS);
            {
                let _alpha = AlphaBatch::new(buffer.dc, 1.).unwrap();
                let batch = Batch::new(buffer.dc, 1.);
                assert!(batch.hover_surface(rect, 20., 0.85, false));
                assert!(batch.hover_glow(rect, 20., [70., 60.], 40));
            }
            let pixels = buffer.snapshot();
            let center_alpha = pixels[(60 * 160 + 70) * 4 + 3];
            assert!(center_alpha > 0);
            assert_eq!(pixels[(2 * 160 + 2) * 4 + 3], 0);

            let glow_only = crate::compositor::Buffer::plain(160, 120, 1.).unwrap();
            PatBlt(glow_only.dc, 0, 0, 160, 120, BLACKNESS);
            {
                let _alpha = AlphaBatch::new(glow_only.dc, 1.).unwrap();
                let batch = Batch::new(glow_only.dc, 1.);
                assert!(batch.hover_glow(rect, 20., [70., 60.], 80));
            }
            let glow_pixels = glow_only.snapshot();
            assert!(glow_pixels[(60 * 160 + 70) * 4 + 3] > 0);
            assert_eq!(glow_pixels[(21 * 160 + 21) * 4 + 3], 0);

            let repeat = crate::compositor::Buffer::plain(160, 120, 1.).unwrap();
            PatBlt(repeat.dc, 0, 0, 160, 120, BLACKNESS);
            take_cost();
            {
                let batch = Batch::new(repeat.dc, 1.);
                assert!(batch.hover_surface(rect, 20., 0.85, false));
                assert!(batch.hover_glow(rect, 20., [70., 60.], 40));
                assert_eq!(ACTIVE_BATCH.with(|a| a.get()), Some((repeat.dc as usize, 0)));
            }
            let cost = take_cost();
            assert_eq!(cost[0], 0);
            assert_eq!(cost[1], 0);
            let repeat_pixels = repeat.snapshot();
            assert!(repeat_pixels[(60 * 160 + 70) * 4] > 0);
        }
    }
    #[test]
    fn canvas_draws_after_restore_dc_expands_clip_even_when_batch_is_held() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = Runtime::start().unwrap();
        unsafe {
            let w = 160;
            let h = 120;
            let buffer = crate::compositor::Buffer::plain(w, h, 1.).unwrap();
            PatBlt(buffer.dc, 0, 0, w, h, BLACKNESS);
            {
                let _batch = Batch::new(buffer.dc, 1.);
                // Flushing a lazy Batch (g == 0) must be safe and a no-op for GdipFlush.
                flush(buffer.dc);
                let saved = SaveDC(buffer.dc);
                IntersectClipRect(buffer.dc, 0, 54, w, h);
                {
                    let c1 = Canvas::new(buffer.dc, 1.);
                    c1.round([20., 64., 120., 94.], 8., 0x224466, 255, false, false);
                }
                RestoreDC(buffer.dc, saved);
                // Even though `_batch` is still alive, clip expanded to y = 0..120,
                // so Canvas::new must not reuse the y >= 54 GDI+ graphics clip.
                let c2 = Canvas::new(buffer.dc, 1.);
                assert!(c2.owned);
                c2.round([20., 10., 120., 40.], 8., 0xffffff, 255, false, false);
            }
            let pixels = buffer.snapshot();
            let has_top_pixels = (10..40usize).any(|y| {
                (20..120usize).any(|x| {
                    let i = (y * w as usize + x) * 4;
                    pixels[i] > 200 && pixels[i + 1] > 200 && pixels[i + 2] > 200
                })
            });
            assert!(
                has_top_pixels,
                "expected GDI+ pixels in y = 10..40 after RestoreDC with active Batch"
            );
        }
    }
    #[test]
    fn ortho_group_polygon_merges_l_shape_and_collapses_uniform_rows() {
        let l_shape = ortho_group_polygon(&[
            [16., 70., 700., 160.],
            [16., 164., 260., 254.],
        ]);
        assert_eq!(
            l_shape,
            vec![
                [700., 70.],
                [700., 160.],
                [260., 160.],
                [260., 254.],
                [16., 254.],
                [16., 70.],
            ]
        );
        let rect = ortho_group_polygon(&[
            [16., 70., 700., 160.],
            [16., 164., 700., 254.],
        ]);
        assert_eq!(
            rect,
            vec![
                [700., 70.],
                [700., 254.],
                [16., 254.],
                [16., 70.],
            ]
        );
    }
    #[test]
    fn split_connected_group_spans_separates_non_overlapping_wraps() {
        let wrapped = split_connected_group_spans(
            &[
                [560., 70., 740., 160.],
                [16., 164., 196., 254.],
            ],
            30.0,
        );
        assert_eq!(wrapped.len(), 2);

        let connected = split_connected_group_spans(
            &[
                [16., 70., 740., 160.],
                [16., 164., 196., 254.],
            ],
            30.0,
        );
        assert_eq!(connected.len(), 1);
        assert_eq!(connected[0].len(), 2);
    }
    #[test]
    fn poly_group_material_and_glass_restore_render_cleanly_at_exact_radius() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = Runtime::start().unwrap();
        unsafe {
            let w = 420;
            let h = 240;
            let mut backing = crate::compositor::Buffer::plain(w, h, 1.).unwrap();
            PatBlt(backing.dc, 0, 0, w, h, WHITENESS);
            let target = crate::compositor::Buffer::plain(w, h, 1.).unwrap();
            PatBlt(target.dc, 0, 0, w, h, BLACKNESS);
            let spans = [
                [20.0f32, 20.0, 380.0, 110.0],
                [20.0f32, 114.0, 180.0, 204.0],
            ];
            backing.restore_glass_group(target.dc, &spans, 20.0, false);
            let batch = Batch::new(target.dc, 1.);
            assert!(batch.group_poly_surface(&spans, 20.0, true));
            drop(batch);
            let px = target.snapshot();
            // Inside row 0 overhang [x=300, y=65] -> restored opaque backing + overlay.
            assert!(px[(65 * w as usize + 300) * 4 + 3] > 200);
            // Inside seamless bridge between row 0 and row 1 [x=100, y=112] -> opaque.
            assert!(px[(112 * w as usize + 100) * 4 + 3] > 200);
            // Concave exterior cutout below row 0 overhang [x=250, y=160] -> remains transparent.
            assert_eq!(px[(160 * w as usize + 250) * 4 + 3], 0);
            // Concave fillet corner near (180 + 4, 110 + 4) is outside; (180 - 6, 110 - 6) is inside.
            assert!(px[(104 * w as usize + 174) * 4 + 3] > 200);
            assert_eq!(px[(116 * w as usize + 186) * 4 + 3], 0);
            let _ = &mut backing;
        }
    }
    #[test]
    fn trim_padded_icon_crops_hollow_jumbo_box_and_preserves_rgb_channels() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = Runtime::start().unwrap();
        let size = 256u32;
        let mut rgba = vec![0u8; (size * size * 4) as usize];
        // Faint 2px hollow outer frame (common Windows SHIL_JUMBO artifact around small glyphs).
        for y in 0..size {
            for x in 0..size {
                if x < 2 || x >= size - 2 || y < 2 || y >= size - 2 {
                    let i = ((y * size + x) as usize) * 4;
                    rgba[i..i + 4].copy_from_slice(&[180, 180, 180, 140]);
                }
            }
        }
        // Bright RED (R=240, G=30, B=20, A=255) 36x36 inner glyph in center [110..146].
        for y in 110..146 {
            for x in 110..146 {
                let i = ((y * size + x) as usize) * 4;
                rgba[i..i + 4].copy_from_slice(&[240, 30, 20, 255]);
            }
        }
        let img = Image::from_rgba(size, size, rgba).unwrap();
        assert!(img.is_jumbo_packed_small());
        assert_eq!(img.effective_icon_bounds(), [110, 110, 146, 146]);
        let trimmed = img.trim_padded_icon();
        assert!(trimmed.width <= 44 && trimmed.height <= 44);
        let center = ((trimmed.height / 2) * trimmed.width + (trimmed.width / 2)) as usize * 4;
        let px = trimmed._pixels.as_ref().unwrap();
        // Stored pixels are BGRA: B=20, G=30, R=240, A=255 (no R/B channel swap).
        assert_eq!([px[center], px[center + 1], px[center + 2], px[center + 3]], [20, 30, 240, 255]);
    }

    #[test]
    fn group_cache_keys_stay_identical_across_fractional_scroll_offsets() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = Runtime::start().unwrap();
        let row = 90.0f32;
        let card_h = 86.0f32;
        let scrolls = [0.0f32, 0.13, 0.37, 0.51, 0.89, 12.47, 57.99];
        for dpi in [1.25f32, 1.75] {
            clear_caches();
            let target = unsafe { crate::compositor::Buffer::plain(800, 400, dpi).unwrap() };
            let batch = unsafe { Batch::new(target.dc, dpi) };
            for scroll in scrolls {
                let single = [16.0, 100.0 - scroll, 200.0, 100.0 + card_h - scroll];
                assert!(batch.group_surface(single, 20.0, false));
                let l_spans = [
                    [16.0, 100.0 - scroll, 500.0, 100.0 + card_h - scroll],
                    [16.0, 100.0 + row - scroll, 250.0, 100.0 + row + card_h - scroll],
                ];
                assert!(batch.group_poly_surface(&l_spans, 20.0, false));
                GROUPS.with(|c| assert_eq!(c.borrow().len(), 1));
                GROUP_POLYS.with(|c| assert_eq!(c.borrow().len(), 1));
            }
        }
        clear_caches();
    }
}
