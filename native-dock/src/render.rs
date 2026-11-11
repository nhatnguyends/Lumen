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
    width: u32,
    height: u32,
    alpha: Vec<u8>,
    image: Option<(u8, Image)>,
}
thread_local! {static GROUPS:RefCell<HashMap<(u32,u32,u32,u32),Image>>=RefCell::new(HashMap::new());}
thread_local! {static SHADOWS:RefCell<HashMap<(u32,u32,u32,u32,u32),ShadowSurface>>=RefCell::new(HashMap::new());}
thread_local! {static FONTS:RefCell<HashMap<(i32,u16),CachedFont>>=RefCell::new(HashMap::new());}
unsafe fn font_at(size: f32, weight: u16) -> Ptr {
    FONTS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let entry = cache
            .entry(((size * 64.).round() as i32, weight))
            .or_insert_with(|| {
                let mut family = null_mut();
                if GdipCreateFontFamilyFromName(
                    wide(if weight >= 3000 {
                        "Segoe UI Light"
                    } else if weight >= 2000 {
                        "Impact"
                    } else if weight >= 1000 {
                        "Consolas"
                    } else if weight == 600 {
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
                    if weight % 1000 >= 700 { 1 } else { 0 },
                    2,
                    &mut font,
                );
                CachedFont { font, family }
            });
        entry.font
    })
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
impl Drop for Runtime {
    fn drop(&mut self) {
        FONTS.with(|fonts| fonts.borrow_mut().clear());
        SHADOWS.with(|shadows| shadows.borrow_mut().clear());
        GROUPS.with(|groups| groups.borrow_mut().clear());
        REFLECTIONS.with(|cache| cache.borrow_mut().clear());
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
        GdipFlush(g as Ptr, 1);
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
        let canvas = Canvas {
            g,
            scale: dpi,
            target: None,
            transform: Cell::new([0., 0., 1., 0.]),
            owned: true,
            state: 0,
        };
        let previous = ALPHA_DC.with(|a| a.replace(Some(dc as usize)));
        let previous_batch = ACTIVE_BATCH.with(|a| a.replace(Some((dc as usize, g as usize))));
        Some(Self {
            batch: Some(Batch {
                _canvas: canvas,
                previous: previous_batch,
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
pub struct Batch {
    _canvas: Canvas,
    previous: Option<(usize, usize)>,
}
impl Batch {
    pub unsafe fn new(dc: HDC, dpi: f32) -> Self {
        let canvas = Canvas::new(dc, dpi);
        let previous = ACTIVE_BATCH.with(|a| a.replace(Some((dc as usize, canvas.g as usize))));
        Self {
            _canvas: canvas,
            previous,
        }
    }
}
impl Drop for Batch {
    fn drop(&mut self) {
        ACTIVE_BATCH.with(|a| a.set(self.previous));
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
                let mut alpha_mask = vec![0; (width * height) as usize];
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
                        alpha_mask[(y * width + x) as usize] =
                            (alpha * 255.).round().clamp(0., 255.) as u8;
                    }
                }
                cache.insert(
                    key,
                    ShadowSurface {
                        width,
                        height,
                        alpha: alpha_mask,
                        image: None,
                    },
                );
            }
            if let Some(surface) = cache.get_mut(&key) {
                let opacity = amount.round().clamp(0., 255.) as u8;
                if surface
                    .image
                    .as_ref()
                    .is_none_or(|(old, _)| *old != opacity)
                {
                    let mut pixels = vec![0; surface.alpha.len() * 4];
                    for (p, alpha) in pixels.chunks_exact_mut(4).zip(&surface.alpha) {
                        p[3] = ((*alpha as u32 * opacity as u32 + 127) / 255) as u8;
                    }
                    surface.image = Image::from_rgba(surface.width, surface.height, pixels)
                        .map(|image| (opacity, image));
                }
                if let Some((_, image)) = &surface.image {
                    let pad = pad as f32 / self.scale;
                    self.image(image, [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad]);
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
    pub fn pane_overlay(size: [u32; 2], dpi: f32, tint: f32, light: bool) -> Option<Image> {
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
        Some(image)
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
        let key = (
            ((r[2] - r[0]) * self.scale).round() as u32,
            ((r[3] - r[1]) * self.scale).round() as u32,
            (radius * self.scale * 16.).round() as u32,
            (self.scale * 1000.).round() as u32,
        );
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
                if cache.len() >= 16 {
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
        // Localized reflection belongs to the enclosing glass, not the search
        // pane. Its short falloff leaves the rest of the wallpaper transparent.
        unsafe {
            let clip = self.path(r, radius);
            let x = r[0] + radius * 0.65;
            let y = r[1] + 4.;
            let reach = ((r[2] - r[0]) * 0.40).clamp(160., 270.);
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
                    argb(0, (amount * 18.) as u8)
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
    fn reflection_edge(&self, r: [f32; 4], radius: f32, amount: f32) {
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
        // Complete queued GDI copies before GDI+ blends into the same DIB.
        GdiFlush();
        let shared = ACTIVE_BATCH
            .with(|a| a.get())
            .filter(|(handle, _)| *handle == dc as usize);
        let owned = shared.is_none();
        let mut state = 0;
        let g = if let Some((_, pointer)) = shared {
            let g = pointer as Ptr;
            GdipSaveGraphics(g, &mut state);
            GdipResetWorldTransform(g);
            g
        } else {
            let mut g = null_mut();
            GdipCreateFromHDC(dc, &mut g);
            GdipSetSmoothingMode(g, 4);
            GdipSetTextRenderingHint(g, 4);
            GdipSetInterpolationMode(g, 3);
            GdipSetPixelOffsetMode(g, 4);
            g
        };
        let mut dib: DIBSECTION = std::mem::zeroed();
        let bitmap = GetCurrentObject(dc, OBJ_BITMAP as u32);
        let mut clip = std::mem::zeroed();
        GetClipBox(dc, &mut clip);
        if !owned {
            GdipSetClipRect(
                g,
                clip.left as f32,
                clip.top as f32,
                (clip.right - clip.left) as f32,
                (clip.bottom - clip.top) as f32,
                0,
            );
        }
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
}
#[cfg(test)]
mod tests {
    use super::*;
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
}
