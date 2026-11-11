//! Cached surface accents derived from prominent saturated wallpaper colors.
pub const THEME: &str = "wallpaper-auto";
pub const FALLBACK: u32 = 0x00352f29;
fn pack(c: [f32; 3]) -> u32 {
    c[0].round() as u32 | (c[1].round() as u32) << 8 | (c[2].round() as u32) << 16
}
pub fn dominant(pixels: &[u8]) -> u32 {
    let mut bins = vec![[0f32; 4]; 4096];
    let mut counts = vec![0u32; 4096];
    for broad in [false, true] {
        for p in pixels.chunks_exact(4).filter(|p| p[3] >= 128) {
            let [r, g, b] = [p[0] as f32, p[1] as f32, p[2] as f32];
            let l = (54. * r + 183. * g + 19. * b) / 256.;
            if !broad && !(12. ..246.).contains(&l) {
                continue;
            }
            // Prefer colorful regions over broad pale/gray backgrounds, while
            // retaining area support so tiny colored highlights cannot win.
            let peak = r.max(g).max(b);
            let saturation = (peak - r.min(g).min(b)) / peak.max(1.);
            let weight = 0.12 + 4. * saturation * saturation;
            let index =
                ((p[0] as usize >> 4) << 8) | ((p[1] as usize >> 4) << 4) | (p[2] as usize >> 4);
            bins[index][0] += weight;
            bins[index][1] += r * weight;
            bins[index][2] += g * weight;
            bins[index][3] += b * weight;
            counts[index] += 1;
        }
        if bins.iter().any(|b| b[0] > 0.) {
            break;
        }
    }
    let colorful = |b: &[f32; 4]| {
        let color = [b[1] / b[0], b[2] / b[0], b[3] / b[0]];
        let peak = color.into_iter().fold(0f32, f32::max);
        let chroma = peak - color.into_iter().fold(255f32, f32::min);
        chroma >= 45. && chroma / peak.max(1.) >= 0.55
    };
    let total: u32 = counts.iter().sum();
    let saturated: u32 = bins
        .iter()
        .zip(&counts)
        .filter(|(b, _)| b[0] > 0. && colorful(b))
        .map(|(_, n)| *n)
        .sum();
    let prefer_saturated = saturated as f32 >= total as f32 * 0.05;
    // Aggregate neighboring bins before choosing: a colorful gradient is
    // spread over many bins, whereas a pale background often fits one bin.
    let mut scores = vec![0f32; 4096];
    for (i, bin) in bins.iter().enumerate().filter(|(_, b)| b[0] > 0.) {
        if prefer_saturated && !colorful(bin) {
            continue;
        }
        let center = [(i >> 8) as i32, ((i >> 4) & 15) as i32, (i & 15) as i32];
        for r in (center[0] - 1).max(0)..=(center[0] + 1).min(15) {
            for g in (center[1] - 1).max(0)..=(center[1] + 1).min(15) {
                for b in (center[2] - 1).max(0)..=(center[2] + 1).min(15) {
                    scores[((r << 8) | (g << 4) | b) as usize] += bin[0];
                }
            }
        }
    }
    let Some((best, _)) = bins
        .iter()
        .enumerate()
        .filter(|(_, b)| b[0] > 0. && (!prefer_saturated || colorful(b)))
        .max_by(|a, b| scores[a.0].total_cmp(&scores[b.0]))
    else {
        return FALLBACK;
    };
    let center = [
        (best >> 8) as i32,
        ((best >> 4) & 15) as i32,
        (best & 15) as i32,
    ];
    let mut sum = [0.; 4];
    for (i, bin) in bins.iter().enumerate() {
        let c = [(i >> 8) as i32, ((i >> 4) & 15) as i32, (i & 15) as i32];
        if (0..3).all(|n| (c[n] - center[n]).abs() <= 1) {
            if prefer_saturated && (bin[0] == 0. || !colorful(bin)) {
                continue;
            }
            for n in 0..4 {
                sum[n] += bin[n];
            }
        }
    }
    let color = [sum[1] / sum[0], sum[2] / sum[0], sum[3] / sum[0]];
    let peak = color.into_iter().fold(0f32, f32::max);
    if peak < 8. {
        return FALLBACK;
    }
    pack(color)
}
pub fn light(color: u32) -> bool {
    crate::contrast::luminance(color) >= 0.48
}
pub fn interaction(color: u32) -> u32 {
    let light = light(color);
    pack([0, 8, 16].map(|shift| {
        let c = ((color >> shift) & 255) as f32;
        if light {
            c * 0.55
        } else {
            c + (255. - c) * 0.32
        }
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wallpaper_auto_prefers_the_dominant_muted_color_over_small_highlights() {
        let mut pixels = [60, 76, 88, 255].repeat(1000);
        pixels.extend([220, 220, 220, 255].repeat(500));
        pixels.extend([130, 40, 220, 255].repeat(20));
        let color = dominant(&pixels);
        let r = color & 255;
        let g = (color >> 8) & 255;
        let b = (color >> 16) & 255;
        assert!(r < g && g < b && (80..=90).contains(&b));
        assert!(!light(color));
        assert_eq!(dominant(&[]), FALLBACK);
    }
    #[test]
    fn bright_wallpaper_retains_its_light_blue_instead_of_becoming_a_dark_palette() {
        let mut pixels = [170, 205, 225, 255].repeat(1000);
        pixels.extend([255, 255, 255, 255].repeat(80));
        let color = dominant(&pixels);
        assert!((color & 255) > 165);
        assert!(((color >> 8) & 255) > 195);
        assert!(((color >> 16) & 255) > 215);
        assert!(light(color));
    }
    #[test]
    fn saturated_wallpaper_region_wins_over_a_larger_pale_background() {
        let mut pixels = [170, 205, 225, 255].repeat(1000);
        pixels.extend([10, 65, 210, 255].repeat(400));
        pixels.extend([240, 20, 20, 255].repeat(10));
        let color = dominant(&pixels);
        assert!((color & 255) < 30);
        assert!((50..80).contains(&((color >> 8) & 255)));
        assert!(((color >> 16) & 255) > 200);
    }
    #[test]
    fn saturated_gradient_is_not_outvoted_by_one_uniform_pale_bin() {
        let mut pixels = [170, 205, 225, 255].repeat(1000);
        for i in 0..400u32 {
            pixels.extend_from_slice(&[
                (i % 48) as u8,
                (24 + i % 96) as u8,
                (200 + i % 55) as u8,
                255,
            ]);
        }
        let color = dominant(&pixels);
        let channels = [color & 255, (color >> 8) & 255, (color >> 16) & 255];
        let peak = *channels.iter().max().unwrap() as f32;
        let low = *channels.iter().min().unwrap() as f32;
        assert!((peak - low) / peak >= 0.55);
        assert!(channels[2] > 190);
    }
    #[test]
    fn interaction_colors_retain_the_selected_palette_hue() {
        let warm = interaction(0x00203070);
        let cool = interaction(0x00703020);
        assert!((warm & 255) > ((warm >> 16) & 255));
        assert!((cool & 255) < ((cool >> 16) & 255));
        assert!(!light(interaction(0x00eeeeee)));
    }
}
