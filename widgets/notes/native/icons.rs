//! Small outline icons for note actions; geometry uses DIP and native AA.
use super::*;
use crate::render::Image;
use std::collections::HashMap;

thread_local! {
    static SEARCH_CACHE: RefCell<HashMap<(u32, u32, u8), Vec<u8>>> = RefCell::new(HashMap::new());
}

pub fn pin(c: &Canvas, x: f32, y: f32, ink: u32, pinned: bool, hover: f32, hot: bool) {
    let hover = hover.clamp(0., 1.);
    if hover <= 0.001 {
        return;
    }
    if hot {
        c.round(
            [x - 4., y - 4., x + 20., y + 20.],
            12.,
            ink,
            (26. * hover).round() as u8,
            false,
            false,
        );
    }
    let alpha = ((if pinned || hot { 245. } else { 165. }) * hover)
        .round()
        .clamp(0., 255.) as u8;
    if alpha == 0 {
        return;
    }
    let width = if pinned { 1.45 } else { 1.25 };
    c.line(
        &[
            (x + 5., y + 1.),
            (x + 11., y + 1.),
            (x + 10., y + 6.),
            (x + 13., y + 9.),
            (x + 3., y + 9.),
            (x + 6., y + 6.),
            (x + 5., y + 1.),
        ],
        ink,
        alpha,
        width,
    );
    if pinned {
        c.round(
            [x + 5.5, y + 2., x + 10.5, y + 8.5],
            2.,
            ink,
            (95. * hover).round() as u8,
            false,
            false,
        );
    }
    c.line(
        &[(x + 8., y + 9.), (x + 8., y + 15.)],
        ink,
        alpha,
        width,
    );
}
pub fn trash(c: &Canvas, x: f32, y: f32, ink: u32, hover: f32, hot: bool) {
    let hover = hover.clamp(0., 1.);
    if hover <= 0.001 {
        return;
    }
    if hot {
        c.round(
            [x - 4., y - 4., x + 20., y + 20.],
            12.,
            ink,
            (26. * hover).round() as u8,
            false,
            false,
        );
    }
    let alpha = ((if hot { 245. } else { 165. }) * hover)
        .round()
        .clamp(0., 255.) as u8;
    if alpha == 0 {
        return;
    }
    c.line(&[(x + 2., y + 4.), (x + 14., y + 4.)], ink, alpha, 1.25);
    c.line(
        &[
            (x + 5., y + 4.),
            (x + 5., y + 1.5),
            (x + 11., y + 1.5),
            (x + 11., y + 4.),
        ],
        ink,
        alpha,
        1.25,
    );
    c.line(
        &[
            (x + 4., y + 5.),
            (x + 4.8, y + 14.),
            (x + 11.2, y + 14.),
            (x + 12., y + 5.),
        ],
        ink,
        alpha,
        1.25,
    );
    c.line(&[(x + 7., y + 6.5), (x + 7., y + 12.)], ink, alpha, 1.);
    c.line(&[(x + 9., y + 6.5), (x + 9., y + 12.)], ink, alpha, 1.);
}
pub fn folder(c: &Canvas, x: f32, y: f32, ink: u32) {
    c.line(
        &[
            (x + 1., y + 13.),
            (x + 1., y + 3.),
            (x + 6., y + 3.),
            (x + 8., y + 5.),
            (x + 15., y + 5.),
            (x + 15., y + 13.),
            (x + 1., y + 13.),
        ],
        ink,
        180,
        1.25,
    );
}
pub fn search(c: &Canvas, dpi: f32, x: f32, y: f32, ink: u32, alpha: u8) {
    let side = (16. * dpi * 2.).ceil().max(16.) as u32;
    let key = (side, ink, alpha);
    SEARCH_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&key) {
            if cache.len() >= 16 {
                cache.clear();
            }
            let scale = side as f32 / 16.;
            let aa = 1. / scale;
            let half_w = 0.92f32;
            let r = (ink & 255) as f32;
            let g = ((ink >> 8) & 255) as f32;
            let b = ((ink >> 16) & 255) as f32;
            let max_a = alpha as f32 / 255.;
            let mut pixels = vec![0u8; (side * side * 4) as usize];
            for py in 0..side {
                for px in 0..side {
                    let ux = (px as f32 + 0.5) / scale;
                    let uy = (py as f32 + 0.5) / scale;
                    let d_ring = ((ux - 6.8).hypot(uy - 6.8) - 4.4).abs();
                    let (vx, vy) = (4.2f32, 4.2f32);
                    let (wx, wy) = (ux - 10.0, uy - 10.0);
                    let t = ((wx * vx + wy * vy) / (vx * vx + vy * vy)).clamp(0., 1.);
                    let d_handle = (wx - t * vx).hypot(wy - t * vy);
                    let d = d_ring.min(d_handle);
                    let cov = ((half_w - d) / aa + 0.5).clamp(0., 1.) * max_a;
                    if cov > 0. {
                        let a = (cov * 255.).round().clamp(0., 255.) as u8;
                        let af = a as f32 / 255.;
                        let i = ((py * side + px) * 4) as usize;
                        pixels[i] = (r * af).round().min(a as f32) as u8;
                        pixels[i + 1] = (g * af).round().min(a as f32) as u8;
                        pixels[i + 2] = (b * af).round().min(a as f32) as u8;
                        pixels[i + 3] = a;
                    }
                }
            }
                cache.insert(key, pixels);
        }
        if let Some(pixels) = cache.get(&key) {
            if let Some(image) = Image::from_rgba(side, side, pixels.clone()) {
                c.image(&image, [x, y, x + 16., y + 16.]);
            }
        }
    });
}
pub fn bead(c: &Canvas, x: f32, y: f32, color: u32) {
    let r = [x, y, x + 16., y + 16.];
    c.soft_shadow(r, 8., 50., 3., 1.5);
    c.round(r, 8., color, 245, false, false);
    c.glow(
        [x + 1., y + 1., x + 10., y + 10.],
        4.5,
        [x + 4., y + 3.],
        0xffffff,
        190,
    );
    c.round(r, 8., 0xffffff, 100, true, false);
}
pub fn note_day(stamp: u64) -> String {
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ticks = stamp
        .saturating_mul(10_000)
        .saturating_add(116_444_736_000_000_000);
    let time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    unsafe {
        let mut utc: SYSTEMTIME = zeroed();
        let mut local: SYSTEMTIME = zeroed();
        if FileTimeToSystemTime(&time, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(null(), &utc, &mut local) == 0
        {
            return "Saved".into();
        }
        ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][local.wDayOfWeek as usize % 7].into()
    }
}
