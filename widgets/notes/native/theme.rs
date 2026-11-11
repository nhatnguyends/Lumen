//! Full-bleed content materials; the compositor owns the only outer mask/bevel.
//! Texture generation runs only when the backdrop cache changes.
use super::*;
pub fn solid_material(design: &str) -> bool {
    matches!(design, "notebook" | "index" | "honey")
}
fn pixels(size: [i32; 2], dpi: f32, design: &str, content_top: f32) -> Vec<u8> {
    let [width, height] = size;
    if width <= 0 || height <= 0 || dpi <= 0. {
        return Vec::new();
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            let u = x as f32 / width as f32;
            let v = y as f32 / height as f32;
            let t = (u * 0.2 + v * 0.8).clamp(0., 1.);
            let (a, b) = match design {
                "notebook" => ([251., 247., 236.], [238., 229., 207.]),
                "index" => ([24., 52., 47.], [12., 31., 29.]),
                _ => ([255., 183., 40.], [255., 216., 100.]),
            };
            let mut color: [f32; 3] = std::array::from_fn(|i| a[i] * (1. - t) + b[i] * t);
            if design == "honey" {
                // A cover-to-paper transition spans the entire widget, rather than
                // drawing a smaller rounded sheet inside its rounded chrome.
                let blend = ((y as f32 / dpi - (content_top - 9.)) / 6.).clamp(0., 1.);
                let blend = blend * blend * (3. - 2. * blend);
                let paper = [249. - v * 6., 243. - v * 8., 231. - v * 12.];
                for i in 0..3 {
                    color[i] = color[i] * (1. - blend) + paper[i] * blend;
                }
            }
            if design == "notebook" {
                let seed =
                    (x as u32 / 2).wrapping_mul(73856093) ^ (y as u32 / 2).wrapping_mul(19349663);
                let grain = ((seed & 3) as f32 - 1.5) * 0.35;
                for c in &mut color {
                    *c += grain;
                }
                // Binding is part of the full-height material at the outer edge,
                // with a soft fold. It is not another inset rounded rectangle.
                let binding = (1. - (x as f32 / dpi - 9.) / 3.).clamp(0., 1.);
                let cover = [117., 77., 49.];
                for i in 0..3 {
                    color[i] = color[i] * (1. - binding) + cover[i] * binding;
                }
            }
            pixels.extend(color.map(|c| c.round().clamp(0., 255.) as u8));
            pixels.push(255);
        }
    }
    pixels
}
pub unsafe fn backdrop(dc: HDC, size: [i32; 2], dpi: f32, design: &str, content_top: f32) {
    let rgba = pixels(size, dpi, design, content_top);
    if rgba.is_empty() {
        return;
    }
    if let Some(image) = crate::render::Image::from_rgba(size[0] as u32, size[1] as u32, rgba) {
        Canvas::new(dc, dpi).image(&image, [0., 0., size[0] as f32 / dpi, size[1] as f32 / dpi]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn theme_materials_fill_edges_without_an_inset_sheet_boundary() {
        for dpi in [1., 1.5] {
            for [w, h] in [[240, 240], [420, 650], [880, 540]] {
                let size = [(w as f32 * dpi) as i32, (h as f32 * dpi) as i32];
                let top = if w >= 420 { 96. } else { 66. };
                for design in ["notebook", "index", "honey"] {
                    let rgba = pixels(size, dpi, design, top);
                    assert_eq!(rgba.len(), size[0] as usize * size[1] as usize * 4);
                    assert!(
                        rgba.chunks_exact(4).all(|p| p[3] == 255),
                        "Outer masking belongs to the compositor"
                    );
                    let y = ((top + 36.) * dpi) as usize;
                    let pixel = |x: f32| {
                        let i = (y * size[0] as usize + (x * dpi) as usize) * 4;
                        &rgba[i..i + 3]
                    };
                    // These were the old inner panel edges. They must no longer
                    // introduce a material seam in the continuous background.
                    for x in [12., 22., w as f32 - 12., w as f32 - 10.] {
                        if design == "notebook" && x < 22. {
                            continue;
                        }
                        assert!(
                            pixel(x)
                                .iter()
                                .zip(pixel(x + 1.))
                                .all(|(a, b)| a.abs_diff(*b) <= 2),
                            "{design}: material seam at {x}"
                        );
                    }
                    if design == "honey" {
                        assert_eq!(pixel(0.), pixel(w as f32 - 1.));
                    }
                }
            }
        }
    }
}
