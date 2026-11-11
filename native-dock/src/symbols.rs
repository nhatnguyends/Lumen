//! Original Lumen vectors, rasterized at twice the device resolution on demand.
use crate::render::{Canvas, Image};
use std::{cell::RefCell, collections::HashMap};
use windows_sys::Win32::Graphics::Gdi::HDC;
const VECTORS: [&str; 7] = [
    include_str!("../assets/toolbar/0.svg"),
    include_str!("../assets/toolbar/1.svg"),
    include_str!("../assets/toolbar/2.svg"),
    include_str!("../assets/toolbar/3.svg"),
    include_str!("../assets/toolbar/4.svg"),
    include_str!("../assets/toolbar/5.svg"),
    include_str!("../assets/toolbar/6.svg"),
];
#[derive(Default)]
pub struct Symbols(RefCell<HashMap<(usize, u32, u32), Image>>);
impl Symbols {
    pub unsafe fn draw(
        &self,
        dc: HDC,
        dpi: f32,
        kind: usize,
        x: f32,
        y: f32,
        size: f32,
        color: u32,
    ) {
        self.draw_rotated(dc, dpi, kind, x, y, size, color, 0.);
    }
    pub unsafe fn draw_rotated(
        &self,
        dc: HDC,
        dpi: f32,
        kind: usize,
        x: f32,
        y: f32,
        size: f32,
        color: u32,
        angle: f32,
    ) {
        self.draw_rotated_alpha(dc, dpi, kind, x, y, size, color, angle, 1.);
    }
    pub unsafe fn draw_rotated_alpha(
        &self,
        dc: HDC,
        dpi: f32,
        kind: usize,
        x: f32,
        y: f32,
        size: f32,
        color: u32,
        angle: f32,
        opacity: f32,
    ) {
        let kind = kind.min(VECTORS.len() - 1);
        let px = (size * dpi * 2.).ceil().max(1.) as u32;
        let key = (kind, color, px);
        let mut cache = self.0.borrow_mut();
        if let std::collections::hash_map::Entry::Vacant(entry) = cache.entry(key) {
            if let Some(image) =
                raster(kind, color, px).and_then(|data| Image::from_rgba(px, px, data))
            {
                entry.insert(image);
            }
        }
        if let Some(image) = cache.get(&key) {
            let canvas = Canvas::new(dc, dpi);
            canvas.transform_about([x + size / 2., y + size / 2.], 1., angle);
            canvas.image_alpha(image, [x, y, x + size, y + size], opacity);
        }
    }
}
fn raster(kind: usize, color: u32, px: u32) -> Option<Vec<u8>> {
    let css = format!(
        "#{:02x}{:02x}{:02x}",
        color & 255,
        (color >> 8) & 255,
        (color >> 16) & 255
    );
    let svg = VECTORS[kind].replace("currentColor", &css);
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(px, px)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(px as f32 / 24., px as f32 / 24.),
        &mut pixmap.as_mut(),
    );
    Some(pixmap.data().to_vec())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_original_vector_has_smooth_transparent_edges() {
        for i in 0..7 {
            let pixels = raster(i, 0xffffff, 64).unwrap();
            assert!(pixels.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
            assert!(pixels.chunks_exact(4).any(|p| p[3] == 0));
            assert!(pixels.chunks_exact(4).any(|p| p[3] > 200));
        }
    }
}
