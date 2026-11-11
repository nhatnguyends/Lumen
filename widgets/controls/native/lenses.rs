//! Same optical button material as Music; cached locally by Control Center.
//! Kept package-local so Control Center has no source dependency on another widget.
//! A directional rim, curved reflection and inner shade give depth without a
//! flat white disc. Geometry is rasterized at twice the actual device DPI.
use crate::render::{Canvas, Image};
use std::{cell::RefCell, collections::HashMap};
pub(super) const ENABLED: u16 = 1;
pub(super) const HOVER: u16 = 2;
pub(super) const PRESSED: u16 = 4;
pub(super) const ACTIVE: u16 = 8;
pub(super) const PRIMARY: u16 = 16;
pub(super) const LIGHT: u16 = 32;
pub(super) const CLOUD: u16 = 64;
pub(super) const SOFT: u16 = 128;
pub(super) const LIQUID: u16 = 256;
pub(super) fn material(design: &str) -> u16 {
    match design {
        "cloud" => CLOUD,
        "soft-violet" => SOFT,
        "liquid-pastel" => LIQUID,
        _ => 0,
    }
}
pub(super) fn radius(style: u16, height: f32) -> f32 {
    (if style & CLOUD != 0 {
        height / 2.
    } else if style & SOFT != 0 {
        22.
    } else if style & LIQUID != 0 {
        24.
    } else {
        16.
    })
    .min(height / 2.)
}
#[derive(Default)]
pub(super) struct Lenses(RefCell<HashMap<(u32, u32, u32, u16, u32), Image>>);
impl Lenses {
    pub fn clear(&self) {
        self.0.borrow_mut().clear();
    }
    pub unsafe fn draw(&self, c: &Canvas, dpi: f32, r: [f32; 4], color: u32, style: u16) {
        let diameter = ((r[2] - r[0]) * dpi * 2.).ceil().max(8.) as u32;
        let height = ((r[3] - r[1]) * dpi * 2.).ceil().max(8.) as u32;
        let key = (diameter, height, color, style, (dpi * 1000.).round() as u32);
        let mut cache = self.0.borrow_mut();
        if !cache.contains_key(&key) {
            if let Some(image) = Image::from_rgba(
                diameter,
                height,
                pixels(diameter, height, dpi * 2., color, style),
            ) {
                if cache.len() >= 24 {
                    cache.clear();
                }
                cache.insert(key, image);
            }
        }
        if let Some(image) = cache.get(&key) {
            c.image(image, r);
        }
    }
}
fn pixels(diameter: u32, height: u32, scale: f32, color: u32, style: u16) -> Vec<u8> {
    let mut pixels = vec![0u8; (diameter * height * 4) as usize];
    let center = diameter as f32 / 2.;
    let half_height = height as f32 / 2.;
    let radius = radius(style, height as f32 / scale) * scale;
    let radius = radius.min(half_height - 0.5).min(center - 0.5);
    let cloud = style & CLOUD != 0;
    let soft = style & SOFT != 0;
    let liquid = style & LIQUID != 0;
    let light = style & LIGHT != 0;
    let primary = style & PRIMARY != 0;
    let hover = style & HOVER != 0;
    let pressed = style & PRESSED != 0;
    let enabled = style & ENABLED != 0;
    let active = style & ACTIVE != 0;
    let tint = [
        (color & 255) as f32 / 255.,
        ((color >> 8) & 255) as f32 / 255.,
        ((color >> 16) & 255) as f32 / 255.,
    ];
    for y in 0..height {
        for x in 0..diameter {
            let dx = x as f32 + 0.5 - center;
            let dy = y as f32 + 0.5 - half_height;
            let qx = dx.abs() - (center - radius - 0.5);
            let qy = dy.abs() - (half_height - radius - 0.5);
            let signed = qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - radius;
            let coverage = (0.5 - signed).clamp(0., 1.);
            if coverage <= 0. {
                continue;
            }
            let distance = radius + signed;
            let nx = dx / center.max(1.);
            let ny = dy / half_height.max(1.);
            let radial = (1. + signed / radius.max(1.)).clamp(0., 1.);
            let facing = (-nx * 0.52 - ny * 0.85).max(0.);
            let strength = if pressed {
                0.74
            } else if hover {
                1.18
            } else {
                1.
            };
            let mut rgb = [0f32; 3];
            let mut alpha = 0f32;
            let mut over = |color: [f32; 3], a: f32| {
                let a = a.clamp(0., 1.);
                for i in 0..3 {
                    rgb[i] = color[i] * a + rgb[i] * (1. - a);
                }
                alpha = a + alpha * (1. - a);
            };
            // A luminous translucent body; only the lower shoulder carries
            // shade, so the lens stays bright on both milk and dark glass.
            over(
                [0.94, 0.97, 1.],
                if soft {
                    if light {
                        0.68
                    } else {
                        0.30
                    }
                } else if cloud {
                    if light {
                        0.42
                    } else {
                        0.22
                    }
                } else if liquid {
                    0.075
                } else {
                    if light {
                        0.19
                    } else {
                        0.09
                    }
                },
            );
            over(
                tint,
                if active {
                    if soft {
                        0.48
                    } else {
                        0.18
                    }
                } else if liquid {
                    0.05
                } else if soft {
                    0.045
                } else {
                    0.012
                },
            );
            over(
                [0.; 3],
                (ny * 0.7 + nx * 0.18).max(0.) * if light { 0.045 } else { 0.035 },
            );
            // Broad reflection rolls over the curved upper shoulder, rather than
            // filling the whole circle at the same opacity.
            let shoulder = (-((ny + 0.57) / 0.36).powi(2) - ((nx + 0.19) / 0.94).powi(2)).exp();
            over(
                [1.; 3],
                shoulder
                    * strength
                    * if soft {
                        0.055
                    } else if cloud {
                        0.13
                    } else if liquid {
                        0.30
                    } else if primary {
                        0.25
                    } else {
                        0.19
                    },
            );
            let caustic = (-((radial - 0.79) / 0.16).powi(2)).exp() * ny.max(0.);
            over(
                [0.86, 0.94, 1.],
                caustic
                    * if liquid {
                        0.16
                    } else if soft {
                        0.025
                    } else {
                        0.09
                    },
            );
            // One antialiased optical rim. Strong at the upper-left; a subdued
            // return reflection at the bottom keeps the lens round and substantial.
            let rim =
                (-((distance - (radius - 0.75 * scale)) / (0.64 * scale).max(0.8)).powi(2)).exp();
            over(
                [0.; 3],
                rim * (1. - facing) * if light { 0.055 } else { 0.04 },
            );
            over(
                [0.91, 0.97, 1.],
                rim * (0.10 + 0.46 * facing.powf(1.4)) * strength,
            );
            let opacity = coverage * if enabled { 1. } else { 0.46 };
            let i = ((y * diameter + x) * 4) as usize;
            for ch in 0..3 {
                pixels[i + ch] = (rgb[ch] * opacity * 255.).round().clamp(0., 255.) as u8;
            }
            pixels[i + 3] = (alpha * opacity * 255.).round().clamp(0., 255.) as u8;
        }
    }
    pixels
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn themes_have_distinct_silhouettes_and_reflections_with_identical_color() {
        let styles = [0, CLOUD, SOFT, LIQUID];
        let images: Vec<_> = styles
            .iter()
            .map(|s| pixels(180, 72, 1., 0x887766, ENABLED | LIGHT | s))
            .collect();
        for i in 0..images.len() {
            for j in i + 1..images.len() {
                assert_ne!(images[i], images[j]);
            }
        }
        assert_eq!(radius(CLOUD, 72.), 36.);
        assert_eq!(radius(SOFT, 72.), 22.);
        assert_eq!(radius(LIQUID, 72.), 24.);
        assert_eq!(radius(0, 72.), 16.);
        // The same corner pixel lies inside the soft surface but outside
        // the rounded milk-glass pill. This cannot pass on color alone.
        assert!(images[2][(8 * 180 + 8) * 4 + 3] > 0);
        assert_eq!(images[1][(8 * 180 + 8) * 4 + 3], 0);
    }
    #[test]
    fn glass_lens_has_directional_volume_and_correct_transparency() {
        let p = pixels(80, 80, 2., 0x705040, ENABLED | PRIMARY);
        let at = |x: usize, y: usize, ch: usize| p[(y * 80 + x) * 4 + ch];
        assert_eq!(at(0, 0, 3), 0);
        assert!(
            at(13, 13, 0) > at(66, 66, 0) + 15,
            "light must be directional, not a flat ring"
        );
        assert!(
            at(40, 40, 3) > 0 && at(40, 40, 3) < 110,
            "keep the belly translucent"
        );
        assert!(p.chunks_exact(4).all(|p| p[..3].iter().all(|c| *c <= p[3])));
        assert!(p.chunks_exact(4).any(|p| p[3] > 100));
    }
}
