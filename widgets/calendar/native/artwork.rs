//! CSS gradient/card materials. Plain pixel generation is independently testable.
use crate::render::{Canvas, Image};
use std::collections::HashMap;
use windows_sys::Win32::Graphics::Gdi::HDC;
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum Material {
    Mist,
    Agenda,
    Soft,
    Ribbon,
    AgendaRed,
    LumenPaper,
    LumenMidnight,
    Almanac,
    Blocks,
    Pop,
    Prism,
    Grove,
    BlocksDate,
    BlocksProgress,
    PopDate,
    PopProgress,
    PrismDate,
    PrismProgress,
    GroveDate,
    GroveProgress,
    Selection(u32),
    Border(u32, u8, u32),
}
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0., 1.);
    std::array::from_fn(|i| a[i] * (1. - t) + b[i] * t)
}
fn rgb(value: u32) -> [f32; 3] {
    [
        (value >> 16) as f32,
        ((value >> 8) & 255) as f32,
        (value & 255) as f32,
    ]
}
fn linear(u: f32, v: f32, w: f32, h: f32, degrees: f32) -> f32 {
    let a = degrees.to_radians();
    let dx = a.sin();
    let dy = -a.cos();
    (((u - 0.5) * w * dx + (v - 0.5) * h * dy) / (w * dx.abs() + h * dy.abs()).max(1.) + 0.5)
        .clamp(0., 1.)
}
fn radial(u: f32, v: f32, c: [f32; 2], stop: f32) -> f32 {
    let x = (u - c[0]) / c[0].max(1. - c[0]).max(0.001);
    let y = (v - c[1]) / c[1].max(1. - c[1]).max(0.001);
    (1. - (x.hypot(y) / std::f32::consts::SQRT_2) / stop).clamp(0., 1.)
}
pub fn sample(material: Material, u: f32, v: f32, w: f32, h: f32) -> [f32; 4] {
    use Material::*;
    let (mut color, alpha) = match material {
        Border(c, a, _) => (
            [
                (c & 255) as f32,
                ((c >> 8) & 255) as f32,
                ((c >> 16) & 255) as f32,
            ],
            a as f32 / 255.,
        ),
        LumenPaper => (rgb(0xe6ecf2), 1.),
        LumenMidnight => (rgb(0x191919), 1.),
        Mist => {
            let base = mix(rgb(0xa6acb2), rgb(0x7b99b9), v);
            (
                mix(base, rgb(0xb8c0c7), radial(u, v, [0.1, 0.1], 0.8) * 0.3),
                1.,
            )
        }
        Agenda => (rgb(0xfafaf8), 1.),
        Soft => (
            mix(rgb(0xf7f7f5), rgb(0xeeeeec), linear(u, v, w, h, 145.)),
            1.,
        ),
        AgendaRed => (mix(rgb(0xf03852), rgb(0xe52b49), v), 1.),
        Ribbon => {
            let bend = (u - 0.5).powi(2) * 0.4;
            let edge =
                ((v - 0.18 - bend) * h).clamp(0., 1.) * ((0.72 + bend - v) * h).clamp(0., 1.);
            (rgb(0xe1e9f1), edge * 0.16)
        }
        Almanac => (
            mix(rgb(0xf2ede2), rgb(0xded8c9), linear(u, v, w, h, 135.)),
            1.,
        ),
        Blocks => (
            mix(rgb(0x141619), rgb(0x08090a), linear(u, v, w, h, 145.)),
            1.,
        ),
        Pop => (rgb(0x090a0c), 1.),
        Prism => (
            mix(rgb(0x101116), rgb(0x342f4b), radial(u, v, [0., 0.], 0.7)),
            1.,
        ),
        Grove => (
            mix(rgb(0x14251d), rgb(0x060d0a), linear(u, v, w, h, 155.)),
            1.,
        ),
        BlocksDate => (rgb(0xff9029), 1.),
        BlocksProgress => (rgb(0x306cf3), 1.),
        PopDate => (rgb(0x3378ff), 1.),
        PopProgress => (rgb(0xffe100), 1.),
        PrismDate => {
            let t = linear(u, v, w, h, 155.);
            let base = if t < 0.55 {
                mix(rgb(0xb3a5ec), rgb(0xf666ba), t / 0.55)
            } else {
                mix(rgb(0xf666ba), rgb(0xffd852), (t - 0.55) / 0.45)
            };
            let pink = mix(base, rgb(0xff057b), radial(u, v, [1., 0.], 0.7));
            (mix(pink, rgb(0xfff5b8), radial(u, v, [0., 1.], 0.65)), 1.)
        }
        PrismProgress => {
            let base = mix(rgb(0x141e24), rgb(0x384448), linear(u, v, w, h, 145.));
            (mix(base, rgb(0x9fbd53), radial(u, v, [0., 1.], 0.7)), 1.)
        }
        GroveDate => (
            mix(rgb(0x172619), rgb(0x45620b), radial(u, v, [0.9, 1.], 0.75)),
            1.,
        ),
        GroveProgress => (
            mix(rgb(0x143b32), rgb(0x071d17), linear(u, v, w, h, 160.)),
            1.,
        ),
        Selection(c) => {
            let t = linear(u, v, w, h, 145.);
            (
                [
                    (c & 255) as f32,
                    ((c >> 8) & 255) as f32,
                    ((c >> 16) & 255) as f32,
                ],
                0.95 * (1. - t) + 0.58 * t,
            )
        }
    };
    if matches!(material, PrismDate | PrismProgress) {
        let top = (1. - v * h / 3.).clamp(0., 1.) * if material == PrismDate { 0.81 } else { 0.4 };
        let bottom =
            (1. - (1. - v) * h / 8.).clamp(0., 1.) * if material == PrismDate { 0.4 } else { 0. };
        color = mix(color, [255.; 3], top.max(bottom));
    }
    [color[0], color[1], color[2], alpha]
}
pub fn pixels(material: Material, width: u32, height: u32, radius: f32, dpi: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let coverage = (0.5
                - crate::compositor::distance(
                    x as f32 + 0.5,
                    y as f32 + 0.5,
                    width as f32,
                    height as f32,
                    radius * dpi,
                    0.,
                ))
            .clamp(0., 1.);
            let c = sample(
                material,
                (x as f32 + 0.5) / width as f32,
                (y as f32 + 0.5) / height as f32,
                width as f32,
                height as f32,
            );
            let coverage = if let Material::Border(_, _, thickness) = material {
                let t = thickness as f32 / 64. * dpi;
                let inner = (0.5
                    - crate::compositor::distance(
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                        width as f32,
                        height as f32,
                        (radius * dpi - t).max(0.),
                        t,
                    ))
                .clamp(0., 1.);
                (coverage - inner).max(0.)
            } else {
                coverage
            };
            let a = c[3] * coverage;
            out.extend([
                (c[0] * a).round() as u8,
                (c[1] * a).round() as u8,
                (c[2] * a).round() as u8,
                (a * 255.).round() as u8,
            ]);
        }
    }
    out
}
#[derive(Default)]
pub struct Artwork {
    cache: HashMap<(Material, u32, u32, u32, u32), Image>,
}
impl Artwork {
    pub unsafe fn border(
        &mut self,
        dc: HDC,
        dpi: f32,
        r: [f32; 4],
        radius: f32,
        color: u32,
        alpha: f32,
        thickness: f32,
    ) {
        self.draw(
            dc,
            dpi,
            r,
            radius,
            Material::Border(
                color,
                (alpha.clamp(0., 1.) * 255.).round() as u8,
                (thickness * 64.).round() as u32,
            ),
            1.,
        );
    }
    pub fn clear(&mut self) {
        self.cache.clear();
    }
    pub unsafe fn draw(
        &mut self,
        dc: HDC,
        dpi: f32,
        r: [f32; 4],
        radius: f32,
        material: Material,
        alpha: f32,
    ) {
        if r[2] <= r[0] || r[3] <= r[1] {
            return;
        }
        let w = ((r[2] - r[0]) * dpi).round().max(1.) as u32;
        let h = ((r[3] - r[1]) * dpi).round().max(1.) as u32;
        let key = (
            material,
            w,
            h,
            (radius * 64.).round() as u32,
            (dpi * 256.).round() as u32,
        );
        if !self.cache.contains_key(&key) {
            if self.cache.len() >= 16 {
                self.cache.clear();
            }
            if let Some(image) = Image::from_rgba(w, h, pixels(material, w, h, radius, dpi)) {
                self.cache.insert(key, image);
            } else {
                return;
            }
        }
        Canvas::new(dc, dpi).image_alpha(self.cache.get(&key).unwrap(), r, alpha);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outer_materials_fill_backing_without_an_inner_rounded_alpha_edge() {
        for dpi in [1., 1.25, 1.5, 2.] {
            let (w, h) = ((240. * dpi) as u32, (280. * dpi) as u32);
            for material in [
                Material::LumenPaper,
                Material::LumenMidnight,
                Material::Almanac,
                Material::Blocks,
                Material::Pop,
                Material::Prism,
                Material::Grove,
                Material::Mist,
                Material::Agenda,
                Material::Soft,
                Material::AgendaRed,
            ] {
                // Zero-radius full-frame material. The compositor supplies the only outer mask.
                let data = pixels(material, w, h, 0., dpi);
                assert!(
                    data.chunks_exact(4).all(|p| p[3] == 255),
                    "{material:?} introduced a nested alpha edge"
                );
                for (x, y) in [
                    (6, w.min(h) / 2),
                    (w / 2, 6),
                    (w - 7, h / 2),
                    (w / 2, h - 7),
                ] {
                    let i = ((y * w + x) * 4) as usize;
                    let expected = sample(
                        material,
                        (x as f32 + 0.5) / w as f32,
                        (y as f32 + 0.5) / h as f32,
                        w as f32,
                        h as f32,
                    );
                    let rgb: [u8; 3] = std::array::from_fn(|i| expected[i].round() as u8);
                    assert_eq!(&data[i..i + 3], rgb.as_slice());
                }
            }
        }
    }
    #[test]
    fn reference_materials_keep_blue_fade_red_sidebar_soft_white_and_alpha_edges() {
        let top = sample(Material::Mist, 0.5, 0., 400., 400.);
        let bottom = sample(Material::Mist, 0.5, 1., 400., 400.);
        assert!(bottom[2] - bottom[0] > top[2] - top[0]);
        let red = sample(Material::AgendaRed, 0.5, 0.5, 160., 400.);
        assert!(red[0] > 220. && red[1] < 60. && red[2] < 90.);
        let white = sample(Material::Soft, 0.5, 0.5, 300., 400.);
        assert!(white[..3].iter().all(|c| *c > 235.));
        assert_eq!(sample(Material::Ribbon, 0.5, 0., 400., 78.)[3], 0.);
        assert!(sample(Material::Ribbon, 0.5, 0.5, 400., 78.)[3] > 0.);
        for m in [
            Material::Mist,
            Material::Agenda,
            Material::AgendaRed,
            Material::Soft,
            Material::Ribbon,
        ] {
            for dpi in [1., 1.25, 2.] {
                let data = pixels(m, (120. * dpi) as u32, (90. * dpi) as u32, 22., dpi);
                assert!(data
                    .chunks_exact(4)
                    .all(|p| p[..3].iter().all(|v| *v <= p[3])));
            }
        }
    }
    #[test]
    fn css_one_and_two_dip_borders_do_not_fill_the_card() {
        for dpi in [1., 1.25, 2.] {
            let w = (80. * dpi) as u32;
            let h = (60. * dpi) as u32;
            let thin = pixels(Material::Border(0xffffff, 255, 64), w, h, 12., dpi);
            let thick = pixels(Material::Border(0xffffff, 255, 128), w, h, 12., dpi);
            let x = w / 2;
            let alpha = |p: &[u8], y: u32| p[((y * w + x) * 4 + 3) as usize] as u32;
            assert_eq!(alpha(&thin, h / 2), 0);
            assert_eq!(alpha(&thick, h / 2), 0);
            let thin_total: u32 = (0..h / 2).map(|y| alpha(&thin, y)).sum();
            let thick_total: u32 = (0..h / 2).map(|y| alpha(&thick, y)).sum();
            assert!(thick_total > thin_total);
        }
    }
    #[test]
    fn css_card_colors_gradients_and_premultiplied_edges_match_reference() {
        assert_eq!(
            sample(Material::BlocksDate, 0.5, 0.5, 100., 100.),
            [255., 144., 41., 1.]
        );
        assert_eq!(
            sample(Material::PopDate, 0.5, 0.5, 100., 100.),
            [51., 120., 255., 1.]
        );
        let pink = sample(Material::PrismDate, 0.98, 0.05, 180., 130.);
        let cream = sample(Material::PrismDate, 0.02, 0.97, 180., 130.);
        assert!(pink[0] > pink[1] + 50.);
        assert!(cream[1] > pink[1] + 60.);
        let green = sample(Material::GroveDate, 0.9, 1., 100., 100.);
        assert!(green[1] > green[0]);
        for m in [
            Material::Almanac,
            Material::PrismDate,
            Material::GroveDate,
            Material::Selection(0x804020),
        ] {
            let p = pixels(m, 160, 100, 24., 1.);
            assert_eq!(p[3], 0);
            assert!(p.chunks_exact(4).all(|p| p[..3].iter().all(|v| *v <= p[3])));
            assert!(p.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
        }
    }
}
