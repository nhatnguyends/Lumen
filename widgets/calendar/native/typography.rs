//! Theme-local typography; no changes to Dock's fixed label font renderer.
//! Cache owned rasters at final device size. Never stretch text to fit a cell.
use crate::{
    compositor::Buffer,
    model::wide,
    render::{recover_icon_alpha, Canvas, Image},
};
use std::collections::HashMap;
use windows_sys::Win32::Graphics::Gdi::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Sans,
    Serif,
}
#[derive(Clone, Copy)]
pub struct Style {
    pub family: Family,
    pub size: f32,
    pub weight: u16,
    pub color: u32,
    pub spacing: f32,
    pub alpha: f32,
    glow: Option<[f32; 4]>,
}
impl Style {
    pub fn new(size: f32, weight: u16, color: u32) -> Self {
        Self {
            family: Family::Sans,
            size,
            weight,
            color,
            spacing: 0.,
            alpha: 1.,
            glow: None,
        }
    }
    pub fn serif(mut self) -> Self {
        self.family = Family::Serif;
        self
    }
    pub fn spacing(mut self, value: f32) -> Self {
        self.spacing = value;
        self
    }
    pub fn alpha(mut self, value: f32) -> Self {
        self.alpha = value;
        self
    }
    pub fn glow(mut self, clip: [f32; 4]) -> Self {
        self.glow = Some(clip);
        self
    }
}
#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    text: String,
    family: Family,
    size: i32,
    weight: u16,
    color: u32,
    spacing: i32,
    width: i32,
    height: i32,
    align: i32,
    glow: bool,
    dpi: u32,
}
struct Glyph {
    image: Image,
    halo: Option<(u32, Image)>,
}
#[derive(Default)]
pub struct Typography {
    cache: HashMap<Key, Glyph>,
    metrics: HashMap<(String, Family, i32, u16, i32), f32>,
}
impl Typography {
    pub fn clear(&mut self) {
        self.cache.clear();
        self.metrics.clear();
    }
    pub unsafe fn measure(&mut self, dpi: f32, text: &str, style: Style) -> f32 {
        let key = (
            text.into(),
            style.family,
            (style.size * dpi).round().max(1.) as i32,
            style.weight,
            (style.spacing * dpi).round() as i32,
        );
        if let Some(width) = self.metrics.get(&key) {
            return *width / dpi;
        }
        let font = make_font(style.family, key.2, style.weight);
        if font.is_null() {
            return text.len() as f32 * style.size * 0.55;
        }
        let dc = CreateCompatibleDC(std::ptr::null_mut());
        if dc.is_null() {
            DeleteObject(font);
            return text.len() as f32 * style.size * 0.55;
        }
        let old = SelectObject(dc, font);
        SetTextCharacterExtra(dc, key.4);
        let value = wide(text);
        let mut size = std::mem::zeroed();
        GetTextExtentPoint32W(dc, value.as_ptr(), value.len() as i32 - 1, &mut size);
        SelectObject(dc, old);
        DeleteObject(font);
        DeleteDC(dc);
        if self.metrics.len() >= 256 {
            self.metrics.clear();
        }
        self.metrics.insert(key, size.cx as f32);
        size.cx as f32 / dpi
    }
    pub unsafe fn draw(
        &mut self,
        dc: HDC,
        dpi: f32,
        text: &str,
        r: [f32; 4],
        style: Style,
        align: i32,
    ) {
        if text.is_empty() || r[2] <= r[0] || r[3] <= r[1] {
            return;
        }
        // Short glyph runs (dates) use a stable tight raster through resizing.
        let natural = if text.chars().count() <= 3 {
            self.measure(dpi, text, style) + 4.
        } else {
            r[2] - r[0]
        };
        let width = if text.chars().count() <= 3 {
            (r[2] - r[0]).min(natural)
        } else {
            r[2] - r[0]
        };
        let height = (r[3] - r[1]).min(style.size * 1.6 + 4.);
        let key = Key {
            text: text.into(),
            family: style.family,
            size: (style.size * dpi).round().max(1.) as i32,
            weight: style.weight,
            color: style.color,
            spacing: (style.spacing * dpi).round() as i32,
            width: (width * dpi).round().max(1.) as i32,
            height: (height * dpi).round().max(1.) as i32,
            align,
            glow: style.glow.is_some(),
            dpi: (dpi * 256.).round() as u32,
        };
        if !self.cache.contains_key(&key) {
            if self.cache.len() >= 256 {
                self.cache.clear();
            }
            if let Some(image) = raster(&key) {
                self.cache.insert(key.clone(), image);
            } else {
                return;
            }
        }
        let glyph = self.cache.get(&key).unwrap();
        let image = &glyph.image;
        let [pw, ph] = image.pixel_size();
        let width = pw as f32 / dpi;
        let height = ph as f32 / dpi;
        let x = match align {
            1 => (r[0] + r[2] - width) * 0.5,
            2 => r[2] - width,
            _ => r[0],
        };
        let y = (r[1] + r[3] - height) * 0.5;
        if let Some((pad, halo)) = &glyph.halo {
            let p = *pad as f32 / dpi;
            let c = Canvas::new(dc, dpi);
            let target = [x - p, y - p + 1., x + width + p, y + height + p + 1.];
            if let Some(clip) = style.glow {
                c.clipped_image_alpha(halo, target, clip, style.alpha);
            } else {
                c.image_alpha(halo, target, style.alpha);
            }
        }
        Canvas::new(dc, dpi).image_alpha(
            image,
            [
                (x * dpi).round() / dpi,
                (y * dpi).round() / dpi,
                (x * dpi).round() / dpi + width,
                (y * dpi).round() / dpi + height,
            ],
            style.alpha,
        );
    }
    pub unsafe fn vertical(&mut self, dc: HDC, dpi: f32, text: &str, r: [f32; 4], style: Style) {
        let width = r[3] - r[1];
        let height = r[2] - r[0];
        let key = Key {
            text: text.into(),
            family: style.family,
            size: (style.size * dpi).round().max(1.) as i32,
            weight: style.weight,
            color: style.color,
            spacing: (style.spacing * dpi).round() as i32,
            width: (width * dpi).round().max(1.) as i32,
            height: (height * dpi).round().max(1.) as i32,
            align: 1,
            glow: false,
            dpi: (dpi * 256.).round() as u32,
        };
        if !self.cache.contains_key(&key) {
            if self.cache.len() >= 256 {
                self.cache.clear();
            }
            if let Some(image) = raster(&key) {
                self.cache.insert(key.clone(), image);
            } else {
                return;
            }
        }
        let image = &self.cache.get(&key).unwrap().image;
        let [pw, ph] = image.pixel_size();
        let width = pw as f32 / dpi;
        let height = ph as f32 / dpi;
        let x = (((r[0] + r[2]) * 0.5) * dpi).round() / dpi;
        let y = (((r[1] + r[3]) * 0.5) * dpi).round() / dpi;
        let c = Canvas::new(dc, dpi);
        c.transform_about([x, y], 1., 90.);
        c.image_alpha(
            image,
            [
                x - width / 2.,
                y - height / 2.,
                x + width / 2.,
                y + height / 2.,
            ],
            style.alpha,
        );
    }
}
unsafe fn raster(key: &Key) -> Option<Glyph> {
    let font = make_font(key.family, key.size, key.weight);
    if font.is_null() {
        return None;
    }
    let mut mattes = vec![];
    for white in [false, true] {
        let Some(buffer) = Buffer::plain(key.width, key.height, 1.) else {
            DeleteObject(font);
            return None;
        };
        PatBlt(
            buffer.dc,
            0,
            0,
            key.width,
            key.height,
            if white { WHITENESS } else { BLACKNESS },
        );
        let old = SelectObject(buffer.dc, font);
        SetBkMode(buffer.dc, TRANSPARENT as i32);
        SetTextColor(buffer.dc, key.color);
        SetTextCharacterExtra(buffer.dc, key.spacing);
        let mut rect = windows_sys::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: key.width,
            bottom: key.height,
        };
        let mut text = wide(&key.text);
        if key.text.chars().count() <= 3 {
            // DrawText's ellipsis fitting disagrees with tight negative tracking.
            // Dates/progress values must stay complete, not become "1…"/"3…%".
            let mut extent = std::mem::zeroed();
            GetTextExtentPoint32W(buffer.dc, text.as_ptr(), text.len() as i32 - 1, &mut extent);
            let mut metrics: TEXTMETRICW = std::mem::zeroed();
            GetTextMetricsW(buffer.dc, &mut metrics);
            let x = match key.align {
                1 => (key.width - extent.cx) / 2,
                2 => key.width - extent.cx - 2,
                _ => 2,
            }
            .max(0);
            TextOutW(
                buffer.dc,
                x,
                (key.height - metrics.tmHeight) / 2,
                text.as_ptr(),
                text.len() as i32 - 1,
            );
        } else {
            DrawTextW(
                buffer.dc,
                text.as_mut_ptr(),
                text.len() as i32 - 1,
                &mut rect,
                DT_SINGLELINE
                    | DT_VCENTER
                    | DT_END_ELLIPSIS
                    | DT_NOPREFIX
                    | match key.align {
                        1 => DT_CENTER,
                        2 => DT_RIGHT,
                        _ => DT_LEFT,
                    },
            );
        }
        SelectObject(buffer.dc, old);
        mattes.push(buffer.snapshot());
    }
    DeleteObject(font);
    let pixels = recover_icon_alpha(&mattes[0], &mattes[1]);
    let halo = if key.glow {
        let sigma = (key.dpi as f32 / 256. * 7.5).max(1.);
        let pad = (sigma * 3.).ceil() as u32;
        let w = key.width as u32;
        let h = key.height as u32;
        let mut mask = image::RgbaImage::new(w + pad * 2, h + pad * 2);
        for y in 0..h {
            for x in 0..w {
                let a = pixels[((y * w + x) * 4 + 3) as usize];
                mask.put_pixel(x + pad, y + pad, image::Rgba([a, a, a, a]));
            }
        }
        let mut blurred = image::imageops::fast_blur(&mask, sigma).into_raw();
        for p in blurred.chunks_exact_mut(4) {
            let a = (p[3] as f32 * 0.467).round() as u8;
            p.copy_from_slice(&[a, a, a, a]);
        }
        Image::from_rgba(w + pad * 2, h + pad * 2, blurred).map(|image| (pad, image))
    } else {
        None
    };
    Some(Glyph {
        image: Image::from_rgba(key.width as u32, key.height as u32, pixels)?,
        halo,
    })
}
unsafe fn make_font(family: Family, size: i32, weight: u16) -> HFONT {
    CreateFontW(
        -size,
        0,
        0,
        0,
        weight as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32,
        CLIP_DEFAULT_PRECIS as u32,
        ANTIALIASED_QUALITY as u32,
        DEFAULT_PITCH as u32,
        wide(if family == Family::Serif {
            "Georgia"
        } else {
            "Segoe UI Variable Text"
        })
        .as_ptr(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn themed_typefaces_keep_premultiplied_alpha_and_drop_before_renderer() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = crate::render::Runtime::start().unwrap();
        unsafe {
            for dpi in [1., 1.25, 1.5, 2.] {
                let mut buffer =
                    Buffer::plain((240. * dpi) as i32, (180. * dpi) as i32, dpi).unwrap();
                buffer.background_alpha(0);
                let batch = crate::render::AlphaBatch::new(buffer.dc, dpi);
                let mut text = Typography::default();
                text.draw(
                    buffer.dc,
                    dpi,
                    "09",
                    [12., 8., 140., 80.],
                    Style::new(56., 350, 0).serif().spacing(-6.),
                    0,
                );
                text.draw(
                    buffer.dc,
                    dpi,
                    "09",
                    [12., 90., 140., 160.],
                    Style::new(48., 250, 0xffffff)
                        .spacing(-2.)
                        .glow([0., 82., 240., 180.]),
                    0,
                );
                text.vertical(
                    buffer.dc,
                    dpi,
                    "November 2026",
                    [190., 12., 210., 168.],
                    Style::new(12., 400, 0xffffff).spacing(1.),
                );
                drop(batch);
                let pixels = buffer.snapshot();
                assert!(pixels
                    .chunks_exact(4)
                    .all(|p| p[..3].iter().all(|v| *v <= p[3].saturating_add(1))));
                assert!(pixels
                    .chunks_exact(4)
                    .any(|p| p[3] > 200 && p[..3] == [0, 0, 0]));
                assert!(pixels.chunks_exact(4).any(|p| p[0] > 100));
                text.clear();
                assert!(text.cache.is_empty() && text.metrics.is_empty());
            }
        }
    }
}
