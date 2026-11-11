//! Small, unblended background tiles. No content, desktop capture or alpha fade.
use super::geometry::Pane;
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Material {
    pub width: usize,
    pub height: usize,
    pub bgr: Vec<[u8; 3]>,
}
pub struct Field {
    pub window: [i32; 4],
    pub width: usize,
    pub height: usize,
    pub bgr: Vec<[f32; 3]>,
}
impl Field {
    pub fn sample(&self, x: f32, y: f32) -> [f32; 3] {
        let u = ((x - self.window[0] as f32) / (self.window[2] - self.window[0]) as f32
            * self.width as f32
            - 0.5)
            .clamp(0., (self.width - 1) as f32);
        let v = ((y - self.window[1] as f32) / (self.window[3] - self.window[1]) as f32
            * self.height as f32
            - 0.5)
            .clamp(0., (self.height - 1) as f32);
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        std::array::from_fn(|c| {
            let a = self.bgr[y0 * self.width + x0][c] * (1. - fx)
                + self.bgr[y0 * self.width + x1][c] * fx;
            let b = self.bgr[y1 * self.width + x0][c] * (1. - fx)
                + self.bgr[y1 * self.width + x1][c] * fx;
            a * (1. - fy) + b * fy
        })
    }
}
impl Material {
    /// Area filtering prevents individual grain/caustic pixels from becoming
    /// large bright cells when the low-resolution fusion tile is enlarged.
    pub fn from_bgra(width: usize, height: usize, pixels: &[u8]) -> Self {
        let (tw, th) = (width.min(64), height.min(64));
        let mut bgr = Vec::with_capacity(tw * th);
        for y in 0..th {
            for x in 0..tw {
                let (l, r, t, b) = (
                    x * width / tw,
                    (x + 1) * width / tw,
                    y * height / th,
                    (y + 1) * height / th,
                );
                let mut sum = [0u64; 3];
                for sy in t..b {
                    for sx in l..r {
                        let i = (sy * width + sx) * 4;
                        for c in 0..3 {
                            sum[c] += pixels[i + c] as u64;
                        }
                    }
                }
                let n = ((r - l) * (b - t)) as u64;
                bgr.push(sum.map(|v| ((v + n / 2) / n) as u8));
            }
        }
        // A small positive Gaussian kernel softens the reflection, without
        // ringing, alpha changes or overshoot into brighter/darker colors.
        let mut scratch = bgr.clone();
        for axis in 0..2 {
            for y in 0..th {
                for x in 0..tw {
                    let mut sum = [0u32; 3];
                    for (k, w) in [1, 4, 6, 4, 1].into_iter().enumerate() {
                        let sx = if axis == 0 {
                            (x as isize + k as isize - 2).clamp(0, tw as isize - 1) as usize
                        } else {
                            x
                        };
                        let sy = if axis == 1 {
                            (y as isize + k as isize - 2).clamp(0, th as isize - 1) as usize
                        } else {
                            y
                        };
                        for c in 0..3 {
                            sum[c] += bgr[sy * tw + sx][c] as u32 * w;
                        }
                    }
                    scratch[y * tw + x] = sum.map(|v| ((v + 8) / 16) as u8);
                }
            }
            std::mem::swap(&mut bgr, &mut scratch);
        }
        Self {
            width: tw,
            height: th,
            bgr,
        }
    }
    pub fn sample(&self, window: [i32; 4], x: f32, y: f32) -> [f32; 3] {
        let u = ((x - window[0] as f32) / (window[2] - window[0]).max(1) as f32
            * self.width as f32
            - 0.5)
            .clamp(0., (self.width - 1) as f32);
        let v = ((y - window[1] as f32) / (window[3] - window[1]).max(1) as f32
            * self.height as f32
            - 0.5)
            .clamp(0., (self.height - 1) as f32);
        let (x0, y0) = (u.floor() as usize, v.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.width - 1), (y0 + 1).min(self.height - 1));
        let (fx, fy) = (u - x0 as f32, v - y0 as f32);
        std::array::from_fn(|c| {
            let a = self.bgr[y0 * self.width + x0][c] as f32 * (1. - fx)
                + self.bgr[y0 * self.width + x1][c] as f32 * fx;
            let b = self.bgr[y1 * self.width + x0][c] as f32 * (1. - fx)
                + self.bgr[y1 * self.width + x1][c] as f32 * fx;
            a * (1. - fy) + b * fy
        })
    }
}
pub fn blend<'a>(
    sources: impl Iterator<Item = (&'a Pane, [i32; 4], &'a Material)>,
    x: f32,
    y: f32,
) -> Option<[f32; 3]> {
    let mut sum = 0.;
    let mut rgb = [0.; 3];
    for (pane, window, material) in sources {
        // Signed distance is negative inside. Outside, extend the edge color
        // smoothly into the neighboring pane; both sides evaluate one field.
        let weight = (-pane.distance(x, y).max(0.) / (48. * pane.dpi)).exp();
        let sample = material.sample(window, x, y);
        for c in 0..3 {
            rgb[c] += sample[c] * weight;
        }
        sum += weight;
    }
    (sum > 0.).then(|| rgb.map(|v| v / sum))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downsampled_grain_does_not_turn_into_bright_reflection_cells() {
        let mut source = vec![0u8; 256 * 256 * 4];
        for y in 0..256 {
            for x in 0..256 {
                let v = if (x + y) % 2 == 0 { 52 } else { 28 };
                source[(y * 256 + x) * 4..(y * 256 + x) * 4 + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        let tile = Material::from_bgra(256, 256, &source);
        assert!(
            tile.bgr.iter().all(|p| *p == [40; 3]),
            "area filtering must reject phase-locked grain aliasing"
        );
    }
    #[test]
    fn localized_reflection_softens_without_ringing_or_extra_color_quantization() {
        let mut source = vec![40u8; 64 * 64 * 4];
        source[(16 * 64 + 16) * 4..(16 * 64 + 16) * 4 + 3].fill(240);
        let tile = Material::from_bgra(64, 64, &source);
        let center = tile.bgr[16 * 64 + 16][0];
        assert!(center > 40 && center < 100);
        assert!(tile.bgr[16 * 64 + 17][0] < center);
        assert!(tile.bgr[16 * 64 + 18][0] <= tile.bgr[16 * 64 + 17][0]);
        assert!(tile.bgr.iter().flatten().all(|v| *v >= 40 && *v <= center));
        let field = Field {
            window: [0, 0, 32, 16],
            width: 2,
            height: 1,
            bgr: vec![[100.1; 3], [100.3; 3]],
        };
        assert!(
            (field.sample(16., 8.)[0] - 100.2).abs() < 0.001,
            "field retains sub-byte lighting precision"
        );
    }
    #[test]
    fn joining_saturated_panels_mix_colors_without_losing_opacity_or_using_wallpaper() {
        let a = Pane {
            id: 1,
            rect: [0., 0., 200., 200.],
            radius: 34.,
            dpi: 1.,
            topmost: false,
        };
        let b = Pane {
            id: 2,
            rect: [0., 200., 200., 400.],
            ..a
        };
        let red = Material {
            width: 1,
            height: 1,
            bgr: vec![[0, 0, 255]],
        };
        let blue = Material {
            width: 1,
            height: 1,
            bgr: vec![[255, 0, 0]],
        };
        let field = |y| {
            blend(
                [
                    (&a, [0, 0, 200, 200], &red),
                    (&b, [0, 200, 200, 400], &blue),
                ]
                .into_iter(),
                100.,
                y,
            )
            .unwrap()
        };
        let seam = field(200.);
        assert!((seam[0] - 127.5).abs() < 0.01 && seam[1] == 0. && (seam[2] - 127.5).abs() < 0.01);
        assert!(field(140.)[2] > field(140.)[0]);
        assert!(field(260.)[0] > field(260.)[2]);
        let before = field(199.99);
        let after = field(200.01);
        assert!(before
            .into_iter()
            .zip(after)
            .all(|(a, b)| (a - b).abs() < 0.1));
    }
    #[test]
    fn source_gradient_is_bilinear_and_extends_its_edge_instead_of_fading_out() {
        let m = Material {
            width: 2,
            height: 1,
            bgr: vec![[20, 40, 60], [100, 120, 140]],
        };
        assert_eq!(m.sample([0, 0, 100, 100], 50., 50.), [60., 80., 100.]);
        assert_eq!(m.sample([0, 0, 100, 100], 200., 50.), [100., 120., 140.]);
    }
}
