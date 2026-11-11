//! Premultiplied bilinear sprite sampling with continuous subpixel geometry.
#[derive(Clone, Copy)]
pub struct Target {
    pub bits: *mut u8,
    pub width: usize,
    pub height: usize,
    pub clip: [i32; 4],
    pub opaque: bool,
}
impl Target {
    // Cached sprites retain their device size while moving. Fixed interpolation
    // weights avoid inverse affine math and floating point work for every pixel.
    unsafe fn translate(self, source: &[u8], size: [usize; 2], origin: [f32; 2], opacity: f32) {
        let ox = origin[0].floor() as i32;
        let oy = origin[1].floor() as i32;
        let fx = ((origin[0] - ox as f32) * 256.).round() as u32;
        let fy = ((origin[1] - oy as f32) * 256.).round() as u32;
        let weights = [
            (256 - fx) * (256 - fy),
            fx * (256 - fy),
            (256 - fx) * fy,
            fx * fy,
        ];
        let opacity = (opacity.clamp(0., 1.) * 256.).round() as u32;
        let left = ox.max(self.clip[0]).max(0);
        let top = oy.max(self.clip[1]).max(0);
        let right = (ox + size[0] as i32 + 1)
            .min(self.clip[2])
            .min(self.width as i32);
        let bottom = (oy + size[1] as i32 + 1)
            .min(self.clip[3])
            .min(self.height as i32);
        let dest = std::slice::from_raw_parts_mut(self.bits, self.width * self.height * 4);
        for y in top..bottom {
            for x in left..right {
                let sx = x - ox;
                let sy = y - oy;
                let mut sample = [0u32; 4];
                for (i, (qx, qy)) in [(sx, sy), (sx - 1, sy), (sx, sy - 1), (sx - 1, sy - 1)]
                    .into_iter()
                    .enumerate()
                {
                    let w = weights[i];
                    if w == 0 || qx < 0 || qy < 0 || qx >= size[0] as i32 || qy >= size[1] as i32 {
                        continue;
                    }
                    let offset = (qy as usize * size[0] + qx as usize) * 4;
                    if source[offset + 3] == 0 {
                        continue;
                    }
                    for c in 0..4 {
                        sample[c] += source[offset + c] as u32 * w;
                    }
                }
                let alpha = (((sample[3] + 32768) >> 16) * opacity + 128) >> 8;
                if alpha == 0 {
                    continue;
                }
                let offset = (y as usize * self.width + x as usize) * 4;
                for c in 0..if self.opaque { 3 } else { 4 } {
                    let value = (((sample[c] + 32768) >> 16) * opacity + 128) >> 8;
                    dest[offset + c] = (value
                        + (dest[offset + c] as u32 * (255 - alpha) + 127) / 255)
                        .min(255) as u8;
                }
            }
        }
    }
    unsafe fn axis_aligned(self, source: &[u8], size: [usize; 2], r: [f32; 4], opacity: f32) {
        let pad = ((r[2] - r[0]) / size[0] as f32)
            .max((r[3] - r[1]) / size[1] as f32)
            .max(1.);
        let left = ((r[0] - pad).floor() as i32).max(self.clip[0]).max(0);
        let top = ((r[1] - pad).floor() as i32).max(self.clip[1]).max(0);
        let right = ((r[2] + pad).ceil() as i32)
            .min(self.clip[2])
            .min(self.width as i32);
        let bottom = ((r[3] + pad).ceil() as i32)
            .min(self.clip[3])
            .min(self.height as i32);
        let axis = |a: i32, b: i32, start: f32, end: f32, source: usize| -> Vec<(i32, u32)> {
            (a..b)
                .map(|v| {
                    let p = (v as f32 + 0.5 - start) / (end - start) * source as f32 - 0.5;
                    let i = p.floor();
                    (i as i32, ((p - i) * 256.).round() as u32)
                })
                .collect()
        };
        let xs = axis(left, right, r[0], r[2], size[0]);
        let ys = axis(top, bottom, r[1], r[3], size[1]);
        let opacity = (opacity.clamp(0., 1.) * 256.).round() as u32;
        let dest = std::slice::from_raw_parts_mut(self.bits, self.width * self.height * 4);
        for (yy, (sy, fy)) in ys.into_iter().enumerate() {
            for (xx, &(sx, fx)) in xs.iter().enumerate() {
                let weights = [
                    (256 - fx) * (256 - fy),
                    fx * (256 - fy),
                    (256 - fx) * fy,
                    fx * fy,
                ];
                let mut sample = [0u32; 4];
                for (i, (qx, qy)) in [(sx, sy), (sx + 1, sy), (sx, sy + 1), (sx + 1, sy + 1)]
                    .into_iter()
                    .enumerate()
                {
                    let w = weights[i];
                    if w == 0 || qx < 0 || qy < 0 || qx >= size[0] as i32 || qy >= size[1] as i32 {
                        continue;
                    }
                    let offset = (qy as usize * size[0] + qx as usize) * 4;
                    if source[offset + 3] == 0 {
                        continue;
                    }
                    for c in 0..4 {
                        sample[c] += source[offset + c] as u32 * w;
                    }
                }
                let alpha = (((sample[3] + 32768) >> 16) * opacity + 128) >> 8;
                if alpha == 0 {
                    continue;
                }
                let offset = ((top as usize + yy) * self.width + left as usize + xx) * 4;
                for c in 0..if self.opaque { 3 } else { 4 } {
                    let value = (((sample[c] + 32768) >> 16) * opacity + 128) >> 8;
                    dest[offset + c] = (value
                        + (dest[offset + c] as u32 * (255 - alpha) + 127) / 255)
                        .min(255) as u8;
                }
            }
        }
    }
    // The UI thread owns this DIB; callers flush GDI/GDI+ before accessing it.
    pub unsafe fn draw(
        self,
        source: &[u8],
        size: [usize; 2],
        r: [f32; 4],
        transform: [f32; 4],
        opacity: f32,
        tint: Option<[f32; 3]>,
    ) {
        if (transform[2] - 1.).abs() < 0.00001
            && transform[3].abs() < 0.00001
            && ((r[2] - r[0]) - size[0] as f32).abs() < 0.01
            && ((r[3] - r[1]) - size[1] as f32).abs() < 0.01
        {
            self.translate(source, size, [r[0], r[1]], opacity);
            return;
        }
        if (transform[2] - 1.).abs() < 0.00001 && transform[3].abs() < 0.00001 {
            self.axis_aligned(source, size, r, opacity);
            return;
        }
        let [cx, cy, scale, angle] = transform;
        let angle = angle.to_radians();
        let (sin, cos) = angle.sin_cos();
        let forward = |x: f32, y: f32| {
            [
                cx + ((x - cx) * cos - (y - cy) * sin) * scale,
                cy + ((x - cx) * sin + (y - cy) * cos) * scale,
            ]
        };
        let pad = ((r[2] - r[0]) / size[0] as f32)
            .max((r[3] - r[1]) / size[1] as f32)
            .max(1.);
        let corners = [
            forward(r[0] - pad, r[1] - pad),
            forward(r[2] + pad, r[1] - pad),
            forward(r[0] - pad, r[3] + pad),
            forward(r[2] + pad, r[3] + pad),
        ];
        let left = (corners
            .iter()
            .map(|p| p[0])
            .fold(f32::INFINITY, f32::min)
            .floor() as i32)
            .max(self.clip[0])
            .max(0);
        let top = (corners
            .iter()
            .map(|p| p[1])
            .fold(f32::INFINITY, f32::min)
            .floor() as i32)
            .max(self.clip[1])
            .max(0);
        let right = (corners
            .iter()
            .map(|p| p[0])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as i32)
            .min(self.clip[2])
            .min(self.width as i32);
        let bottom = (corners
            .iter()
            .map(|p| p[1])
            .fold(f32::NEG_INFINITY, f32::max)
            .ceil() as i32)
            .min(self.clip[3])
            .min(self.height as i32);
        let pixels = std::slice::from_raw_parts_mut(self.bits, self.width * self.height * 4);
        for y in top..bottom {
            for x in left..right {
                let dx = (x as f32 + 0.5 - cx) / scale;
                let dy = (y as f32 + 0.5 - cy) / scale;
                let px = cx + dx * cos + dy * sin;
                let py = cy - dx * sin + dy * cos;
                let sx = (px - r[0]) / (r[2] - r[0]) * size[0] as f32 - 0.5;
                let sy = (py - r[1]) / (r[3] - r[1]) * size[1] as f32 - 0.5;
                let ix = sx.floor() as i32;
                let iy = sy.floor() as i32;
                if ix < -1 || iy < -1 || ix >= size[0] as i32 || iy >= size[1] as i32 {
                    continue;
                }
                let fx = sx - ix as f32;
                let fy = sy - iy as f32;
                let mut sample = [0.; 4];
                let taps = [
                    (ix, iy, (1. - fx) * (1. - fy)),
                    (ix + 1, iy, fx * (1. - fy)),
                    (ix, iy + 1, (1. - fx) * fy),
                    (ix + 1, iy + 1, fx * fy),
                ];
                let mut offsets = [usize::MAX; 4];
                for (index, (qx, qy, weight)) in taps.iter().copied().enumerate() {
                    if qx >= 0 && qy >= 0 && qx < size[0] as i32 && qy < size[1] as i32 {
                        let offset = (qy as usize * size[0] + qx as usize) * 4;
                        offsets[index] = offset;
                        sample[3] += source[offset + 3] as f32 * weight;
                    }
                }
                sample[3] *= opacity;
                if sample[3] < 0.001 {
                    continue;
                }
                if let Some(tint) = tint {
                    for c in 0..3 {
                        sample[c] = tint[c] * sample[3];
                    }
                } else {
                    for (index, (_, _, weight)) in taps.iter().copied().enumerate() {
                        let offset = offsets[index];
                        if offset != usize::MAX {
                            for c in 0..3 {
                                sample[c] += source[offset + c] as f32 * weight * opacity;
                            }
                        }
                    }
                }
                let dest = (y as usize * self.width + x as usize) * 4;
                let remaining = 1. - sample[3] / 255.;
                for c in 0..3 {
                    pixels[dest + c] = (sample[c] + pixels[dest + c] as f32 * remaining)
                        .round()
                        .clamp(0., 255.) as u8;
                }
                if !self.opaque {
                    pixels[dest + 3] = (sample[3] + pixels[dest + 3] as f32 * remaining)
                        .round()
                        .clamp(0., 255.) as u8;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translated_sprite_matches_general_affine_sampler() {
        let source: Vec<u8> = (0..48)
            .flat_map(|n| {
                let a = ((n * 29) % 256) as u8;
                [a / 3, a / 2, a, a]
            })
            .collect();
        for origin in [[0., 0.], [-0.3, 2.6], [3.25, 1.75], [2.999, 2.001]] {
            for factor in [0.7, 1., 1.05, 1.4] {
                for opaque in [false, true] {
                    let mut fast = vec![31u8; 16 * 16 * 4];
                    let mut reference = fast.clone();
                    unsafe {
                        let target = Target {
                            bits: fast.as_mut_ptr(),
                            width: 16,
                            height: 16,
                            clip: [1, 1, 15, 14],
                            opaque,
                        };
                        let r = [
                            origin[0],
                            origin[1],
                            origin[0] + 8. * factor,
                            origin[1] + 6. * factor,
                        ];
                        target.draw(&source, [8, 6], r, [0., 0., 1., 0.], 0.73, None);
                        Target {
                            bits: reference.as_mut_ptr(),
                            ..target
                        }
                        .draw(
                            &source,
                            [8, 6],
                            r,
                            [0., 0., 1.00002, 0.],
                            0.73,
                            None,
                        );
                    }
                    assert!(fast
                        .iter()
                        .zip(&reference)
                        .all(|(a, b)| a.abs_diff(*b) <= 2));
                }
            }
        }
    }
}
