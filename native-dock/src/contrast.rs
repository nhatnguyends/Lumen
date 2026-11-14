//! Contrast is measured from the final glass DIB, including wallpaper/tint/frost.
use windows_sys::Win32::Graphics::Gdi::*;
pub fn luminance(color: u32) -> f32 {
    let channel = |shift: u32| {
        let v = ((color >> shift) & 255u32) as f32 / 255.;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(0) + 0.7152 * channel(8) + 0.0722 * channel(16)
}
pub fn ratio(a: f32, b: f32) -> f32 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
pub fn inks(background: f32, previous: Option<u32>) -> (u32, u32) {
    // Prefer light foregrounds on smoked/mid-gray glass. Choosing only the
    // largest mathematical contrast flips to black around ordinary gray,
    // even though that surface still reads as dark glass. Switch the entire
    // widget only on clearly bright glass, with a wide stable hysteresis band.
    let light_ink = match previous {
        Some(old) if luminance(old) < 0.5 => background < 0.26,
        Some(_) => background < 0.36,
        None => background < 0.31,
    };
    let selected = if light_ink {
        rgb(248, 250, 253)
    } else {
        rgb(12, 16, 23)
    };
    let muted = if luminance(selected) > 0.5 {
        rgb(193, 202, 215)
    } else {
        rgb(48, 58, 73)
    };
    (
        selected,
        if ratio(background, luminance(muted)) >= 4.5 {
            muted
        } else {
            selected
        },
    )
}
pub unsafe fn sample(dc: HDC, dpi: f32, rect: [f32; 4]) -> f32 {
    GdiFlush();
    let mut dib: DIBSECTION = std::mem::zeroed();
    if GetObjectW(
        GetCurrentObject(dc, OBJ_BITMAP as u32),
        std::mem::size_of::<DIBSECTION>() as i32,
        &mut dib as *mut _ as _,
    ) != std::mem::size_of::<DIBSECTION>() as i32
        || dib.dsBm.bmBits.is_null()
        || dib.dsBm.bmBitsPixel != 32
    {
        return 0.1;
    }
    let width = dib.dsBm.bmWidth;
    let height = dib.dsBm.bmHeight.abs();
    let stride = dib.dsBm.bmWidthBytes as usize;
    let bytes = std::slice::from_raw_parts(dib.dsBm.bmBits as *const u8, stride * height as usize);
    let mut samples = [0.0f32; 21];
    let mut idx = 0;
    for y in 0..3 {
        for x in 0..7 {
            let px = ((rect[0] + (rect[2] - rect[0]) * (x as f32 + 0.5) / 7.) * dpi)
                .round()
                .clamp(0., width as f32 - 1.) as usize;
            let py = ((rect[1] + (rect[3] - rect[1]) * (y as f32 + 0.5) / 3.) * dpi)
                .round()
                .clamp(0., height as f32 - 1.) as usize;
            let i = py * stride + px * 4;
            samples[idx] = luminance(rgb(
                bytes[i + 2] as u32,
                bytes[i + 1] as u32,
                bytes[i] as u32,
            ));
            idx += 1;
        }
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    samples[5..16].iter().sum::<f32>() / 11.
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn composited_glass_includes_tint_opacity_and_prefers_light_ink_on_gray() {
        for alpha in [0., 0.1, 0.3, 0.5, 0.8, 1.] {
            let channel = (245. * (1. - alpha) + 18. * alpha) as u32;
            let bg = luminance(rgb(channel, channel, channel));
            let (ink, muted) = inks(bg, None);
            assert!(ratio(bg, luminance(ink)) >= 3.);
            assert!(ratio(bg, luminance(muted)) >= 3.);
            if alpha < 0.2 {
                assert!(luminance(ink) < 0.1);
            }
            if alpha > 0.8 {
                assert!(luminance(ink) > 0.8);
            }
        }
    }
    #[test]
    fn mid_gray_glass_stays_white_even_if_previous_ink_was_black() {
        for gray in [90, 110, 130] {
            let bg = luminance(rgb(gray, gray, gray));
            assert!(luminance(inks(bg, None).0) > 0.8);
            assert!(luminance(inks(bg, Some(rgb(12, 16, 23))).0) > 0.8);
        }
        assert!(luminance(inks(luminance(rgb(210, 210, 210)), Some(0xffffff)).0) < 0.1);
    }
    #[test]
    fn transition_band_keeps_one_global_palette_stable() {
        let white = rgb(248, 250, 253);
        let dark = rgb(12, 16, 23);
        assert_eq!(inks(0.32, Some(white)).0, white);
        assert_eq!(inks(0.32, Some(dark)).0, dark);
        assert_eq!(inks(0.25, Some(dark)).0, white);
        assert_eq!(inks(0.37, Some(white)).0, dark);
    }
    #[test]
    fn small_background_changes_keep_readable_ink_stable() {
        let (ink, _) = inks(0.05, None);
        assert_eq!(inks(0.06, Some(ink)).0, ink);
    }
}
