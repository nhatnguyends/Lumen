//! Shared glass materials. Persisted keys carry either a gradient identity or
//! a precise custom sRGB color; the wallpaper/content themes stay independent.
use crate::render::{Canvas, Image};
use std::{cell::RefCell, collections::HashMap, rc::Rc};
const fn cref(rgb: u32) -> u32 {
    ((rgb >> 16) & 255) | (rgb & 0xff00) | ((rgb & 255) << 16)
}
pub const PRESETS: [(&str, &str, u32); 12] = [
    ("gradient:afterglow", "Afterglow", cref(0xc6b9cb)),
    ("gradient:dawn", "Dawn", cref(0xb3a0c8)),
    ("gradient:dusk", "Dusk", cref(0x6d4d88)),
    ("gradient:dust", "Dust", cref(0xb39c9e)),
    ("gradient:aurora", "Aurora", cref(0x376c73)),
    ("gradient:ocean", "Ocean", cref(0x286e9b)),
    ("gradient:rose", "Rose", cref(0xc68ba5)),
    ("gradient:ember", "Ember", cref(0xa75a4d)),
    ("gradient:mist", "Mist", cref(0xc2d5df)),
    ("gradient:midnight", "Midnight", cref(0x29304d)),
    (
        "wallpaper-auto",
        "Wallpaper · Auto",
        crate::accent::FALLBACK,
    ),
    ("custom", "Custom color…", cref(0x67798e)),
];
const LEGACY: [(&str, u32); 9] = [
    ("obsidian", 0x27221f),
    ("porcelain", 0xe8e4df),
    ("aurora", 0x554336),
    ("sunset", 0x504372),
    ("ocean", 0x3a2e1c),
    ("neon", 0x503025),
    ("ember", 0x25395b),
    ("candy", 0x73597c),
    ("cobalt", 0x563621),
];
pub fn custom_key(color: u32) -> String {
    format!(
        "custom:#{:02X}{:02X}{:02X}",
        color & 255,
        (color >> 8) & 255,
        (color >> 16) & 255
    )
}
pub fn custom_color(key: &str) -> Option<u32> {
    let hex = key.strip_prefix("custom:#")?;
    if hex.len() != 6 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok().map(cref)
}
pub fn is_gradient(key: &str) -> bool {
    key.starts_with("gradient:") && PRESETS.iter().any(|p| p.0 == key)
}
pub fn managed(key: &str) -> bool {
    is_gradient(key) || custom_color(key).is_some()
}
pub fn valid(key: &str) -> bool {
    key == crate::accent::THEME || managed(key) || LEGACY.iter().any(|p| p.0 == key)
}
pub fn normalize(key: &str) -> Option<String> {
    if let Some(color) = custom_color(key) {
        return Some(custom_key(color));
    }
    if let Some((_, color)) = LEGACY.iter().find(|p| p.0 == key) {
        return Some(custom_key(*color));
    }
    (key == crate::accent::THEME || is_gradient(key)).then(|| key.into())
}
pub fn color(key: &str, wallpaper: u32) -> u32 {
    custom_color(key)
        .or_else(|| LEGACY.iter().find(|p| p.0 == key).map(|p| p.1))
        .or_else(|| {
            PRESETS
                .iter()
                .find(|p| p.0 == key && key != crate::accent::THEME)
                .map(|p| p.2)
        })
        .unwrap_or(wallpaper)
}
fn rgb(hex: u32) -> [f32; 3] {
    [
        ((hex >> 16) & 255) as f32,
        ((hex >> 8) & 255) as f32,
        (hex & 255) as f32,
    ]
}
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0., 1.);
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}
fn radial(u: f32, v: f32, cx: f32, cy: f32, stop: f32) -> f32 {
    let rx = cx.max(1. - cx) * std::f32::consts::SQRT_2;
    let ry = cy.max(1. - cy) * std::f32::consts::SQRT_2;
    (1. - (((u - cx) / rx).powi(2) + ((v - cy) / ry).powi(2)).sqrt() / stop).clamp(0., 1.)
}
fn linear(u: f32, v: f32, aspect: f32, angle: f32) -> f32 {
    let a = angle.to_radians();
    let dx = a.sin();
    let dy = -a.cos();
    (((u - 0.5) * aspect * dx + (v - 0.5) * dy) / (aspect * dx.abs() + dy.abs()) + 0.5)
        .clamp(0., 1.)
}
pub fn sample(key: &str, u: f32, v: f32, aspect: f32) -> [u8; 3] {
    let p = linear(u, v, aspect, 165.);
    let c = match key.strip_prefix("gradient:").unwrap_or(key) {
        "afterglow" => {
            let base = if p < 0.43 {
                mix(rgb(0x929fc9), rgb(0xc6b9cb), (p - 0.02) / 0.41)
            } else {
                mix(rgb(0xc6b9cb), rgb(0xffb97f), (p - 0.43) / 0.37)
            };
            let pale = mix(base, rgb(0xffd4a0), radial(u, v, 0.95, 0.88, 0.60));
            mix(pale, rgb(0xff713b), radial(u, v, 0.20, 1., 0.62))
        }
        "dawn" => {
            let c = mix(rgb(0x627494), rgb(0xf5bdaa), p);
            mix(c, rgb(0xffe3ad), radial(u, v, 0.82, 0.95, 0.67))
        }
        "dusk" => {
            let c = mix(rgb(0x222b55), rgb(0xb16d91), p);
            mix(c, rgb(0xf2aa88), radial(u, v, 0.12, 0.95, 0.62))
        }
        "dust" => {
            let c = mix(rgb(0x7e879b), rgb(0xd4b6a7), p);
            mix(c, rgb(0xc593a0), radial(u, v, 0.2, 0.9, 0.75))
        }
        "aurora" => {
            let c = mix(rgb(0x202a50), rgb(0x357f79), p);
            mix(c, rgb(0xa99bce), radial(u, v, 0.08, 0.08, 0.8))
        }
        "ocean" => {
            let c = mix(rgb(0x253757), rgb(0x448ca3), p);
            mix(c, rgb(0x83c7bd), radial(u, v, 0.08, 0.15, 0.8))
        }
        "rose" => {
            let c = mix(rgb(0x8d7fa7), rgb(0xe1a2ac), p);
            mix(c, rgb(0xf6d5be), radial(u, v, 0.9, 0.85, 0.7))
        }
        "ember" => {
            let c = mix(rgb(0x39314e), rgb(0xc36c4a), p);
            mix(c, rgb(0xf1bb75), radial(u, v, 0.15, 1., 0.75))
        }
        "mist" => {
            let c = mix(rgb(0xa6bdd4), rgb(0xe6dfdd), p);
            mix(c, rgb(0xc8bdda), radial(u, v, 0.9, 0.1, 0.8))
        }
        "midnight" => {
            let c = mix(rgb(0x151e32), rgb(0x34324d), p);
            mix(c, rgb(0x52667f), radial(u, v, 0.1, 0.1, 0.8))
        }
        _ => {
            let c = color(key, crate::accent::FALLBACK);
            [
                (c & 255) as f32,
                ((c >> 8) & 255) as f32,
                ((c >> 16) & 255) as f32,
            ]
        }
    };
    c.map(|v| v.round().clamp(0., 255.) as u8)
}
// Bounded GDI+ texture cache; cleared before GdiplusShutdown in Runtime::drop.
thread_local! {static TEXTURES:RefCell<HashMap<(String,u32,u32,u32,u32),Rc<Image>>>=RefCell::new(HashMap::new());}
pub fn clear_cache() {
    TEXTURES.with(|cache| cache.borrow_mut().clear());
}
fn texture(key: &str, w: u32, h: u32, aspect: f32, radius: f32) -> Option<Rc<Image>> {
    let cache_key = (
        key.into(),
        w,
        h,
        (aspect * 64.).round() as u32,
        (radius * 16.).round() as u32,
    );
    TEXTURES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(existing) = cache.get(&cache_key) {
            return Some(existing.clone());
        }
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let c = sample(
                    key,
                    (x as f32 + 0.5) / w as f32,
                    (y as f32 + 0.5) / h as f32,
                    aspect,
                );
                let alpha = if radius > 0. {
                    ((0.5
                        - crate::compositor::distance(
                            x as f32 + 0.5,
                            y as f32 + 0.5,
                            w as f32,
                            h as f32,
                            radius,
                            0.,
                        ))
                    .clamp(0., 1.)
                        * 255.)
                        .round() as u8
                } else {
                    255
                };
                pixels.extend_from_slice(&[
                    ((c[0] as u32 * alpha as u32 + 127) / 255) as u8,
                    ((c[1] as u32 * alpha as u32 + 127) / 255) as u8,
                    ((c[2] as u32 * alpha as u32 + 127) / 255) as u8,
                    alpha,
                ]);
            }
        }
        let image = Rc::new(Image::from_rgba(w, h, pixels)?);
        if cache.len() >= 16 {
            cache.clear();
        }
        cache.insert(cache_key, image.clone());
        Some(image)
    })
}
pub fn paint(c: &Canvas, pane: [f32; 4], key: &str, opacity: f32) {
    if !managed(key) || opacity <= 0. {
        return;
    }
    let aspect = ((pane[2] - pane[0]) / (pane[3] - pane[1]).max(1.)).clamp(0.1, 10.);
    if let Some(image) = texture(key, 160, 160, aspect, 0.) {
        c.image_alpha(&image, pane, opacity.clamp(0., 1.));
    }
}
pub fn swatch(c: &Canvas, r: [f32; 4], radius: f32, key: &str, dpi: f32) {
    let w = ((r[2] - r[0]) * dpi).round().clamp(1., 512.) as u32;
    let h = ((r[3] - r[1]) * dpi).round().clamp(1., 256.) as u32;
    if let Some(image) = texture(
        key,
        w,
        h,
        (r[2] - r[0]) / (r[3] - r[1]).max(1.),
        radius * dpi,
    ) {
        c.image(&image, r);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_solids_migrate_without_changing_color_and_bad_values_are_rejected() {
        for (key, c) in LEGACY {
            let migrated = normalize(key).unwrap();
            assert_eq!(custom_color(&migrated), Some(c));
            assert_eq!(normalize(&migrated), Some(migrated));
        }
        for key in [
            "custom",
            "custom:#12345",
            "custom:#GG0000",
            "gradient:unknown",
            "",
        ] {
            assert!(!valid(key));
        }
        assert_eq!(custom_key(cref(0x12abef)), "custom:#12ABEF");
    }
    #[test]
    fn all_gradients_have_real_color_variation_and_afterglow_keeps_its_warm_footer() {
        for (key, _, _) in PRESETS.iter().filter(|p| is_gradient(p.0)) {
            assert!(valid(key));
            assert_ne!(sample(key, 0.2, 0.05, 1.), sample(key, 0.2, 0.95, 1.));
        }
        let sky = sample("gradient:afterglow", 0.1, 0.03, 1.);
        let warm = sample("gradient:afterglow", 0.2, 0.95, 1.);
        assert!(sky[2] > sky[0]);
        assert!(warm[0] > warm[2] + 30);
    }
}
