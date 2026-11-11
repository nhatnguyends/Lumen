//! Frosted wallpaper, rendered from the wallpaper FILE, never from screen pixels.
use crate::model::wide;
use image::{imageops, DynamicImage, ImageDecoder, ImageReader};
use std::{
    cell::{Cell, RefCell},
    mem::{size_of, zeroed},
    path::{Path, PathBuf},
    ptr::null_mut,
    time::SystemTime,
};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::Registry::*, UI::WindowsAndMessaging::*,
};

fn vertical_offset(viewport: f32, drawn: f32, _style: u32, tile: bool) -> f32 {
    if tile {
        0.
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

pub struct Glass {
    blend_buffer: RefCell<Option<(HWND, crate::compositor::Buffer)>>,
    generation: u64,
    accent: Cell<u32>,
    bitmap: HBITMAP,
    sharp: HBITMAP,
    width: i32,
    height: i32,
    natural: [f32; 2],
    path: PathBuf,
    modified: Option<SystemTime>,
    style: u32,
    tile: bool,
    blur: f32,
    dpi: f32,
}
impl Drop for Glass {
    fn drop(&mut self) {
        unsafe {
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.sharp.is_null() {
                DeleteObject(self.sharp);
            }
        }
    }
}
impl Glass {
    pub fn new(blur: f32, dpi: f32) -> Self {
        Self {
            blend_buffer: RefCell::new(None),
            generation: 0,
            accent: Cell::new(crate::accent::FALLBACK),
            bitmap: null_mut(),
            sharp: null_mut(),
            width: 0,
            height: 0,
            natural: [0.0; 2],
            path: PathBuf::new(),
            modified: None,
            style: 10,
            tile: false,
            blur,
            dpi,
        }
    }
    pub fn wallpaper_accent(&self) -> u32 {
        self.accent.get()
    }
    pub fn available(&self) -> bool {
        !self.bitmap.is_null()
    }
    pub fn revision(&self) -> u64 {
        self.generation
    }
    /// Test-only file source. Does not read or change the user's desktop setup.
    #[cfg(test)]
    pub(crate) unsafe fn fixture_file(path: &Path, blur: f32, dpi: f32) -> Self {
        let mut glass=Self::new(blur,dpi);
        glass.path=path.into(); glass.style=2;
        let (bitmap,sharp,width,height,natural)=glass.load().expect("fixture wallpaper");
        glass.bitmap=bitmap; glass.sharp=sharp; glass.width=width; glass.height=height; glass.natural=natural;
        glass.generation=1;
        glass
    }
    pub unsafe fn set_blur(&mut self, blur: f32, dpi: f32) {
        if (self.blur - blur).abs() < 0.01 && (self.dpi - dpi).abs() < 0.01 {
            return;
        }
        self.blur = blur;
        self.dpi = dpi;
        self.modified = None;
        self.refresh();
    }
    /// Windows writes into our owned UTF-16 buffer. We only read the returned
    /// wallpaper file and two desktop appearance values; no registry writes.
    pub unsafe fn refresh(&mut self) -> bool {
        let mut name = vec![0u16; 32768];
        if SystemParametersInfoW(
            SPI_GETDESKWALLPAPER,
            name.len() as u32,
            name.as_mut_ptr() as _,
            0,
        ) == 0
        {
            return false;
        }
        let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        let path = PathBuf::from(String::from_utf16_lossy(&name[..len]));
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let style = registry_number("WallpaperStyle", 10);
        let tile = registry_number("TileWallpaper", 0) != 0;
        if path == self.path
            && modified == self.modified
            && style == self.style
            && tile == self.tile
        {
            return false;
        }
        self.path = path;
        self.generation += 1;
        self.modified = modified;
        self.style = style;
        self.tile = tile;
        match self.load() {
            Some((bitmap, sharp, w, h, natural)) => {
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                }
                self.bitmap = bitmap;
                if !self.sharp.is_null() {
                    DeleteObject(self.sharp);
                }
                self.sharp = sharp;
                self.width = w;
                self.height = h;
                self.natural = natural;
                true
            }
            None => {
                if !self.sharp.is_null() {
                    DeleteObject(self.sharp);
                    self.sharp = null_mut();
                }
                if !self.bitmap.is_null() {
                    DeleteObject(self.bitmap);
                    self.bitmap = null_mut();
                }
                true
            }
        }
    }
    unsafe fn load(&self) -> Option<(HBITMAP, HBITMAP, i32, i32, [f32; 2])> {
        let source = decode_wallpaper(&self.path)?;
        let natural = [source.width() as f32, source.height() as f32];
        let small = source.thumbnail(1200, 1200).to_rgba8();
        let palette = imageops::thumbnail(&small, 128, 128);
        self.accent.set(crate::accent::dominant(palette.as_raw()));
        if std::env::var_os("LUMEN_TRACE_FRAMES").is_some() {
            eprintln!(
                "native-wallpaper-palette colorref={:06x}",
                self.accent.get()
            );
        }

        drop(source);
        let sigma = (self.blur * self.dpi * small.width() as f32 / natural[0]).clamp(0.1, 40.0);
        let mut blurred = imageops::blur(&small, sigma);
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
        let bitmap = dib(w as i32, h as i32, &bytes)?;
        let mut sharp_bytes = small.into_raw();
        for p in sharp_bytes.chunks_exact_mut(4) {
            p.swap(0, 2);
            p[3] = 255;
        }
        let Some(sharp) = dib(w as i32, h as i32, &sharp_bytes) else {
            DeleteObject(bitmap);
            return None;
        };
        Some((bitmap, sharp, w as i32, h as i32, natural))
    }
    pub unsafe fn draw(&self, dc: HDC, hwnd: HWND, width: i32, height: i32) {
        self.draw_blended(dc, hwnd, width, height, 255);
    }
    pub unsafe fn draw_blended(&self, dc: HDC, hwnd: HWND, width: i32, height: i32, amount: u8) {
        if self.bitmap.is_null() {
            return;
        }
        if width <= 0 || height <= 0 || width as i64 * height as i64 > 32_000_000 {
            return;
        }
        let mut window: RECT = zeroed();
        GetWindowRect(hwnd, &mut window);
        if self.style == 0 || self.style == 6 {
            PatBlt(dc, 0, 0, width, height, BLACKNESS);
        }
        // The caller already caches the finished widget background. A second
        // full-monitor bitmap is unnecessary now that moves freeze that cache.
        if amount == 255 {
            let mut scratch = self.blend_buffer.borrow_mut();
            if scratch.as_ref().is_some_and(|(owner, _)| *owner == hwnd) {
                scratch.take();
            }
            self.draw_layer(dc, hwnd, width, height, self.bitmap, window);
            return;
        }
        self.draw_layer(dc, hwnd, width, height, self.sharp, window);
        if amount == 0 {
            let mut scratch = self.blend_buffer.borrow_mut();
            if scratch.as_ref().is_some_and(|(owner, _)| *owner == hwnd) {
                scratch.take();
            }
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
            self.draw_layer(buffer.dc, hwnd, width, height, self.bitmap, window);
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
        }
    }
    unsafe fn draw_layer(
        &self,
        dc: HDC,
        hwnd: HWND,
        width: i32,
        height: i32,
        bitmap: HBITMAP,
        window: RECT,
    ) {
        if bitmap.is_null() {
            return;
        }
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
        let left = area.left + (if self.tile { 0.0 } else { (aw - dw) / 2.0 }) as i32 - window.left;
        let top = area.top + vertical_offset(ah, dh, self.style, self.tile) as i32 - window.top;
        let source = CreateCompatibleDC(dc);
        let previous = SelectObject(source, bitmap);
        SetStretchBltMode(dc, HALFTONE);
        SetBrushOrgEx(dc, 0, 0, null_mut());
        if self.tile {
            let step_x = (dw as i32).max(1);
            let step_y = (dh as i32).max(1);
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
                        self.width,
                        self.height,
                        SRCCOPY,
                    );
                }
            }
        } else {
            StretchBlt(
                dc,
                left,
                top,
                dw as i32,
                dh as i32,
                source,
                0,
                0,
                self.width,
                self.height,
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
            glass.bitmap = dib(512, 320, &pixels).unwrap();
            glass.sharp = dib(512, 320, &pixels).unwrap();
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
                glass.draw_layer(full.dc, hwnd, full.width, full.height, glass.bitmap, area);
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
        // Fill is centered after applying the file's EXIF orientation.
        assert_eq!(super::vertical_offset(2160., 3840., 10, false), -840.);
        assert_eq!(super::vertical_offset(2160., 3840., 22, false), -840.);
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
}
