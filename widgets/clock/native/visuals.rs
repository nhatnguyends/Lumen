//! Theme paint translated from src/renderer/src/clock.css (legacy source).
//! Textures contain no window mask; resize rebuilds the physical chrome separately.
use super::model::Config;
fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0., 1.);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}
fn rgb(hex: u32) -> [f32; 3] {
    [
        ((hex >> 16) & 255) as f32,
        ((hex >> 8) & 255) as f32,
        (hex & 255) as f32,
    ]
}
fn linear(u: f32, v: f32, aspect: f32, angle: f32) -> f32 {
    let a = angle.to_radians();
    let dx = a.sin();
    let dy = -a.cos();
    ((u - 0.5) * aspect * dx + (v - 0.5) * dy) / (aspect * dx.abs() + dy.abs()) + 0.5
}
fn radial(u: f32, v: f32, cx: f32, cy: f32, stop: f32) -> f32 {
    // CSS ellipse farthest-corner extent, expressed in normalized box coordinates.
    let rx = cx.max(1. - cx) * std::f32::consts::SQRT_2;
    let ry = cy.max(1. - cy) * std::f32::consts::SQRT_2;
    (1. - (((u - cx) / rx).powi(2) + ((v - cy) / ry).powi(2)).sqrt() / stop).clamp(0., 1.)
}
pub(super) fn sample(theme: &str, u: f32, v: f32, aspect: f32) -> [u8; 3] {
    let c = match theme {
        "sunset" => {
            let p = linear(u, v, aspect, 165.);
            let base = if p < 0.43 {
                mix(rgb(0x929fc9), rgb(0xc6b9cb), (p - 0.02) / 0.41)
            } else {
                mix(rgb(0xc6b9cb), rgb(0xffb97f), (p - 0.43) / 0.37)
            };
            let pale = mix(base, rgb(0xffd4a0), radial(u, v, 0.95, 0.88, 0.60));
            mix(pale, rgb(0xff713b), radial(u, v, 0.20, 1., 0.62))
        }
        "paper" => rgb(0xf3f2ed),
        "modular" => rgb(0x080909),
        "neon" => mix(rgb(0x101310), rgb(0x202a1a), radial(u, v, 0.5, 0.45, 0.65)),
        "ocean" => mix(rgb(0x263642), rgb(0x10181f), linear(u, v, aspect, 150.)),
        "rose" => mix(rgb(0x101c13), rgb(0x334d06), radial(u, v, 0.20, 1., 0.80)),
        "aurora" => mix(rgb(0x171326), rgb(0x492c74), radial(u, v, 0.10, 0., 0.85)),
        "terminal" => rgb(0x0e1711),
        _ => mix(rgb(0x343943), rgb(0x181d26), linear(u, v, aspect, 145.)),
    };
    c.map(|x| x.round().clamp(0., 255.) as u8)
}
pub(super) fn pixels(cfg: &Config, w: u32, h: u32, aspect: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let mut c = sample(
                &cfg.theme,
                (x as f32 + 0.5) / w as f32,
                (y as f32 + 0.5) / h as f32,
                aspect,
            );
            if cfg.theme == "terminal" && y % 4 == 0 {
                for (v, t) in c.iter_mut().zip([164., 239., 118.]) {
                    *v = (*v as f32 * (1. - 0.02) + t * 0.02).round() as u8;
                }
            }
            out.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_palette_and_afterglow_layers_are_preserved() {
        assert_eq!(sample("paper", 0.5, 0.5, 1.), [243, 242, 237]);
        assert_eq!(sample("modular", 0.5, 0.5, 1.), [8, 9, 9]);
        let sky = sample("sunset", 0.1, 0.03, 1.);
        let warm = sample("sunset", 0.2, 0.95, 1.);
        assert!(sky[2] > sky[0]);
        assert!(warm[0] > warm[2] + 30);
        assert!(warm[0] > 220);
    }
    #[test]
    fn all_nine_legacy_paints_are_opaque_and_bounded() {
        for (id, _) in super::super::model::THEMES {
            let cfg = Config {
                theme: id.into(),
                ..Config::default()
            };
            let p = pixels(&cfg, 32, 32, 1.);
            assert_eq!(p.len(), 4096);
            assert!(p.chunks_exact(4).all(|p| p[3] == 255));
            if let Some(folder) = std::env::var_os("LUMEN_CLOCK_VISUAL_PREVIEW_DIR") {
                let folder = std::path::PathBuf::from(folder);
                std::fs::create_dir_all(&folder).unwrap();
                image::RgbaImage::from_raw(256, 256, pixels(&cfg, 256, 256, 1.))
                    .unwrap()
                    .save(folder.join(format!("{id}-legacy-paint.png")))
                    .unwrap();
            }
        }
    }
}
