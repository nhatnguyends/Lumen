//! Premultiplied per-pixel window edges. No binary region and no screen capture.
use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

#[inline]
pub fn distance(x: f32, y: f32, w: f32, h: f32, r: f32, inset: f32) -> f32 {
    let qx = (x - w * 0.5).abs() - (w * 0.5 - inset - r);
    let qy = (y - h * 0.5).abs() - (h * 0.5 - inset - r);
    let outside = if qx > 0. && qy > 0. {
        (qx * qx + qy * qy).sqrt()
    } else {
        qx.max(qy).max(0.)
    };
    outside + qx.max(qy).min(0.) - r
}
pub(crate) struct PolySegment {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub ta: [f32; 2],
    pub tb: [f32; 2],
    pub outward: [f32; 2],
    pub horiz: bool,
}
pub(crate) struct PolyCorner {
    pub v: [f32; 2],
    pub c: [f32; 2],
    pub r: f32,
    pub convex: bool,
    pub sx: f32,
    pub sy: f32,
}
pub(crate) struct PolySdf {
    pub bounds: [f32; 4],
    pub max_radius: f32,
    pub segments: Vec<PolySegment>,
    pub corners: Vec<PolyCorner>,
}
impl PolySdf {
    pub(crate) fn new(verts: &[[f32; 2]], radius: f32) -> Self {
        let n = verts.len();
        let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
        let mut corners = Vec::with_capacity(n);
        let mut max_radius = 0.0f32;
        for i in 0..n {
            let v = verts[i];
            bounds[0] = bounds[0].min(v[0]);
            bounds[1] = bounds[1].min(v[1]);
            bounds[2] = bounds[2].max(v[0]);
            bounds[3] = bounds[3].max(v[1]);
            let prev = verts[(i + n - 1) % n];
            let next = verts[(i + 1) % n];
            let l_in = (v[0] - prev[0]).hypot(v[1] - prev[1]).max(1e-4);
            let l_out = (next[0] - v[0]).hypot(next[1] - v[1]).max(1e-4);
            let d_in = [(v[0] - prev[0]) / l_in, (v[1] - prev[1]) / l_in];
            let d_out = [(next[0] - v[0]) / l_out, (next[1] - v[1]) / l_out];
            let r = radius.min(l_in * 0.5).min(l_out * 0.5).max(0.1);
            max_radius = max_radius.max(r);
            let cross = d_in[0] * d_out[1] - d_in[1] * d_out[0];
            let c = [v[0] - d_in[0] * r + d_out[0] * r, v[1] - d_in[1] * r + d_out[1] * r];
            corners.push(PolyCorner {
                v,
                c,
                r,
                convex: cross > 0.0,
                sx: if v[0] >= c[0] { 1.0 } else { -1.0 },
                sy: if v[1] >= c[1] { 1.0 } else { -1.0 },
            });
        }
        let mut segments = Vec::with_capacity(n);
        for i in 0..n {
            let j = (i + 1) % n;
            let a = verts[i];
            let b = verts[j];
            let len = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-4);
            let dir = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
            let ra = corners[i].r;
            let rb = corners[j].r;
            let ta = [a[0] + dir[0] * ra, a[1] + dir[1] * ra];
            let tb = [b[0] - dir[0] * rb, b[1] - dir[1] * rb];
            let horiz = (b[1] - a[1]).abs() < (b[0] - a[0]).abs();
            segments.push(PolySegment {
                a,
                b,
                ta,
                tb,
                outward: [dir[1], -dir[0]],
                horiz,
            });
        }
        Self {
            bounds,
            max_radius,
            segments,
            corners,
        }
    }
    pub(crate) fn interior_x_spans(&self, py: f32, out: &mut Vec<f32>) {
        out.clear();
        for seg in &self.segments {
            if !seg.horiz && ((seg.a[1] > py) != (seg.b[1] > py)) {
                out.push(seg.a[0]);
            }
        }
        out.sort_by(|a, b| a.total_cmp(b));
    }
    pub(crate) fn signed_distance(&self, px: f32, py: f32) -> (f32, f32, f32) {
        let mut inside = false;
        for seg in &self.segments {
            if !seg.horiz && ((seg.a[1] > py) != (seg.b[1] > py)) && px < seg.a[0] {
                inside = !inside;
            }
        }
        for c in &self.corners {
            let x0 = c.c[0].min(c.v[0]);
            let x1 = c.c[0].max(c.v[0]);
            let y0 = c.c[1].min(c.v[1]);
            let y1 = c.c[1].max(c.v[1]);
            if px >= x0 && px <= x1 && py >= y0 && py <= y1 {
                let d2 = (px - c.c[0]) * (px - c.c[0]) + (py - c.c[1]) * (py - c.c[1]);
                inside = if c.convex { d2 <= c.r * c.r } else { d2 >= c.r * c.r };
                break;
            }
        }
        let mut best_d = f32::INFINITY;
        let mut best_n = [0.0f32, -1.0f32];
        for seg in &self.segments {
            let dx = seg.tb[0] - seg.ta[0];
            let dy = seg.tb[1] - seg.ta[1];
            let l2 = dx * dx + dy * dy;
            let (qx, qy) = if l2 <= 1e-6 {
                (seg.ta[0], seg.ta[1])
            } else {
                let t = (((px - seg.ta[0]) * dx + (py - seg.ta[1]) * dy) / l2).clamp(0.0, 1.0);
                (seg.ta[0] + dx * t, seg.ta[1] + dy * t)
            };
            let d = (px - qx).hypot(py - qy);
            if d < best_d {
                best_d = d;
                best_n = seg.outward;
            }
        }
        for c in &self.corners {
            let vx = px - c.c[0];
            let vy = py - c.c[1];
            if vx * c.sx >= 0.0 && vy * c.sy >= 0.0 {
                let dist_c = vx.hypot(vy);
                let d = (dist_c - c.r).abs();
                if d < best_d {
                    best_d = d;
                    if dist_c > 1e-4 {
                        let sign = if c.convex { 1.0 } else { -1.0 };
                        best_n = [(vx / dist_c) * sign, (vy / dist_c) * sign];
                    } else {
                        let s = std::f32::consts::FRAC_1_SQRT_2 * if c.convex { 1.0 } else { -1.0 };
                        best_n = [c.sx * s, c.sy * s];
                    }
                }
            }
        }
        if best_d == f32::INFINITY {
            best_d = 0.0;
        }
        (if inside { -best_d } else { best_d }, best_n[0], best_n[1])
    }
}
struct Rim {
    index: usize,
    sample: [f32; 2],
    light: f32,
    liquid_spec: f32,
}
#[inline]
pub(crate) fn liquid_spec_width(dpi: f32, w: f32, h: f32) -> f32 {
    0.32 * (44.0 * dpi).min(w.min(h) * 0.45).max(1.0)
}
#[inline]
pub(crate) fn liquid_rim_spec(d: f32, nx: f32, ny: f32, spec_w: f32) -> f32 {
    if d > 0.5 || -d >= spec_w {
        return 0.;
    }
    let edge = (1. - (-d).max(0.) / spec_w).clamp(0., 1.);
    let light_dir = (-nx * 0.55 - ny * 0.83).max(0.) + (nx * 0.4 + ny * 0.6).max(0.) * 0.35;
    edge * edge * edge * (16. + 34. * light_dir)
}
fn window_pixel_alpha(alpha: u8, coverage: u8, preserve: bool) -> u8 {
    if !preserve {
        return coverage;
    }
    if coverage == 0 {
        return 0;
    }
    // Windows hit-tests layered windows through nonzero alpha. A one-step
    // floor keeps transparent chrome draggable without a visible backing.
    (((alpha as u32 * coverage as u32 + 127) / 255) as u8).max(1)
}
pub struct Buffer {
    pub dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    pub width: i32,
    pub height: i32,
    pub dpi: f32,
    radius: f32,
    cover: Vec<u8>,
    rim: Vec<Rim>,
    fusion_cover: Option<(Vec<crate::fusion::geometry::Pane>, [i32; 4], Vec<u8>)>,
}
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
pub(crate) struct Seam {
    vertical: bool,
    seam: f32,
    span: [f32; 2],
    scale: f32,
    bounds: [i32; 4],
}

pub(crate) fn collect_seams(
    panes: &[crate::fusion::geometry::Pane],
    own: &crate::fusion::geometry::Pane,
    rect: RECT,
    width: i32,
    height: i32,
) -> Vec<Seam> {
    let a = own.rect;
    panes
        .iter()
        .filter(|p| crate::fusion::geometry::adjacent(own, p))
        .map(|p| {
            let b = p.rect;
            let scale = own.dpi.max(p.dpi);
            let reach_n = 140. * scale;
            let reach_t = 140. * scale;
            let corner_pad = own.radius.max(p.radius);
            let vertical = crate::fusion::geometry::touching_edges(a[2], b[0], scale)
                || crate::fusion::geometry::touching_edges(b[2], a[0], scale);
            if vertical {
                let seam = if crate::fusion::geometry::touching_edges(a[2], b[0], scale) {
                    (a[2] + b[0]) * 0.5
                } else {
                    (a[0] + b[2]) * 0.5
                };
                let span = [
                    a[1].max(b[1]) - corner_pad,
                    a[3].min(b[3]) + corner_pad,
                ];
                Seam {
                    vertical: true,
                    seam,
                    span,
                    scale,
                    bounds: [
                        ((seam - reach_n - rect.left as f32).floor() as i32).clamp(0, width),
                        ((span[0] - reach_t - rect.top as f32).floor() as i32).clamp(0, height),
                        ((seam + reach_n - rect.left as f32).ceil() as i32).clamp(0, width),
                        ((span[1] + reach_t - rect.top as f32).ceil() as i32).clamp(0, height),
                    ],
                }
            } else {
                let seam = if crate::fusion::geometry::touching_edges(a[3], b[1], scale) {
                    (a[3] + b[1]) * 0.5
                } else {
                    (a[1] + b[3]) * 0.5
                };
                let span = [
                    a[0].max(b[0]) - corner_pad,
                    a[2].min(b[2]) + corner_pad,
                ];
                Seam {
                    vertical: false,
                    seam,
                    span,
                    scale,
                    bounds: [
                        ((span[0] - reach_t - rect.left as f32).floor() as i32).clamp(0, width),
                        ((seam - reach_n - rect.top as f32).floor() as i32).clamp(0, height),
                        ((span[1] + reach_t - rect.left as f32).ceil() as i32).clamp(0, width),
                        ((seam + reach_n - rect.top as f32).ceil() as i32).clamp(0, height),
                    ],
                }
            }
        })
        .collect()
}

#[inline]
pub(crate) fn seams_bounds(seams: &[Seam]) -> [i32; 4] {
    [
        seams.iter().map(|s| s.bounds[0]).min().unwrap_or(0),
        seams.iter().map(|s| s.bounds[1]).min().unwrap_or(0),
        seams.iter().map(|s| s.bounds[2]).max().unwrap_or(0),
        seams.iter().map(|s| s.bounds[3]).max().unwrap_or(0),
    ]
}

#[inline]
pub(crate) fn seam_weight(seams: &[Seam], x: i32, y: i32, gx: f32, gy: f32) -> f32 {
    let mut weight = 0f32;
    for s in seams {
        if x < s.bounds[0] || x >= s.bounds[2] || y < s.bounds[1] || y >= s.bounds[3] {
            continue;
        }
        let (normal, tangent) = if s.vertical {
            (
                (gx - s.seam).abs(),
                (s.span[0] - gy).max(gy - s.span[1]).max(0.),
            )
        } else {
            (
                (gy - s.seam).abs(),
                (s.span[0] - gx).max(gx - s.span[1]).max(0.),
            )
        };
        weight = weight.max(crate::fusion::geometry::material_weight(
            normal, tangent, s.scale,
        ));
    }
    weight
}

impl Buffer {
    pub fn matches_geometry(&self, width: i32, height: i32, dpi: f32, radius: f32) -> bool {
        self.width == width && self.height == height && self.dpi == dpi && self.radius == radius
    }
    pub unsafe fn snapshot(&self) -> Vec<u8> {
        GdiFlush();
        std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize).to_vec()
    }
    pub unsafe fn pixels_mut(&mut self) -> &mut [u8] {
        GdiFlush();
        std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize)
    }
    /// A nested material made exclusively from this widget's owned background
    /// bitmap. Padding avoids blur seams; a tiny sample shift simulates refraction.
    pub unsafe fn frosted_piece(
        &self,
        r: [f32; 4],
        radius: f32,
        diffusion: f32,
        offset: [f32; 2],
        tint: u32,
    ) -> Option<crate::render::Image> {
        self.frosted_piece_opacity(r, radius, diffusion, offset, tint, 0.5)
    }
    /// Variant of `frosted_piece` with a caller-supplied blend `opacity` in `[0, 1]`
    /// over the undecorated widget surface.
    pub unsafe fn frosted_piece_opacity(
        &self,
        r: [f32; 4],
        radius: f32,
        diffusion: f32,
        offset: [f32; 2],
        tint: u32,
        opacity: f32,
    ) -> Option<crate::render::Image> {
        let opacity = opacity.clamp(0., 1.);
        let low_power = crate::motion::low_power();
        GdiFlush();
        let width = ((r[2] - r[0]) * self.dpi).round().max(1.) as u32;
        let height = ((r[3] - r[1]) * self.dpi).round().max(1.) as u32;
        let sigma = (diffusion * self.dpi).max(0.5);
        let pad = if low_power {
            1
        } else {
            (sigma * 3.).ceil() as u32
        };
        let sw = width + 2 * pad;
        let sh = height + 2 * pad;
        let left = (r[0] * self.dpi + offset[0] * self.dpi).round() as i32 - pad as i32;
        let top = (r[1] * self.dpi + offset[1] * self.dpi).round() as i32 - pad as i32;
        let source = std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize);
        // Sample directly into the half-resolution blur buffer rather than
        // allocating and filtering a full-resolution padded copy each move.
        let small = image::RgbaImage::from_fn(sw.div_ceil(2), sh.div_ceil(2), |x, y| {
            let mut sum = [0u32; 3];
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = (left + x as i32 * 2 + dx).clamp(0, self.width - 1) as usize;
                    let sy = (top + y as i32 * 2 + dy).clamp(0, self.height - 1) as usize;
                    let i = (sy * self.width as usize + sx) * 4;
                    sum[0] += source[i + 2] as u32;
                    sum[1] += source[i + 1] as u32;
                    sum[2] += source[i] as u32;
                }
            }
            image::Rgba([
                (sum[0] / 4) as u8,
                (sum[1] / 4) as u8,
                (sum[2] / 4) as u8,
                255,
            ])
        });
        let blurred = if low_power {
            small
        } else {
            image::imageops::fast_blur(&small, sigma * 0.5)
        };
        let xs: Vec<_> = (0..width)
            .map(|x| {
                let v = (x + pad) as f32 * 0.5 - 0.25;
                let lo = v.floor().max(0.) as u32;
                (lo, (lo + 1).min(blurred.width() - 1), v - v.floor())
            })
            .collect();
        let radius = radius * self.dpi;
        let tint = [
            (tint & 255) as f32,
            ((tint >> 8) & 255) as f32,
            ((tint >> 16) & 255) as f32,
        ];
        let tint_luma = (tint[0] * 0.2126 + tint[1] * 0.7152 + tint[2] * 0.0722).max(1.);
        let tint_gain = tint.map(|c| (tint_luma + (c - tint_luma) * 1.2).max(0.) / tint_luma);
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let alpha = if x as f32 + 0.5 >= radius && (x as f32 + 0.5) < width as f32 - radius
                {
                    255.
                } else {
                    (0.5 - distance(
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                        width as f32,
                        height as f32,
                        radius,
                        0.,
                    ))
                    .clamp(0., 1.)
                        * 255.
                };
                let v = (y + pad) as f32 * 0.5 - 0.25;
                let y0 = v.floor().max(0.) as u32;
                let fy = v - v.floor();
                let y1 = (y0 + 1).min(blurred.height() - 1);
                let (x0, x1, fx) = xs[x as usize];
                let taps = [
                    blurred.get_pixel(x0, y0),
                    blurred.get_pixel(x1, y0),
                    blurred.get_pixel(x0, y1),
                    blurred.get_pixel(x1, y1),
                ];
                let hash = (x / 2).wrapping_mul(374761393) ^ (y / 2).wrapping_mul(668265263);
                let grain = (((hash ^ (hash >> 13)) % 5) as f32 - 2.) * 0.36;
                let mut sample = [0.; 3];
                for c in 0..3 {
                    let top = taps[0][c] as f32 * (1. - fx) + taps[1][c] as f32 * fx;
                    let bottom = taps[2][c] as f32 * (1. - fx) + taps[3][c] as f32 * fx;
                    sample[c] = top * (1. - fy) + bottom * fy;
                }
                // Recess dark wallpaper regions without covering bright ones
                // in a uniform gray film. One luminance gain preserves hue.
                let luma = sample[0] * 0.2126 + sample[1] * 0.7152 + sample[2] * 0.0722;
                let t = ((luma - 64.) / 136.).clamp(0., 1.);
                let gain = 1. - 0.24 * (1. - t * t * (3. - 2. * t));
                for c in 0..3 {
                    // A more saturated accent mixed at the backdrop's own
                    // luminance, so stronger color does not darken highlights.
                    let value = ((sample[c] * 0.76 + luma * tint_gain[c] * 0.24) * gain + grain)
                        .clamp(0., 255.);
                    // Half-opacity material over the undecorated widget.
                    // Keep this image opaque inside the pane so the outer
                    // decorative glow cannot leak into search underneath it.
                    let sx = ((r[0] * self.dpi).round() as i32 + x as i32).clamp(0, self.width - 1);
                    let sy =
                        ((r[1] * self.dpi).round() as i32 + y as i32).clamp(0, self.height - 1);
                    let base = source[((sy * self.width + sx) * 4) as usize + 2 - c] as f32;
                    pixels.push(
                        ((base * (1. - opacity) + value * opacity) * alpha / 255.).round() as u8,
                    );
                }
                pixels.push(alpha.round() as u8);
            }
        }
        crate::render::Image::from_rgba(width, height, pixels)
    }
    pub unsafe fn new(width: i32, height: i32, dpi: f32, radius: f32) -> Option<Self> {
        Self::allocate(width, height, dpi, radius, true)
    }
    pub unsafe fn plain(width: i32, height: i32, dpi: f32) -> Option<Self> {
        Self::allocate(width, height, dpi, 0., false)
    }
    unsafe fn allocate(
        width: i32,
        height: i32,
        dpi: f32,
        radius: f32,
        shape: bool,
    ) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let dc = CreateCompatibleDC(null_mut());
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, bitmap);
        let mut cover = if shape {
            vec![255; (width * height) as usize]
        } else {
            vec![]
        };
        let mut rim = vec![];
        let w = width as f32;
        let h = height as f32;
        let radius = radius * dpi;
        let inset = 4. * dpi;
        let spec_w = liquid_spec_width(dpi, w, h);
        let rim_limit = (6. * dpi).max(spec_w);
        for y in 0..if shape { height } else { 0 } {
            for x in 0..width {
                let fx = x as f32 + 0.5;
                let fy = y as f32 + 0.5;
                if ((fx > inset + radius && fx < w - inset - radius)
                    && (fy > inset + rim_limit && fy < h - inset - rim_limit))
                    || ((fy > inset + radius && fy < h - inset - radius)
                        && (fx > inset + rim_limit && fx < w - inset - rim_limit))
                {
                    continue;
                }
                let d = distance(fx, fy, w, h, radius, inset);
                let mut coverage = (0.5 - d).clamp(0., 1.);
                if d.abs() < 1.
                    && (fx < inset + radius || fx > w - inset - radius)
                    && (fy < inset + radius || fy > h - inset - radius)
                {
                    let sub = 4;
                    let mut inside = 0;
                    for sy in 0..sub {
                        for sx in 0..sub {
                            if distance(
                                x as f32 + (sx as f32 + 0.5) / sub as f32,
                                y as f32 + (sy as f32 + 0.5) / sub as f32,
                                w,
                                h,
                                radius,
                                inset,
                            ) <= 0.
                            {
                                inside += 1;
                            }
                        }
                    }
                    coverage = inside as f32 / (sub * sub) as f32;
                }
                let index = (y * width + x) as usize;
                cover[index] = (coverage * 255.).round() as u8;
                if d <= 0. && d > -rim_limit {
                    let nx = distance(fx + 0.5, fy, w, h, radius, inset)
                        - distance(fx - 0.5, fy, w, h, radius, inset);
                    let ny = distance(fx, fy + 0.5, w, h, radius, inset)
                        - distance(fx, fy - 0.5, w, h, radius, inset);
                    let length = nx.hypot(ny).max(0.001);
                    let nx = nx / length;
                    let ny = ny / length;
                    let liquid_spec = liquid_rim_spec(d, nx, ny, spec_w);
                    let (sample, light) = if d > -6. * dpi {
                        let depth = -d / dpi;
                        let lighting = (-nx * 0.6 - ny * 0.8).clamp(-1., 1.);
                        // One luminous boundary, with glints that fade away from the corners.
                        let tl =
                            (-((fx / (230. * dpi)).powi(2) + (fy / (155. * dpi)).powi(2))).exp();
                        let br = (-(((w - fx) / (170. * dpi)).powi(2)
                            + ((h - fy) / (140. * dpi)).powi(2)))
                        .exp();
                        let light = (0.045
                            + tl * 0.48 * lighting.max(0.)
                            + br * 0.25
                            + 0.09
                                * (-(((w - fx) / (200. * dpi)).powi(2)
                                    + (fy / (150. * dpi)).powi(2)))
                                .exp())
                            * (1. - (depth - 0.55 - 1. / dpi) / (1.05 / dpi)).clamp(0., 1.);
                        let bend = (1. - depth / 6.).max(0.).powi(2) * 2.5 * dpi;
                        let sx = (x as f32 - nx * bend).clamp(0., w - 1.);
                        let sy = (y as f32 - ny * bend).clamp(0., h - 1.);
                        ([sx, sy], light)
                    } else {
                        ([x as f32, y as f32], 0.)
                    };
                    rim.push(Rim {
                        index: (y * width + x) as usize * 4,
                        sample,
                        light,
                        liquid_spec,
                    });
                }
            }
        }
        Some(Self {
            dc,
            bitmap,
            old,
            bits: bits as _,
            width,
            height,
            dpi,
            radius: radius / dpi,
            cover,
            rim,
            fusion_cover: None,
        })
    }
    pub unsafe fn bevel(&mut self) {
        self.bevel_inner(None, false);
    }
    /// Internal seams do not get a second bright rim. The fused signed-distance
    /// outline controls both the bevel and the final antialiased coverage.
    ///
    /// For shared-wallpaper widgets, we publish and blend only the chrome delta
    /// (`chrome - wallpaper`) across fused seams so themes, tints, coatings,
    /// and overlays bleed smoothly across neighbors without blurring the
    /// underlying shared wallpaper.
    pub unsafe fn bevel_for(&mut self, hwnd: HWND) {
        crate::fusion::prepare(hwnd, self.radius);
        let moving = crate::fusion::is_moving(hwnd) || crate::glass::is_resizing(hwnd);
        if crate::fusion::may_fuse() && !moving {
            GdiFlush();
            let source =
                std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize);
            crate::fusion::publish_chrome_for(
                hwnd,
                self.width as usize,
                self.height as usize,
                source,
            );
        }
        let rect = crate::fusion::window_rect(hwnd);
        let group = crate::fusion::group(hwnd);
        if let Some(ref panes) = group {
            if let Some(material) =
                crate::fusion::shared_material(hwnd, self.width, self.height, self.dpi)
            {
                GdiFlush();
                let pixels = std::slice::from_raw_parts_mut(
                    self.bits,
                    (self.width * self.height * 4) as usize,
                );
                if let Some(own) = panes.iter().find(|p| p.id == hwnd as usize) {
                    let seams = collect_seams(panes, own, rect, self.width, self.height);
                    let [x_min, y_min, x_max, y_max] = seams_bounds(&seams);
                    for y in y_min..y_max {
                        let gy = rect.top as f32 + y as f32 + 0.5;
                        for x in x_min..x_max {
                            let gx = rect.left as f32 + x as f32 + 0.5;
                            let weight = seam_weight(&seams, x, y, gx, gy);
                            if weight > 0. {
                                let i = ((y * self.width + x) * 4) as usize;
                                let color = material.sample(gx, gy);
                                if material.is_delta {
                                    for c in 0..3 {
                                        pixels[i + c] = (pixels[i + c] as f32 + color[c] * weight)
                                            .round()
                                            .clamp(0., 255.)
                                            as u8;
                                    }
                                } else {
                                    for c in 0..3 {
                                        pixels[i + c] = (pixels[i + c] as f32 * (1. - weight)
                                            + color[c] * weight)
                                            .round()
                                            as u8;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.bevel_inner(
            group.map(|p| (p, [rect.left as f32, rect.top as f32])),
            moving,
        );
    }
    pub unsafe fn paint_frosted_chrome_for(
        &mut self,
        hwnd: HWND,
        frosted: bool,
        draw_chrome: impl FnOnce(HDC),
    ) {
        crate::fusion::prepare(hwnd, self.radius);
        if !frosted || !crate::fusion::frosted_wallpaper_active() {
            draw_chrome(self.dc);
            self.bevel_for(hwnd);
            return;
        }
        thread_local! {
            static OVERLAY_SCRATCH: std::cell::RefCell<Option<Buffer>> = const { std::cell::RefCell::new(None) };
        }
        let overlay = OVERLAY_SCRATCH
            .with(|cell| {
                let mut slot = cell.borrow_mut();
                let reusable = slot.as_ref().is_some_and(|b| {
                    b.width == self.width
                        && b.height == self.height
                        && (b.dpi - self.dpi).abs() < 1e-4
                });
                if reusable {
                    slot.take()
                } else {
                    None
                }
            })
            .or_else(|| Buffer::plain(self.width, self.height, self.dpi));
        let Some(overlay) = overlay else {
            draw_chrome(self.dc);
            self.bevel_for(hwnd);
            return;
        };
        let byte_len = (self.width * self.height * 4) as usize;
        std::ptr::write_bytes(overlay.bits, 0, byte_len);
        {
            let alpha_batch = crate::render::AlphaBatch::new(overlay.dc, self.dpi);
            draw_chrome(overlay.dc);
            drop(alpha_batch);
            GdiFlush();
        }
        let moving = crate::fusion::is_moving(hwnd) || crate::glass::is_resizing(hwnd);
        let group = crate::fusion::group(hwnd);
        let inset_px = if group.is_some() {
            1
        } else {
            (4.0 * self.dpi).ceil() as usize
        };
        let overlay_px = std::slice::from_raw_parts(overlay.bits, byte_len);
        if crate::fusion::may_fuse() && !moving {
            crate::fusion::publish_overlay_for(
                hwnd,
                self.width as usize,
                self.height as usize,
                overlay_px,
                inset_px,
            );
        }
        let rect = crate::fusion::window_rect(hwnd);
        let shared = group
            .as_ref()
            .and_then(|_| crate::fusion::shared_overlay(hwnd, self.width, self.height));
        GdiFlush();
        let pixels = std::slice::from_raw_parts_mut(self.bits, byte_len);
        let seam_field = shared.as_ref().and_then(|field| {
            let panes = group.as_ref()?;
            let own = panes.iter().find(|p| p.id == hwnd as usize)?;
            let seams = collect_seams(panes, own, rect, self.width, self.height);
            if seams.is_empty() {
                return None;
            }
            let [x_min, y_min, x_max, y_max] = seams_bounds(&seams);
            Some((field, seams, x_min, y_min, x_max, y_max))
        });
        for y in 0..self.height {
            let gy = rect.top as f32 + y as f32 + 0.5;
            let in_y = seam_field
                .as_ref()
                .is_some_and(|(_, _, _, y_min, _, y_max)| y >= *y_min && y < *y_max);
            for x in 0..self.width {
                let i = ((y * self.width + x) * 4) as usize;
                let weight = if in_y {
                    if let Some((_, seams, x_min, _, x_max, _)) = &seam_field {
                        if x >= *x_min && x < *x_max {
                            let gx = rect.left as f32 + x as f32 + 0.5;
                            seam_weight(seams, x, y, gx, gy)
                        } else {
                            0.0
                        }
                    } else {
                        0.0
                    }
                } else {
                    0.0
                };
                if weight > 0.0 {
                    let (field, _, _, _, _, _) = seam_field.as_ref().unwrap();
                    let gx = rect.left as f32 + x as f32 + 0.5;
                    let s = field.sample(gx, gy);
                    let ov = std::array::from_fn::<f32, 4, _>(|c| {
                        overlay_px[i + c] as f32 * (1.0 - weight) + s[c] * weight
                    });
                    let keep = (1.0 - ov[3] / 255.0).clamp(0.0, 1.0);
                    for c in 0..3 {
                        pixels[i + c] =
                            (pixels[i + c] as f32 * keep + ov[c]).round().clamp(0.0, 255.0) as u8;
                    }
                } else {
                    let oa = overlay_px[i + 3] as u32;
                    if oa > 0 {
                        let inv = 255 - oa;
                        for c in 0..3 {
                            pixels[i + c] = (overlay_px[i + c] as u32
                                + (pixels[i + c] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                        }
                    }
                }
            }
        }
        if !(crate::motion::low_power() && byte_len > 4 * 1024 * 1024) {
            OVERLAY_SCRATCH.with(|cell| {
                *cell.borrow_mut() = Some(overlay);
            });
        }
        self.bevel_inner(
            group.map(|p| (p, [rect.left as f32, rect.top as f32])),
            moving,
        );
    }
    unsafe fn bevel_inner(
        &mut self,
        group: Option<(Vec<crate::fusion::geometry::Pane>, [f32; 2])>,
        moving: bool,
    ) {
        let skip_rim = group.is_none() && moving && crate::motion::low_power();
        if skip_rim {
            return;
        }
        GdiFlush();
        let bytes = std::slice::from_raw_parts_mut(
            self.bits,
            self.width as usize * self.height as usize * 4,
        );
        let liquid = crate::glass::liquid_glass();
        if let Some((ref panes, origin)) = group {
            // Compute fused cluster bounding box across the panes (+ 48.0 * dpi padding)
            let mut min_x = f32::MAX;
            let mut min_y = f32::MAX;
            let mut max_x = f32::MIN;
            let mut max_y = f32::MIN;
            for p in panes {
                min_x = min_x.min(p.rect[0]);
                min_y = min_y.min(p.rect[1]);
                max_x = max_x.max(p.rect[2]);
                max_y = max_y.max(p.rect[3]);
            }
            let pad = 48.0 * self.dpi;
            let cluster_w = (max_x - min_x + pad * 2.0).max(1.0);
            let cluster_h = (max_y - min_y + pad * 2.0).max(1.0);

            // The new concave boundary can pass through an old transparent
            // corner, so it must not reuse just the old rounded-rect rim list.
            let w = self.width;
            let h = self.height;
            let dpi = self.dpi;
            let spec_w = liquid_spec_width(dpi, cluster_w, cluster_h);
            let band = (self.radius + 10.) * dpi + if liquid { spec_w } else { 0. };
            let min_d = if liquid { -spec_w } else { -2.5 * dpi };
            crate::fusion::geometry::with_outline(panes, |outline| {
                for y in 0..h {
                    let gy = origin[1] + y as f32 + 0.5;
                    let in_y_mid = y as f32 > band && ((h - y) as f32) > band;
                    for x in 0..w {
                        if in_y_mid && x as f32 > band && ((w - x) as f32) > band {
                            continue;
                        }
                        let gx = origin[0] + x as f32 + 0.5;
                        let skip_margin = if liquid { spec_w / dpi } else { 4. };
                        if crate::fusion::geometry::covered_core_within(panes, gx, gy, skip_margin) {
                            continue;
                        }
                        let d = outline.distance([gx, gy]);
                        if d < min_d || d > 1. {
                            continue;
                        }
                        let light = if d >= -2.5 * dpi {
                            outline.rim_light(gx, gy, d, dpi)
                        } else {
                            0.
                        };
                        let spec = if liquid && -d < spec_w {
                            let dx =
                                outline.distance([gx + 1., gy]) - outline.distance([gx - 1., gy]);
                            let dy =
                                outline.distance([gx, gy + 1.]) - outline.distance([gx, gy - 1.]);
                            let len = (dx * dx + dy * dy).sqrt().max(1e-5);
                            liquid_rim_spec(d, dx / len, dy / len, spec_w)
                        } else {
                            0.
                        };
                        let i = ((y * w + x) * 4) as usize;
                        for c in 0..3 {
                            let v = bytes[i + c] as f32;
                            let lit = v + (255. - v) * light + spec;
                            bytes[i + c] = lit.min(255.) as u8;
                        }
                    }
                }
            });
            return;
        }
        thread_local! {
            static BEVEL_SCRATCH: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        BEVEL_SCRATCH.with(|scratch| {
            let mut source = scratch.borrow_mut();
            source.clear();
            source.extend_from_slice(bytes);
            for rim in &self.rim {
                let spec = if liquid { rim.liquid_spec } else { 0. };
                if rim.light == 0. && spec == 0. {
                    continue;
                }
                if rim.light == 0. {
                    for channel in 0..3 {
                        let v = source[rim.index + channel] as f32 + spec;
                        bytes[rim.index + channel] = v.min(255.) as u8;
                    }
                    continue;
                }
                let [x, y] = rim.sample;
                let x0 = x as usize;
                let y0 = y as usize;
                let x1 = (x0 + 1).min(self.width as usize - 1);
                let y1 = (y0 + 1).min(self.height as usize - 1);
                let fx = x - x0 as f32;
                let fy = y - y0 as f32;
                for channel in 0..3 {
                    let at = |x: usize, y: usize| {
                        source[(y * self.width as usize + x) * 4 + channel] as f32
                    };
                    let sample = (at(x0, y0) * (1. - fx) + at(x1, y0) * fx) * (1. - fy)
                        + (at(x0, y1) * (1. - fx) + at(x1, y1) * fx) * fy;
                    let lit = if rim.light > 0. {
                        sample + (255. - sample) * rim.light
                    } else {
                        sample * (1. + rim.light)
                    };
                    bytes[rim.index + channel] = (lit + spec).clamp(0., 255.) as u8;
                }
            }
            if crate::motion::low_power() && source.capacity() > 1024 * 1024 {
                *source = Vec::new();
            }
        });
    }
    pub unsafe fn copy_scrolled_vertical(
        &mut self,
        cache: &Self,
        clip_top: i32,
        clip_bottom: i32,
        scroll_px: f32,
    ) {
        if self.width != cache.width || clip_top >= clip_bottom {
            return;
        }
        GdiFlush();
        let stride = self.width as usize * 4;
        let dst_pixels =
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize);
        let src_pixels =
            std::slice::from_raw_parts(cache.bits, (cache.width * cache.height * 4) as usize);
        let y0_src = scroll_px.floor() as i32;
        let fy = ((scroll_px - y0_src as f32) * 256.).round().clamp(0., 256.) as u32;
        let wy0 = 256 - fy;
        let wy1 = fy;
        let top = clip_top.max(0);
        let bottom = clip_bottom.min(self.height);
        for dst_y in top..bottom {
            let src_y = y0_src + (dst_y - clip_top);
            if src_y < 0 || src_y >= cache.height {
                continue;
            }
            let dst_row = &mut dst_pixels[dst_y as usize * stride..(dst_y as usize + 1) * stride];
            let src_row0 = &src_pixels[src_y as usize * stride..(src_y as usize + 1) * stride];
            if fy == 0 || src_y + 1 >= cache.height {
                dst_row.copy_from_slice(src_row0);
            } else {
                let src_row1 =
                    &src_pixels[(src_y as usize + 1) * stride..(src_y as usize + 2) * stride];
                for ((d, s0), s1) in dst_row.iter_mut().zip(src_row0).zip(src_row1) {
                    *d = ((*s0 as u32 * wy0 + *s1 as u32 * wy1 + 128) >> 8) as u8;
                }
            }
        }
    }
    pub unsafe fn copy_from(&self, source: HDC) {
        BitBlt(
            self.dc,
            0,
            0,
            self.width,
            self.height,
            source,
            0,
            0,
            SRCCOPY,
        );
    }
    pub unsafe fn fade_list(&mut self, base: &Self, top: f32, bottom: f32) {
        GdiFlush();
        let size = (self.width * self.height * 4) as usize;
        let pixels = std::slice::from_raw_parts_mut(self.bits, size);
        let background = std::slice::from_raw_parts(base.bits, size);
        let stride = self.width as usize * 4;
        let min_y = ((top * self.dpi).floor() as i32 - 2).max(0) as usize;
        let max_y = ((bottom * self.dpi).ceil() as i32 + 2).min(self.height) as usize;
        if min_y > 0 {
            let end = (min_y * stride).min(size);
            pixels[..end].copy_from_slice(&background[..end]);
        }
        if max_y < self.height as usize {
            let start = (max_y * stride).min(size);
            pixels[start..size].copy_from_slice(&background[start..size]);
        }
        for y in min_y..max_y {
            let dip = (y as f32 + 0.5) / self.dpi;
            let t = ((dip - top) / 14.).min((bottom - dip) / 26.).clamp(0., 1.);
            if t >= 1. {
                continue;
            }
            let alpha = t * t * (3. - 2. * t);
            let wa = (alpha * 256.).round() as u32;
            let row_start = y * stride;
            let row_end = row_start + stride;
            if wa == 0 {
                pixels[row_start..row_end].copy_from_slice(&background[row_start..row_end]);
                continue;
            }
            if wa >= 256 {
                continue;
            }
            let wb = 256 - wa;
            for (px, bg) in pixels[row_start..row_end]
                .iter_mut()
                .zip(&background[row_start..row_end])
            {
                *px = ((*bg as u32 * wb + *px as u32 * wa + 128) >> 8) as u8;
            }
        }
    }
    pub unsafe fn fill_round(&mut self, r: [f32; 4], radius: f32, color: u32, alpha: u8) {
        if alpha == 0 {
            return;
        }
        GdiFlush();
        let [l, t, right, b] = r.map(|v| v * self.dpi);
        let rad = radius * self.dpi;
        let x0 = (l.floor() as i32 - 1).max(0);
        let y0 = (t.floor() as i32 - 1).max(0);
        let x1 = (right.ceil() as i32 + 1).min(self.width);
        let y1 = (b.ceil() as i32 + 1).min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let cr = (color & 0xff) as u32;
        let cg = ((color >> 8) & 0xff) as u32;
        let cb = ((color >> 16) & 0xff) as u32;
        let pixels =
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize);
        let w = right - l;
        let h = b - t;
        for y in y0..y1 {
            let py = y as f32 + 0.5 - t;
            for x in x0..x1 {
                let px = x as f32 + 0.5 - l;
                let cov = (0.5 - distance(px, py, w, h, rad, 0.)).clamp(0., 1.);
                let a = (alpha as f32 * cov).round() as u32;
                if a == 0 {
                    continue;
                }
                let i = ((y * self.width + x) * 4) as usize;
                let inv = 255 - a;
                pixels[i] = ((cb * a + pixels[i] as u32 * inv + 127) / 255).min(255) as u8;
                pixels[i + 1] = ((cg * a + pixels[i + 1] as u32 * inv + 127) / 255).min(255) as u8;
                pixels[i + 2] = ((cr * a + pixels[i + 2] as u32 * inv + 127) / 255).min(255) as u8;
            }
        }
    }
    /// Dock-style smooth edges confined to a widget's scrolling viewport.
    #[cfg(test)]
    pub unsafe fn fade_list_region(&mut self, base: &Self, top: f32, bottom: f32) {
        self.fade_list_edges(base, top, bottom, true, true);
    }
    pub unsafe fn fade_list_edges(
        &mut self,
        base: &Self,
        top: f32,
        bottom: f32,
        fade_top: bool,
        fade_bottom: bool,
    ) {
        if !fade_top && !fade_bottom {
            return;
        }
        GdiFlush();
        let size = (self.width * self.height * 4) as usize;
        let pixels = std::slice::from_raw_parts_mut(self.bits, size);
        let background = std::slice::from_raw_parts(base.bits, size);
        let stride = self.width as usize * 4;
        let y0 = ((top * self.dpi).floor() as i32).max(0) as usize;
        let y1 = ((bottom * self.dpi).ceil() as i32).min(self.height) as usize;
        for y in y0..y1 {
            let dip = (y as f32 + 0.5) / self.dpi;
            if dip < top || dip > bottom {
                continue;
            }
            let t = (if fade_top { (dip - top) / 10. } else { 1. })
                .min(if fade_bottom {
                    (bottom - dip) / 18.
                } else {
                    1.
                })
                .clamp(0., 1.);
            if t >= 1. {
                continue;
            }
            let a = t * t * (3. - 2. * t);
            let wa = (a * 256.).round() as u32;
            if wa >= 256 {
                continue;
            }
            let wb = 256 - wa;
            let row_start = y * stride;
            let row_end = row_start + stride;
            for (px, bg) in pixels[row_start..row_end]
                .iter_mut()
                .zip(&background[row_start..row_end])
            {
                *px = ((*bg as u32 * wb + *px as u32 * wa + 128) >> 8) as u8;
            }
        }
    }
    pub unsafe fn fade_horizontal_edges(
        &mut self,
        base: &Self,
        r: [f32; 4],
        fade_left: bool,
        fade_right: bool,
    ) {
        if !fade_left && !fade_right {
            return;
        }
        GdiFlush();
        let [left, top, right, bottom] = r;
        let pixels =
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize);
        let bg = std::slice::from_raw_parts(base.bits, (base.width * base.height * 4) as usize);
        let y0 = ((top * self.dpi).floor() as i32).max(0);
        let y1 = ((bottom * self.dpi).ceil() as i32).min(self.height);
        let x0 = ((left * self.dpi).floor() as i32).max(0);
        let x1 = ((right * self.dpi).ceil() as i32).min(self.width);
        if x0 >= x1 || y0 >= y1 {
            return;
        }
        let mut fade_cols = Vec::new();
        for x in x0..x1 {
            let px = (x as f32 + 0.5) / self.dpi;
            if px < left || px > right {
                continue;
            }
            let t = (if fade_left { (px - left) / 18. } else { 1. })
                .min(if fade_right { (right - px) / 18. } else { 1. })
                .clamp(0., 1.);
            if t >= 1. {
                continue;
            }
            let a = t * t * (3. - 2. * t);
            let wa = (a * 256.).round() as u32;
            if wa < 256 {
                fade_cols.push((x as usize * 4, wa, 256 - wa));
            }
        }
        if fade_cols.is_empty() {
            return;
        }
        let stride = self.width as usize * 4;
        for y in y0..y1 {
            let py = (y as f32 + 0.5) / self.dpi;
            if py < top || py > bottom {
                continue;
            }
            let row = y as usize * stride;
            for &(col_byte, wa, wb) in &fade_cols {
                let i = row + col_byte;
                for c in 0..4 {
                    pixels[i + c] =
                        ((bg[i + c] as u32 * wb + pixels[i + c] as u32 * wa + 128) >> 8) as u8;
                }
            }
        }
    }
    pub unsafe fn fade_control_region(&mut self, base: &Self, r: [f32; 4], amount: f32) {
        GdiFlush();
        let a = amount.clamp(0., 1.);
        if a >= 1. {
            return;
        }
        let wa = (a * 256.).round() as u32;
        if wa >= 256 {
            return;
        }
        let wb = 256 - wa;
        let pixels =
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize);
        let bg = std::slice::from_raw_parts(base.bits, (base.width * base.height * 4) as usize);
        let [l, t, r, b] = r.map(|v| (v * self.dpi) as i32);
        for y in t.max(0)..b.min(self.height) {
            for x in l.max(0)..r.min(self.width) {
                let i = ((y * self.width + x) * 4) as usize;
                for c in 0..4 {
                    pixels[i + c] =
                        ((bg[i + c] as u32 * wb + pixels[i + c] as u32 * wa + 128) >> 8) as u8;
                }
            }
        }
    }
    pub unsafe fn present(&mut self, hwnd: HWND) -> bool {
        self.present_with_alpha(hwnd, false)
    }
    pub unsafe fn background_alpha(&mut self, alpha: u8) {
        GdiFlush();
        let pixels =
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize);
        for p in pixels.chunks_exact_mut(4) {
            for c in &mut p[..3] {
                *c = ((*c as u32 * alpha as u32 + 127) / 255) as u8;
            }
            p[3] = alpha;
        }
    }
    /// Apply background alpha while smoothly blending across fused seams when
    /// neighboring widgets have different opacities.
    pub unsafe fn background_alpha_for(&mut self, hwnd: HWND, alpha: u8) {
        crate::fusion::opacity(hwnd, alpha);
        let Some((panes, opacities)) = crate::fusion::group_opacities(hwnd, alpha) else {
            self.background_alpha(alpha);
            return;
        };
        let r = crate::fusion::window_rect(hwnd);
        let reach = 120. * self.dpi;
        let own_id = hwnd as usize;
        let mut bx0 = self.width;
        let mut by0 = self.height;
        let mut bx1 = 0;
        let mut by1 = 0;
        for (p, &op) in panes.iter().zip(&opacities) {
            if p.id == own_id || op == alpha {
                continue;
            }
            bx0 = bx0.min(((p.rect[0] - reach - r.left as f32).floor() as i32).max(0));
            by0 = by0.min(((p.rect[1] - reach - r.top as f32).floor() as i32).max(0));
            bx1 = bx1.max(((p.rect[2] + reach - r.left as f32).ceil() as i32).min(self.width));
            by1 = by1.max(((p.rect[3] + reach - r.top as f32).ceil() as i32).min(self.height));
        }
        if bx0 >= bx1 || by0 >= by1 {
            self.background_alpha(alpha);
            return;
        }
        GdiFlush();
        let pixels =
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize);
        let alpha_u = alpha as u32;
        let w = self.width;
        for y in 0..self.height {
            let row = (y * w) as usize;
            let in_y = y >= by0 && y < by1;
            let gy = r.top as f32 + y as f32 + 0.5;
            for x in 0..w {
                let i = (row + x as usize) * 4;
                let a = if in_y && x >= bx0 && x < bx1 {
                    let gx = r.left as f32 + x as f32 + 0.5;
                    let mut sum_w = 0.0f32;
                    let mut sum_a = 0.0f32;
                    for (p, &op) in panes.iter().zip(&opacities) {
                        let [l, t, r_pane, b] = p.rect;
                        let rad = (4.0 * p.dpi).min((r_pane - l).min(b - t) * 0.5);
                        let d = distance(gx - l, gy - t, r_pane - l, b - t, rad, 0.).max(0.);
                        if d < reach {
                            let t = d / reach;
                            let weight = (1. - t) * (1. - t);
                            sum_w += weight;
                            sum_a += weight * op as f32;
                        }
                    }
                    if sum_w > 0. {
                        (sum_a / sum_w).round().clamp(0., 255.) as u32
                    } else {
                        alpha_u
                    }
                } else {
                    alpha_u
                };
                if a == 255 {
                    pixels[i + 3] = 255;
                } else if a == 0 {
                    pixels[i..i + 4].fill(0);
                } else {
                    for c in 0..3 {
                        pixels[i + c] = ((pixels[i + c] as u32 * a + 127) / 255) as u8;
                    }
                    pixels[i + 3] = a as u8;
                }
            }
        }
    }
    /// Restore an independent glass card from the owned, fully opaque backing.
    /// No desktop capture and no per-card bitmap allocation.
    pub unsafe fn restore_glass_card(&self, dc: HDC, r: [f32; 4], radius: f32) {
        self.restore_glass_card_impl(dc, r, radius, true);
    }
    /// Fast rounded-card restore without per-pixel liquid refraction (for active scroll/motion).
    pub unsafe fn restore_glass_card_fast(&self, dc: HDC, r: [f32; 4], radius: f32) {
        self.restore_glass_card_impl(dc, r, radius, false);
    }
    unsafe fn restore_glass_card_impl(
        &self,
        dc: HDC,
        r: [f32; 4],
        radius: f32,
        allow_liquid: bool,
    ) {
        crate::render::flush(dc);
        let mut dib: DIBSECTION = zeroed();
        if GetObjectW(
            GetCurrentObject(dc, OBJ_BITMAP as u32),
            size_of::<DIBSECTION>() as i32,
            &mut dib as *mut _ as _,
        ) == 0
            || dib.dsBm.bmBits.is_null()
        {
            return;
        }
        let mut clip: RECT = zeroed();
        GetClipBox(dc, &mut clip);
        let width = dib.dsBm.bmWidth.min(self.width);
        let height = dib.dsBm.bmHeight.abs().min(self.height);
        let dest = std::slice::from_raw_parts_mut(
            dib.dsBm.bmBits as *mut u8,
            (dib.dsBm.bmWidth * dib.dsBm.bmHeight.abs() * 4) as usize,
        );
        let source = std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize);
        let [left, top, right, bottom] = r.map(|v| v * self.dpi);
        let card_w = (right - left).max(1.0);
        let card_h = (bottom - top).max(1.0);
        let min_dim = card_w.min(card_h);
        let radius = (radius * self.dpi).min(min_dim * 0.5);
        let liquid = allow_liquid && crate::glass::liquid_glass() && min_dim >= 16.0;
        let hw = card_w * 0.5;
        let hh = card_h * 0.5;
        let rx = (hw - radius).max(0.0);
        let ry = (hh - radius).max(0.0);
        let rim = (20.0 * self.dpi).clamp(8.0, min_dim * 0.42);
        let bend = 10.0 * self.dpi;
        let ca_strength = 3.5 * self.dpi;
        let y_start = (top.floor() as i32).max(0).max(clip.top);
        let y_end = (bottom.ceil() as i32).min(height).min(clip.bottom);
        let x_start = (left.floor() as i32).max(0).max(clip.left);
        let x_end = (right.ceil() as i32).min(width).min(clip.right);
        if y_start >= y_end || x_start >= x_end {
            return;
        }
        let corner_pad = radius.max(rim) + 2.0;
        let iy0 = ((top + corner_pad - 0.5).ceil() as i32).max(y_start);
        let iy1 = ((top + card_h - corner_pad - 0.5).floor() as i32 + 1).min(y_end);
        let ix0 = ((left + corner_pad - 0.5).ceil() as i32).max(x_start);
        let ix1 = ((left + card_w - corner_pad - 0.5).floor() as i32 + 1).min(x_end);
        let has_interior = iy0 < iy1 && ix0 < ix1;
        let dest_stride = dib.dsBm.bmWidth;
        let src_stride = self.width;
        let source_h = self.height as usize;
        let dpi = self.dpi;
        let blend_span = |dest: &mut [u8], y: i32, x0: i32, x1: i32| {
            let fy = y as f32 + 0.5 - top;
            let src_row = y * src_stride;
            for x in x0..x1 {
                let fx = x as f32 + 0.5 - left;
                let (d, nx, ny) = if liquid {
                    crate::glass::rounded_rect_sdf(fx, fy, hw, hh, rx, ry, radius)
                } else {
                    (distance(fx, fy, card_w, card_h, radius, 0.), 0.0, 0.0)
                };
                let alpha = ((0.5 - d).clamp(0., 1.) * 255.).round() as u32;
                if alpha == 0 {
                    continue;
                }
                let dst = (x * 4) as usize;
                let pixel = if liquid && d > -rim && d <= 0.5 {
                    crate::glass::refract_bgra_card_pixel(
                        source,
                        src_stride as usize,
                        source_h,
                        x as f32,
                        y as f32,
                        d,
                        nx,
                        ny,
                        rim,
                        bend,
                        ca_strength,
                        dpi,
                    )
                } else {
                    let src = ((src_row + x) * 4) as usize;
                    [source[src], source[src + 1], source[src + 2]]
                };
                if alpha == 255 {
                    dest[dst] = pixel[0];
                    dest[dst + 1] = pixel[1];
                    dest[dst + 2] = pixel[2];
                    dest[dst + 3] = 255;
                } else {
                    let inv = 255 - alpha;
                    for c in 0..3 {
                        dest[dst + c] =
                            ((pixel[c] as u32 * alpha + dest[dst + c] as u32 * inv + 127) / 255)
                                as u8;
                    }
                    dest[dst + 3] = ((255 * alpha + dest[dst + 3] as u32 * inv + 127) / 255) as u8;
                }
            }
        };
        let render_rows = |rows: Vec<(usize, &mut [u8])>| {
            for (y, dest) in rows {
                let y = y as i32;
                if has_interior && y >= iy0 && y < iy1 {
                    if x_start < ix0 {
                        blend_span(dest, y, x_start, ix0);
                    }
                    let dst_start = (ix0 * 4) as usize;
                    let dst_end = (ix1 * 4) as usize;
                    let src_start = ((y * src_stride + ix0) * 4) as usize;
                    let src_end = ((y * src_stride + ix1) * 4) as usize;
                    for (d, s) in dest[dst_start..dst_end]
                        .chunks_exact_mut(4)
                        .zip(source[src_start..src_end].chunks_exact(4))
                    {
                        d[0] = s[0];
                        d[1] = s[1];
                        d[2] = s[2];
                        d[3] = 255;
                    }
                    if ix1 < x_end {
                        blend_span(dest, y, ix1, x_end);
                    }
                } else {
                    blend_span(dest, y, x_start, x_end);
                }
            }
        };
        let work_pixels = if liquid { ((x_end - x_start) * (y_end - y_start)) as usize } else { 0 };
        crate::glass::with_pixel_rows_parallel(
            dest, dest_stride as usize * 4, y_start as usize..y_end as usize,
            work_pixels, 32_768, render_rows,
        );
    }
    /// Restore a multi-row connected group card seamlessly from the owned backing.
    pub unsafe fn restore_glass_group(
        &self,
        dc: HDC,
        spans: &[[f32; 4]],
        radius: f32,
        allow_liquid: bool,
    ) {
        if spans.is_empty() {
            return;
        }
        if spans.len() == 1 {
            self.restore_glass_card_impl(dc, spans[0], radius, allow_liquid);
            return;
        }
        let verts_dip = crate::render::ortho_group_polygon(spans);
        if verts_dip.len() < 4 {
            return;
        }
        let verts: Vec<[f32; 2]> = verts_dip
            .iter()
            .map(|p| [p[0] * self.dpi, p[1] * self.dpi])
            .collect();
        if verts.len() == 4 {
            let mut rect = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
            for v in &verts_dip {
                rect[0] = rect[0].min(v[0]);
                rect[1] = rect[1].min(v[1]);
                rect[2] = rect[2].max(v[0]);
                rect[3] = rect[3].max(v[1]);
            }
            self.restore_glass_card_impl(dc, rect, radius, allow_liquid);
            return;
        }
        crate::render::flush(dc);
        let mut dib: DIBSECTION = zeroed();
        if GetObjectW(
            GetCurrentObject(dc, OBJ_BITMAP as u32),
            size_of::<DIBSECTION>() as i32,
            &mut dib as *mut _ as _,
        ) == 0
            || dib.dsBm.bmBits.is_null()
        {
            return;
        }
        let mut clip: RECT = zeroed();
        GetClipBox(dc, &mut clip);
        let width = dib.dsBm.bmWidth.min(self.width);
        let height = dib.dsBm.bmHeight.abs().min(self.height);
        let dest = std::slice::from_raw_parts_mut(
            dib.dsBm.bmBits as *mut u8,
            (dib.dsBm.bmWidth * dib.dsBm.bmHeight.abs() * 4) as usize,
        );
        let source = std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize);
        let sdf = PolySdf::new(&verts, radius * self.dpi);
        let [left, top, right, bottom] = sdf.bounds;
        let min_span = spans
            .iter()
            .map(|s| ((s[2] - s[0]).min(s[3] - s[1])) * self.dpi)
            .fold(f32::INFINITY, f32::min)
            .max(1.0);
        let liquid = allow_liquid && crate::glass::liquid_glass() && min_span >= 16.0;
        let rim = (20.0 * self.dpi).clamp(8.0, min_span * 0.42);
        let bend = 10.0 * self.dpi;
        let ca_strength = 3.5 * self.dpi;
        let corner_pad = sdf.max_radius.max(rim) + 2.0;
        let y_start = (top.floor() as i32).max(0).max(clip.top);
        let y_end = (bottom.ceil() as i32).min(height).min(clip.bottom);
        let x_start = (left.floor() as i32).max(0).max(clip.left);
        let x_end = (right.ceil() as i32).min(width).min(clip.right);
        if y_start >= y_end || x_start >= x_end {
            return;
        }
        let horiz_ys: Vec<f32> = sdf.segments.iter().filter(|s| s.horiz).map(|s| s.a[1]).collect();
        let dest_stride = dib.dsBm.bmWidth;
        let src_stride = self.width;
        let source_h = self.height as usize;
        let dpi = self.dpi;
        let blend_span = |dest: &mut [u8], y: i32, x0: i32, x1: i32| {
            let fy = y as f32 + 0.5;
            let src_row = y * src_stride;
            for x in x0..x1 {
                let fx = x as f32 + 0.5;
                let (d, nx, ny) = sdf.signed_distance(fx, fy);
                let alpha = ((0.5 - d).clamp(0., 1.) * 255.).round() as u32;
                if alpha == 0 {
                    continue;
                }
                let dst = (x * 4) as usize;
                let pixel = if liquid && d > -rim && d <= 0.5 {
                    crate::glass::refract_bgra_card_pixel(
                        source,
                        src_stride as usize,
                        source_h,
                        x as f32,
                        y as f32,
                        d,
                        nx,
                        ny,
                        rim,
                        bend,
                        ca_strength,
                        dpi,
                    )
                } else {
                    let src = ((src_row + x) * 4) as usize;
                    [source[src], source[src + 1], source[src + 2]]
                };
                if alpha == 255 {
                    dest[dst] = pixel[0];
                    dest[dst + 1] = pixel[1];
                    dest[dst + 2] = pixel[2];
                    dest[dst + 3] = 255;
                } else {
                    let inv = 255 - alpha;
                    for c in 0..3 {
                        dest[dst + c] =
                            ((pixel[c] as u32 * alpha + dest[dst + c] as u32 * inv + 127) / 255)
                                as u8;
                    }
                    dest[dst + 3] = ((255 * alpha + dest[dst + 3] as u32 * inv + 127) / 255) as u8;
                }
            }
        };
        let render_rows = |rows: Vec<(usize, &mut [u8])>| {
            let mut xs = Vec::with_capacity(8);
            for (y, dest) in rows {
                let y = y as i32;
                let fy = y as f32 + 0.5;
                let near_horiz = horiz_ys.iter().any(|hy| (fy - *hy).abs() <= corner_pad);
                if !near_horiz {
                    sdf.interior_x_spans(fy, &mut xs);
                    for pair in xs.chunks_exact(2) {
                        let lx0 = (pair[0].floor() as i32 - 2).max(x_start);
                        let lx1 = (pair[1].ceil() as i32 + 2).min(x_end);
                        if lx0 >= lx1 {
                            continue;
                        }
                        let ix0 = ((pair[0] + corner_pad).ceil() as i32).max(lx0);
                        let ix1 = ((pair[1] - corner_pad).floor() as i32).min(lx1);
                        if ix0 < ix1 {
                            if lx0 < ix0 {
                                blend_span(dest, y, lx0, ix0);
                            }
                            let dst_start = (ix0 * 4) as usize;
                            let dst_end = (ix1 * 4) as usize;
                            let src_start = ((y * src_stride + ix0) * 4) as usize;
                            let src_end = ((y * src_stride + ix1) * 4) as usize;
                            for (d, s) in dest[dst_start..dst_end]
                                .chunks_exact_mut(4)
                                .zip(source[src_start..src_end].chunks_exact(4))
                            {
                                d[0] = s[0];
                                d[1] = s[1];
                                d[2] = s[2];
                                d[3] = 255;
                            }
                            if ix1 < lx1 {
                                blend_span(dest, y, ix1, lx1);
                            }
                        } else {
                            blend_span(dest, y, lx0, lx1);
                        }
                    }
                } else {
                    blend_span(dest, y, x_start, x_end);
                }
            }
        };
        let work_pixels = if liquid { ((x_end - x_start) * (y_end - y_start)) as usize } else { 0 };
        crate::glass::with_pixel_rows_parallel(
            dest, dest_stride as usize * 4, y_start as usize..y_end as usize,
            work_pixels, 32_768, render_rows,
        );
    }
    /// Composite opaque chrome UNDER already drawn controls. Filling only the
    /// missing alpha preserves glyphs, icons, reflections and pressed states.
    pub unsafe fn retain_control_backing(
        &mut self,
        source: &Self,
        r: [f32; 4],
        radius: f32,
        amount: f32,
    ) {
        if amount <= 0. {
            return;
        }
        crate::render::flush(self.dc);
        GdiFlush();
        let pixels =
            std::slice::from_raw_parts_mut(self.bits, (self.width * self.height * 4) as usize);
        let backing =
            std::slice::from_raw_parts(source.bits, (source.width * source.height * 4) as usize);
        let mut clip: RECT = zeroed();
        GetClipBox(self.dc, &mut clip);
        let [l, t, r, b] = r.map(|v| v * self.dpi);
        for y in (t.floor() as i32).max(0).max(clip.top)
            ..(b.ceil() as i32)
                .min(self.height)
                .min(source.height)
                .min(clip.bottom)
        {
            for x in (l.floor() as i32).max(0).max(clip.left)
                ..(r.ceil() as i32)
                    .min(self.width)
                    .min(source.width)
                    .min(clip.right)
            {
                let i = ((y * self.width + x) * 4) as usize;
                if pixels[i + 3] == 255 {
                    continue;
                }
                let coverage = (0.5
                    - distance(
                        x as f32 + 0.5 - l,
                        y as f32 + 0.5 - t,
                        r - l,
                        b - t,
                        radius * self.dpi,
                        0.,
                    ))
                .clamp(0., 1.);
                let missing =
                    ((255 - pixels[i + 3]) as f32 * coverage * amount.clamp(0., 1.)).round() as u32;
                if missing == 0 {
                    continue;
                }
                let s = ((y * source.width + x) * 4) as usize;
                for c in 0..3 {
                    pixels[i + c] = (pixels[i + c] as u32
                        + (backing[s + c] as u32 * missing + 127) / 255)
                        .min(255) as u8;
                }
                pixels[i + 3] = (pixels[i + 3] as u32 + missing).min(255) as u8;
            }
        }
    }
    pub unsafe fn present_with_alpha(&mut self, hwnd: HWND, preserve_alpha: bool) -> bool {
        crate::fusion::prepare(hwnd, self.radius);
        let group = crate::fusion::group(hwnd);
        if let Some(panes) = group {
            let r = crate::fusion::window_rect(hwnd);
            let bounds = [r.left, r.top, self.width, self.height];
            if self
                .fusion_cover
                .as_ref()
                .is_none_or(|(old_panes, old_bounds, _)| {
                    *old_panes != panes || *old_bounds != bounds
                })
            {
                let mut cover = self.cover.clone();
                let band = ((self.radius + 10.) * self.dpi).ceil() as i32;
                let w = self.width;
                let h = self.height;
                let interior_clear = !panes.iter().any(|p| {
                    p.id != hwnd as usize
                        && p.rect[0] - 4. * p.dpi < (r.right - band) as f32
                        && p.rect[2] + 4. * p.dpi > (r.left + band) as f32
                        && p.rect[1] - 4. * p.dpi < (r.bottom - band) as f32
                        && p.rect[3] + 4. * p.dpi > (r.top + band) as f32
                });
                crate::fusion::geometry::with_outline(&panes, |outline| {
                    for y in 0..h {
                        let in_y_mid = interior_clear && y >= band && y < h - band;
                        let gy = r.top as f32 + y as f32 + 0.5;
                        let row = (y * w) as usize;
                        for x in 0..w {
                            if in_y_mid && x >= band && x < w - band {
                                continue;
                            }
                            let gx = r.left as f32 + x as f32 + 0.5;
                            let i = row + x as usize;
                            cover[i] = if crate::fusion::geometry::owner(&panes, gx, gy)
                                != Some(hwnd as usize)
                            {
                                0
                            } else if crate::fusion::geometry::covered_core(&panes, gx, gy) {
                                255
                            } else {
                                let d = outline.distance([gx, gy]);
                                (outline.coverage_with_distance(gx, gy, d) * 255.).round() as u8
                            };
                        }
                    }
                });
                self.fusion_cover = Some((panes, bounds, cover));
            }
        } else {
            self.fusion_cover = None;
        }
        GdiFlush();
        let pixels = std::slice::from_raw_parts_mut(
            self.bits,
            self.width as usize * self.height as usize * 4,
        );
        let cover = self
            .fusion_cover
            .as_ref()
            .map_or(self.cover.as_slice(), |(_, _, c)| c.as_slice());
        for (p, &cov) in pixels.chunks_exact_mut(4).zip(cover.iter()) {
            let a = cov as u32;
            if a == 255 {
                p[3] = if preserve_alpha { p[3].max(1) } else { 255 };
                continue;
            }
            if a == 0 {
                p.copy_from_slice(&[0; 4]);
                continue;
            }
            for c in &mut p[..3] {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
            // The separate wide shadow owns all shadow pixels. Keeping a
            // second tight shadow here made the bottom edge look hard.
            p[3] = window_pixel_alpha(p[3], a as u8, preserve_alpha);
        }
        self.upload(hwnd)
    }
    pub unsafe fn replace_pixels(&self, pixels: &[u8]) {
        assert_eq!(
            pixels.len(),
            (self.width as usize * self.height as usize * 4)
        );
        GdiFlush();
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.bits, pixels.len());
    }
    pub unsafe fn upload(&self, hwnd: HWND) -> bool {
        self.upload_alpha(hwnd, 255)
    }
    pub unsafe fn upload_alpha(&self, hwnd: HWND, opacity: u8) -> bool {
        let pending = crate::fusion::pending_move_point(hwnd);
        if crate::gpu::is_gpu_window(hwnd) {
            if !crate::fusion::in_wm_moving() {
                let mut wr: RECT = std::mem::zeroed();
                windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut wr);
                let need_size =
                    (wr.right - wr.left) != self.width || (wr.bottom - wr.top) != self.height;
                if pending.is_some() || need_size {
                    let (x, y) = pending.map(|p| (p.x, p.y)).unwrap_or((wr.left, wr.top));
                    let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
                    if pending.is_none() {
                        flags |= SWP_NOMOVE;
                    }
                    if !need_size {
                        flags |= SWP_NOSIZE;
                    }
                    SetWindowPos(hwnd, null_mut(), x, y, self.width, self.height, flags);
                }
            }
            GdiFlush();
            let len = self.width as usize * self.height as usize * 4;
            let pixels = std::slice::from_raw_parts(self.bits, len);
            let result = if opacity == 255 {
                crate::gpu::present_bgra(hwnd, self.width, self.height, pixels)
            } else {
                let a = opacity as u32;
                let mut scaled = vec![0u8; len];
                for (dst, src) in scaled.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
                    dst[0] = ((src[0] as u32 * a + 127) / 255) as u8;
                    dst[1] = ((src[1] as u32 * a + 127) / 255) as u8;
                    dst[2] = ((src[2] as u32 * a + 127) / 255) as u8;
                    dst[3] = ((src[3] as u32 * a + 127) / 255) as u8;
                }
                crate::gpu::present_bgra(hwnd, self.width, self.height, &scaled)
            };
            return result;
        }
        let origin = POINT { x: 0, y: 0 };
        #[cfg(test)]
        test_capture::record(hwnd, pending, self.width, self.height, self.bits);
        let size = SIZE {
            cx: self.width,
            cy: self.height,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: opacity,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let result = UpdateLayeredWindow(
            hwnd,
            null_mut(),
            pending.as_ref().map_or(std::ptr::null(), |p| p as *const POINT),
            &size,
            self.dc,
            &origin,
            0,
            &blend,
            ULW_ALPHA,
        ) != 0;
        if !result {
            eprintln!("native-dock compositor failed: {}", GetLastError());
        }
        result
    }
}
unsafe extern "system" fn shadow_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCHITTEST => HTTRANSPARENT as isize,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_ERASEBKGND => 1,
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}
pub struct WindowShadow {
    pub(crate) hwnd: HWND,
    owner: HWND,
    template: Vec<u8>,
    side: usize,
    edge: usize,
    dpi: f32,
    radius: f32,
    rect: [i32; 4],
    opacity: u8,
}
impl Drop for WindowShadow {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.hwnd) != 0 {
                DestroyWindow(self.hwnd);
            }
        }
    }
}
impl WindowShadow {
    /// A Win32 owned popup is forced above its owner even when SetWindowPos asks
    /// otherwise. Shadows are unowned decoration windows, explicitly stacked
    /// behind their visual owner and constrained to its desktop band.
    unsafe fn stack_below_owner(&self) {
        let desktop = crate::desktop_layer::contains(self.owner);
        crate::desktop_layer::register(self.hwnd, desktop);
        let topmost =
            !desktop && GetWindowLongPtrW(self.owner, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0;
        if (GetWindowLongPtrW(self.hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0) != topmost {
            SetWindowPos(
                self.hwnd,
                if topmost {
                    HWND_TOPMOST
                } else {
                    HWND_NOTOPMOST
                },
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
        if GetWindow(self.hwnd, GW_HWNDPREV) != self.owner {
            SetWindowPos(
                self.hwnd,
                self.owner,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }
    /// Follow the native move before the owner repaint starts. Reuse the
    /// uploaded bitmap; moving a shadow must not wait for wallpaper rendering.
    pub unsafe fn follow_position(&mut self, position: &WINDOWPOS, dpi: f32) {
        self.stack_below_owner();
        if position.flags & SWP_NOMOVE != 0 || self.rect[2] == 0 || self.dpi != dpi {
            return;
        }
        let pad = (40. * dpi).ceil() as i32;
        let x = position.x - pad;
        let y = position.y - pad;
        if self.rect[0] == x && self.rect[1] == y {
            return;
        }
        SetWindowPos(
            self.hwnd,
            self.owner,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        self.rect[0] = x;
        self.rect[1] = y;
    }
    pub unsafe fn hide(&self) {
        if IsWindowVisible(self.hwnd) != 0 {
            ShowWindow(self.hwnd, SW_HIDE);
        }
    }
    pub unsafe fn new(owner: HWND) -> Option<Self> {
        let class = crate::model::wide("LumenNativeGlassShadow");
        let instance =
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(shadow_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            std::ptr::null(),
            WS_POPUP | WS_DISABLED,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return None;
        }
        Some(Self {
            hwnd,
            owner,
            template: vec![],
            side: 0,
            edge: 0,
            dpi: 0.,
            radius: 0.,
            rect: [0; 4],
            opacity: 255,
        })
    }
    pub unsafe fn sync(&mut self, owner: HWND, dpi: f32, opacity: u8) {
        self.sync_with_radius(owner, dpi, opacity, 34.);
    }
    pub unsafe fn sync_with_radius(&mut self, owner: HWND, dpi: f32, opacity: u8, radius: f32) {
        self.owner = owner;
        crate::fusion::opacity(owner, opacity);
        if crate::fusion::fused(owner) || IsWindowVisible(owner) == 0 || IsIconic(owner) != 0 {
            if IsWindowVisible(self.hwnd) != 0 {
                ShowWindow(self.hwnd, SW_HIDE);
            }
            return;
        }
        let r = crate::fusion::window_rect(owner);
        let pad = (40. * dpi).ceil() as i32;
        let width = r.right - r.left + pad * 2;
        let height = r.bottom - r.top + pad * 2;
        let rect = [r.left - pad, r.top - pad, width, height];
        if self.rect == rect && self.dpi == dpi && self.radius == radius && self.opacity == opacity
        {
            if IsWindowVisible(self.hwnd) == 0 {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
                self.stack_below_owner();
            } else if GetWindow(owner, GW_HWNDNEXT) != self.hwnd {
                self.stack_below_owner();
            }
            return;
        }
        if self.dpi != dpi || self.radius != radius || self.template.is_empty() {
            let core = ((radius * 2. + 28.).max(96.) * dpi).round() as i32;
            let side = core + pad * 2;
            self.side = side as usize;
            self.edge = ((40. + 4. + radius) * dpi).ceil() as usize;
            self.template = vec![0; (side * side) as usize];
            for y in 0..side {
                for x in 0..side {
                    let px = x as f32 + 0.5 - pad as f32;
                    let py = y as f32 + 0.5 - pad as f32;
                    let original =
                        distance(px, py, core as f32, core as f32, radius * dpi, 4. * dpi);
                    let shifted = distance(
                        px - 5. * dpi,
                        py - 8. * dpi,
                        core as f32,
                        core as f32,
                        radius * dpi,
                        4. * dpi,
                    );
                    let alpha = 0.5
                        * (1. - (shifted / (11. * dpi * 1.55)).tanh())
                        * (original / (0.75 * dpi)).clamp(0., 1.);
                    let fade = ((x.min(side - 1 - x).min(y).min(side - 1 - y)) as f32 / (4. * dpi))
                        .clamp(0., 1.);
                    self.template[(y * side + x) as usize] =
                        (alpha * 96. * fade).round().clamp(0., 255.) as u8;
                }
            }
        }
        let changed = self.rect[2] != width
            || self.rect[3] != height
            || self.dpi != dpi
            || self.radius != radius
            || self.opacity != opacity;
        let upload = if changed {
            let Some(buffer) = Buffer::plain(width, height, dpi) else {
                return;
            };
            let pixels = std::slice::from_raw_parts_mut(buffer.bits, (width * height * 4) as usize);
            pixels.fill(0);
            let edge = self.edge.min(width as usize / 2).min(height as usize / 2);
            for y in 0..height as usize {
                let sy = if y < edge {
                    y
                } else if y >= height as usize - edge {
                    self.side - (height as usize - y)
                } else {
                    self.side / 2
                };
                if y >= edge && y < height as usize - edge {
                    for x in (0..edge).chain(width as usize - edge..width as usize) {
                        let sx = if x < edge {
                            x
                        } else {
                            self.side - (width as usize - x)
                        };
                        pixels[(y * width as usize + x) * 4 + 3] =
                            self.template[sy * self.side + sx];
                    }
                } else {
                    for x in 0..width as usize {
                        let sx = if x < edge {
                            x
                        } else if x >= width as usize - edge {
                            self.side - (width as usize - x)
                        } else {
                            self.side / 2
                        };
                        pixels[(y * width as usize + x) * 4 + 3] =
                            self.template[sy * self.side + sx];
                    }
                }
            }
            Some(buffer)
        } else {
            None
        };
        SetWindowPos(
            self.hwnd,
            owner,
            rect[0],
            rect[1],
            width,
            height,
            SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_SHOWWINDOW,
        );
        self.stack_below_owner();
        if let Some(buffer) = upload {
            // UpdateLayeredWindow copies the surface. Keep only the small
            // nine-slice alpha template after upload, not a duplicate CPU DIB.
            buffer.upload_alpha(self.hwnd, opacity);
        }
        self.opacity = opacity;
        self.rect = rect;
        self.dpi = dpi;
        self.radius = radius;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn moving_desktop_shadow_remains_below_widget_and_application() {
        let _lock = crate::native_ui_test_lock();
        unsafe {
            let make = || {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    crate::model::wide("STATIC").as_ptr(),
                    crate::model::wide("Lumen shadow stack test").as_ptr(),
                    WS_POPUP,
                    -22000,
                    -22000,
                    160,
                    120,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    std::ptr::null(),
                )
            };
            let application = make();
            let widget = make();
            assert!(!application.is_null() && !widget.is_null());
            ShowWindow(application, SW_SHOWNOACTIVATE);
            ShowWindow(widget, SW_SHOWNOACTIVATE);
            crate::desktop_layer::register(widget, true);
            crate::desktop_layer::pin(widget);
            let mut shadow = WindowShadow::new(widget).unwrap();
            assert!(
                GetWindow(shadow.hwnd, GW_OWNER).is_null(),
                "shadow cannot be an owned popup forced above its owner"
            );
            for index in 0..4 {
                SetWindowPos(
                    widget,
                    HWND_TOP,
                    -22000 + index * 5,
                    -22000 + index * 3,
                    160,
                    120,
                    SWP_NOACTIVATE,
                );
                let position = WINDOWPOS {
                    hwnd: widget,
                    hwndInsertAfter: HWND_TOP,
                    x: -22000 + index * 5,
                    y: -22000 + index * 3,
                    cx: 160,
                    cy: 120,
                    flags: 0,
                };
                shadow.follow_position(&position, 1.);
                shadow.sync(widget, 1., 255);
                assert!(crate::desktop_layer::contains(shadow.hwnd));
                assert_eq!(
                    GetWindow(shadow.hwnd, GW_HWNDPREV),
                    widget,
                    "shadow must sit directly behind its widget during movement"
                );
                assert_eq!(
                    GetWindowLongPtrW(shadow.hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize,
                    0
                );
                let mut next = GetWindow(application, GW_HWNDNEXT);
                let mut found = false;
                for _ in 0..4096 {
                    if next.is_null() {
                        break;
                    }
                    if next == shadow.hwnd {
                        found = true;
                        break;
                    }
                    next = GetWindow(next, GW_HWNDNEXT);
                }
                assert!(found, "shadow must remain behind other application windows");
            }
            drop(shadow);
            DestroyWindow(widget);
            DestroyWindow(application);
        }
    }
    #[test]
    fn button_backing_matches_opaque_reference_without_overwriting_content() {
        let _runtime = crate::render::Runtime::start().unwrap();
        unsafe {
            let mut backing = Buffer::plain(100, 100, 1.).unwrap();
            let color = CreateSolidBrush(0x805030);
            FillRect(
                backing.dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 100,
                },
                color,
            );
            DeleteObject(color);
            backing.background_alpha(255);
            let mut reference = None;
            for opacity in [255, 128, 0] {
                let mut frame = Buffer::plain(100, 100, 1.).unwrap();
                frame.copy_from(backing.dc);
                frame.background_alpha(opacity);
                {
                    let batch = crate::render::AlphaBatch::new(frame.dc, 1.);
                    let c = crate::render::Canvas::new(frame.dc, 1.);
                    c.round([20., 20., 80., 80.], 20., 0xffffff, 55, false, false);
                    c.round([45., 40., 55., 60.], 1., 0x102030, 255, false, false);
                    drop(c);
                    drop(batch);
                }
                let before = frame.snapshot();
                frame.retain_control_backing(&backing, [20., 20., 80., 80.], 20., 1.);
                let pixels = frame.snapshot();
                let glyph = (50 * 100 + 50) * 4;
                assert_eq!(
                    &pixels[glyph..glyph + 4],
                    &before[glyph..glyph + 4],
                    "glyph stays untouched"
                );
                assert_eq!(
                    pixels[(10 * 100 + 10) * 4 + 3],
                    opacity,
                    "only the button is retained"
                );
                let body = (30 * 100 + 50) * 4;
                assert_eq!(pixels[body + 3], 255);
                if let Some(expected) = reference {
                    let expected: [u8; 4] = expected;
                    assert!(pixels[body..body + 4]
                        .iter()
                        .zip(expected)
                        .all(|(a, b)| (*a as i16 - b as i16).abs() <= 1));
                } else {
                    reference = Some(<[u8; 4]>::try_from(&pixels[body..body + 4]).unwrap());
                }
                assert!(pixels
                    .chunks_exact(4)
                    .all(|p| p[..3].iter().all(|v| *v <= p[3])));
                assert_eq!(
                    pixels[(20 * 100 + 20) * 4 + 3],
                    opacity,
                    "rounded corners do not become square"
                );
            }
        }
    }
    #[test]
    fn corner_geometry_and_shadow_refresh_without_a_size_change() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                crate::model::wide("STATIC").as_ptr(),
                crate::model::wide("Corner geometry regression").as_ptr(),
                WS_POPUP,
                -20000,
                -20000,
                420,
                360,
                null_mut(),
                null_mut(),
                null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            for dpi in [1.0_f32, 1.25, 1.75, 2.] {
                let mut shadow = WindowShadow::new(hwnd).unwrap();
                shadow.sync_with_radius(hwnd, dpi, 255, 34.);
                let before = shadow.template.clone();
                let bounds = shadow.rect;
                shadow.sync_with_radius(hwnd, dpi, 255, 30.);
                assert_eq!(shadow.rect, bounds, "radius updates without resize");
                assert_ne!(before, shadow.template);
                let mut fresh = WindowShadow::new(hwnd).unwrap();
                fresh.sync_with_radius(hwnd, dpi, 255, 30.);
                assert_eq!(
                    shadow.template, fresh.template,
                    "updated shadow equals a fresh one"
                );
                let frame = Buffer::new(420, 360, dpi, 34.).unwrap();
                assert!(frame.matches_geometry(420, 360, dpi, 34.));
                assert!(!frame.matches_geometry(420, 360, dpi, 30.));
                let corrected = Buffer::new(420, 360, dpi, 30.).unwrap();
                assert_ne!(
                    frame.cover, corrected.cover,
                    "radius changes the actual alpha cut"
                );
            }
            DestroyWindow(hwnd);
        }
    }
    #[test]
    fn transparent_chrome_keeps_a_hit_target_but_not_outside_the_rounded_window() {
        assert_eq!(window_pixel_alpha(0, 255, true), 1);
        assert_eq!(window_pixel_alpha(0, 0, true), 0);
        assert_eq!(window_pixel_alpha(128, 255, true), 128);
        assert_eq!(window_pixel_alpha(255, 128, true), 128);
        assert_eq!(window_pixel_alpha(0, 255, false), 255);
    }
    #[test]
    fn independent_glass_card_stays_opaque_over_a_transparent_widget() {
        unsafe {
            let backing = Buffer::plain(100, 100, 1.).unwrap();
            let brush = CreateSolidBrush(0x204060);
            FillRect(
                backing.dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 100,
                },
                brush,
            );
            DeleteObject(brush);
            let source = backing.snapshot();
            let mut frame = Buffer::plain(100, 100, 1.).unwrap();
            frame.copy_from(backing.dc);
            frame.background_alpha(0);
            backing.restore_glass_card(frame.dc, [20., 20., 80., 80.], 20.);
            let result = frame.snapshot();
            let center = (50 * 100 + 50) * 4;
            assert_eq!(&result[center..center + 3], &source[center..center + 3]);
            assert_eq!(result[center + 3], 255);
            assert_eq!(result[3], 0);
            assert_eq!(result[(20 * 100 + 20) * 4 + 3], 0);
            assert!(result.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
            assert!(result
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|c| *c <= p[3])));
        }
    }
    #[test]
    fn search_glass_deepens_dark_backdrops_but_preserves_bright_ones() {
        let _runtime = crate::render::Runtime::start().unwrap();
        unsafe {
            let buffer = Buffer::plain(240, 80, 1.).unwrap();
            let pixels = std::slice::from_raw_parts_mut(buffer.bits, 240 * 80 * 4);
            for y in 0..80 {
                for x in 0..240 {
                    let value = if x < 120 { 72 } else { 220 };
                    pixels[(y * 240 + x) * 4..(y * 240 + x) * 4 + 4]
                        .copy_from_slice(&[value, value, value, 255]);
                }
            }
            let pane = [10., 10., 230., 70.];
            let material = buffer
                .frosted_piece(pane, 30., 8., [0., 0.], 0x003a3936)
                .unwrap();
            {
                let c = crate::render::Canvas::new(buffer.dc, 1.);
                c.image_snapped(&material, pane);
                c.search_surface(pane, 30., 0x003a3936, false);
            }
            let result = buffer.snapshot();
            let dark = result[(40 * 240 + 60) * 4];
            let bright = result[(40 * 240 + 180) * 4];
            assert!(dark < 69, "dark wallpaper should gain depth: {dark}");
            assert!(
                bright >= 210,
                "bright wallpaper should stay luminous: {bright}"
            );
        }
    }
    #[test]
    fn nested_frost_blurs_its_own_backdrop_and_applies_only_its_optical_offset() {
        let _runtime = crate::render::Runtime::start().unwrap();
        unsafe {
            let buffer = Buffer::plain(240, 120, 1.).unwrap();
            let pixels = std::slice::from_raw_parts_mut(buffer.bits, 240 * 120 * 4);
            for y in 0..120 {
                for x in 0..240 {
                    let i = (y * 240 + x) * 4;
                    pixels[i..i + 4].copy_from_slice(&[x as u8, x as u8, x as u8, 255]);
                }
            }
            let original = buffer.snapshot();
            let base = buffer
                .frosted_piece([30., 30., 210., 90.], 30., 8., [0., 0.], 0x00808080)
                .unwrap();
            let shifted = buffer
                .frosted_piece([30., 30., 210., 90.], 30., 8., [4., 0.], 0x00808080)
                .unwrap();
            // Compose onto transparent bitmaps to inspect premultiplied alpha.
            let read = |image: &crate::render::Image| {
                let out = Buffer::plain(180, 60, 1.).unwrap();
                std::slice::from_raw_parts_mut(out.bits, 180 * 60 * 4).fill(0);
                {
                    let c = crate::render::Canvas::new(out.dc, 1.);
                    c.image_snapped(image, [0., 0., 180., 60.]);
                }
                out.snapshot()
            };
            let a = read(&base);
            let b = read(&shifted);
            let center = (30 * 180 + 90) * 4;
            assert!((b[center] as i32 - a[center] as i32 - 4).abs() <= 1);
            assert_eq!(a[0], 0);
            assert_eq!(buffer.snapshot(), original);
            let pixels = std::slice::from_raw_parts_mut(buffer.bits, 240 * 120 * 4);
            for y in 0..120 {
                for x in 0..240 {
                    let value = if (x / 4 + y / 4) % 2 == 0 { 0 } else { 255 };
                    pixels[(y * 240 + x) * 4..(y * 240 + x) * 4 + 3].fill(value);
                }
            }
            let softened = read(
                &buffer
                    .frosted_piece([30., 30., 210., 90.], 30., 8., [0., 0.], 0x00808080)
                    .unwrap(),
            );
            let values: Vec<_> = (20..40)
                .flat_map(|y| (60..120).map(move |x| (y * 180 + x) * 4))
                .map(|i| softened[i])
                .collect();
            assert!(values.iter().max().unwrap() - values.iter().min().unwrap() < 150);
        }
    }
    #[test]
    fn widget_list_fade_softens_both_edges_without_touching_header() {
        unsafe {
            let base = Buffer::new(20, 120, 1., 0.).unwrap();
            let mut frame = Buffer::new(20, 120, 1., 0.).unwrap();
            std::slice::from_raw_parts_mut(base.bits, 20 * 120 * 4).fill(20);
            std::slice::from_raw_parts_mut(frame.bits, 20 * 120 * 4).fill(220);
            frame.fade_list_region(&base, 40., 110.);
            let p = frame.snapshot();
            let at = |y: usize| p[y * 20 * 4];
            assert_eq!(at(20), 220);
            assert!(at(40) < at(45));
            assert_eq!(at(60), 220);
            assert!(at(109) < at(100));
            assert_eq!(at(115), 220);
        }
    }
    #[test]
    fn resting_scroll_edges_leave_the_first_and_last_items_intact() {
        unsafe {
            let base = Buffer::new(20, 120, 1., 0.).unwrap();
            let mut frame = Buffer::new(20, 120, 1., 0.).unwrap();
            std::slice::from_raw_parts_mut(base.bits, 20 * 120 * 4).fill(20);
            std::slice::from_raw_parts_mut(frame.bits, 20 * 120 * 4).fill(220);
            frame.fade_list_edges(&base, 40., 110., false, false);
            let p = frame.snapshot();
            assert_eq!(p[40 * 20 * 4], 220);
            assert_eq!(p[109 * 20 * 4], 220);
        }
    }
    #[test]
    fn horizontal_fade_preserves_resting_edge_header_and_scrollbar() {
        unsafe {
            let base = Buffer::new(120, 100, 1., 0.).unwrap();
            let mut frame = Buffer::new(120, 100, 1., 0.).unwrap();
            std::slice::from_raw_parts_mut(base.bits, 120 * 100 * 4).fill(20);
            std::slice::from_raw_parts_mut(frame.bits, 120 * 100 * 4).fill(220);
            frame.fade_horizontal_edges(&base, [14., 54., 106., 88.], false, true);
            let p = frame.snapshot();
            let at = |x: usize, y: usize| p[(y * 120 + x) * 4];
            assert_eq!(at(14, 60), 220);
            assert_eq!(at(60, 60), 220);
            assert!(at(105, 60) < at(97, 60));
            assert_eq!(at(105, 30), 220);
            assert_eq!(at(105, 95), 220);
            std::slice::from_raw_parts_mut(frame.bits, 120 * 100 * 4).fill(220);
            frame.fade_horizontal_edges(&base, [14., 54., 106., 88.], true, false);
            let p = frame.snapshot();
            assert!(p[(60 * 120 + 14) * 4] < p[(60 * 120 + 23) * 4]);
            assert_eq!(p[(60 * 120 + 105) * 4], 220);
        }
    }
    #[test]
    fn fractional_viewport_bottom_does_not_leave_a_bright_pixel_row() {
        unsafe {
            let base = Buffer::new(20, 100, 1.75, 0.).unwrap();
            let mut frame = Buffer::new(20, 100, 1.75, 0.).unwrap();
            std::slice::from_raw_parts_mut(base.bits, 20 * 100 * 4).fill(60);
            std::slice::from_raw_parts_mut(frame.bits, 20 * 100 * 4).fill(200);
            frame.fade_list(&base, 10.2, 44.3);
            let pixels = frame.snapshot();
            for y in 77..100 {
                for x in 0..20 {
                    assert_eq!(pixels[(y * 20 + x) * 4], 60);
                }
            }
        }
    }
    #[test]
    fn curved_edge_has_fractional_coverage() {
        let mut partial = 0;
        for y in 0..45 {
            for x in 0..45 {
                let a = (0.5 - distance(x as f32 + 0.5, y as f32 + 0.5, 180., 100., 34., 4.))
                    .clamp(0., 1.);
                if a > 0. && a < 1. {
                    partial += 1;
                }
            }
        }
        assert!(partial > 20);
    }
    #[test]
    fn rounded_mask_is_symmetric_and_clear_outside() {
        assert!(distance(0.5, 0.5, 180., 100., 34., 4.) > 1.);
        assert!(distance(90., 50., 180., 100., 34., 4.) < 0.);
        for x in 0..40 {
            let a = distance(x as f32, 18., 180., 100., 34., 4.);
            let b = distance(180. - x as f32, 18., 180., 100., 34., 4.);
            assert!((a - b).abs() < 0.001);
        }
    }
    #[test]
    fn restore_glass_card_matches_full_sdf_across_interior_and_rim() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::glass::set_liquid_glass(self.0);
            }
        }
        let _reset = Reset(crate::glass::liquid_glass());
        crate::glass::set_liquid_glass(true);
        unsafe {
            let w = 180i32;
            let h = 120i32;
            let dpi = 1.25f32;
            let card = [12.3, 10.7, 131.7, 85.3];
            let radius = 20.0f32;
            let backing = Buffer::plain(w, h, dpi).unwrap();
            let src_px = std::slice::from_raw_parts_mut(backing.bits, (w * h * 4) as usize);
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    src_px[i] = ((x * 3 + y) & 0xff) as u8;
                    src_px[i + 1] = ((x + y * 5) & 0xff) as u8;
                    src_px[i + 2] = (((x ^ y) * 7) & 0xff) as u8;
                    src_px[i + 3] = 255;
                }
            }
            let source = backing.snapshot();
            let frame = Buffer::plain(w, h, dpi).unwrap();
            let init = [15u8, 25, 35, 80];
            for px in std::slice::from_raw_parts_mut(frame.bits, (w * h * 4) as usize)
                .chunks_exact_mut(4)
            {
                px.copy_from_slice(&init);
            }
            let mut reference = frame.snapshot();
            backing.restore_glass_card(frame.dc, card, radius);
            let actual = frame.snapshot();

            let [left, top, right, bottom] = card.map(|v| v * dpi);
            let card_w = (right - left).max(1.0);
            let card_h = (bottom - top).max(1.0);
            let min_dim = card_w.min(card_h);
            let r_px = (radius * dpi).min(min_dim * 0.5);
            let hw = card_w * 0.5;
            let hh = card_h * 0.5;
            let rx = (hw - r_px).max(0.0);
            let ry = (hh - r_px).max(0.0);
            let rim = (20.0 * dpi).clamp(8.0, min_dim * 0.42);
            let bend = 10.0 * dpi;
            let ca_strength = 3.5 * dpi;
            for y in (top.floor() as i32).max(0)..(bottom.ceil() as i32).min(h) {
                for x in (left.floor() as i32).max(0)..(right.ceil() as i32).min(w) {
                    let fx = x as f32 + 0.5 - left;
                    let fy = y as f32 + 0.5 - top;
                    let (d, nx, ny) = crate::glass::rounded_rect_sdf(fx, fy, hw, hh, rx, ry, r_px);
                    let alpha = ((0.5 - d).clamp(0., 1.) * 255.).round() as u32;
                    let dst = ((y * w + x) * 4) as usize;
                    let pixel = if d > -rim && d <= 0.5 {
                        crate::glass::refract_bgra_card_pixel(
                            &source,
                            w as usize,
                            h as usize,
                            x as f32,
                            y as f32,
                            d,
                            nx,
                            ny,
                            rim,
                            bend,
                            ca_strength,
                            dpi,
                        )
                    } else {
                        [source[dst], source[dst + 1], source[dst + 2]]
                    };
                    for c in 0..4 {
                        let value = if c == 3 { 255 } else { pixel[c] as u32 };
                        reference[dst + c] =
                            ((value * alpha + reference[dst + c] as u32 * (255 - alpha) + 127)
                                / 255) as u8;
                    }
                }
            }
            assert_eq!(actual, reference);
            let center = ((h / 2) * w + (w / 2)) as usize * 4;
            assert_eq!(&actual[center..center + 3], &source[center..center + 3]);
            assert_eq!(actual[center + 3], 255);
            assert_eq!(&actual[0..4], &init);
            let rim_px = (top.ceil() as usize + 3) * w as usize * 4 + (w / 2) as usize * 4;
            assert_ne!(&actual[rim_px..rim_px + 3], &source[rim_px..rim_px + 3]);
        }
    }

    #[test]
    fn parallel_nested_glass_preserves_clipping_and_matches_serial_pixels() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::glass::set_liquid_glass(self.0);
                crate::motion::set_low_power(self.1);
            }
        }
        let _reset = Reset(crate::glass::liquid_glass(), crate::motion::low_power());
        crate::glass::set_liquid_glass(true);
        unsafe {
            let (w, h, dpi) = (640, 480, 1.75);
            let mut backing = Buffer::plain(w, h, dpi).unwrap();
            for (i, p) in backing.pixels_mut().chunks_exact_mut(4).enumerate() {
                p.copy_from_slice(&[((i * 37) % 256) as u8, ((i * 61) % 256) as u8, ((i * 17) % 256) as u8, 255]);
            }
            let mut serial = Buffer::plain(w, h, dpi).unwrap();
            let mut parallel = Buffer::plain(w, h, dpi).unwrap();
            let initial = [15u8, 25, 35, 80].repeat((w * h) as usize);
            serial.pixels_mut().copy_from_slice(&initial);
            parallel.pixels_mut().copy_from_slice(&initial);
            IntersectClipRect(serial.dc, 18, 23, 610, 354);
            IntersectClipRect(parallel.dc, 18, 23, 610, 354);
            let card = [8.25, 10.5, 350.5, 225.5];
            crate::motion::set_low_power(true);
            backing.restore_glass_card(serial.dc, card, 23.);
            crate::motion::set_low_power(false);
            backing.restore_glass_card(parallel.dc, card, 23.);
            let result = parallel.snapshot();
            assert!(result == serial.snapshot(), "parallel card must match the serial optical path");
            assert_eq!(&result[..4], &initial[..4], "clipped pixels must stay untouched");
            assert_ne!(result, initial, "test must exercise optical rendering");
        }
    }
}

#[cfg(test)]
pub(crate) mod test_capture {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    #[derive(Clone)]
    pub(crate) struct CapturedUpload {
        pub rect: [i32; 4],
        pub w: i32,
        pub h: i32,
        pub bgra: Vec<u8>,
    }

    thread_local! {
        static CAPTURES: RefCell<(bool, HashMap<isize, CapturedUpload>)> =
            RefCell::new((false, HashMap::new()));
    }

    pub(crate) struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            CAPTURES.with(|c| {
                let mut borrow = c.borrow_mut();
                borrow.0 = false;
                borrow.1.clear();
            });
        }
    }

    pub(crate) fn start() -> Guard {
        CAPTURES.with(|c| {
            let mut borrow = c.borrow_mut();
            borrow.0 = true;
            borrow.1.clear();
        });
        Guard
    }

    pub(crate) fn get(hwnd: HWND) -> Option<CapturedUpload> {
        CAPTURES.with(|c| c.borrow().1.get(&(hwnd as isize)).cloned())
    }

    pub(crate) unsafe fn record(
        hwnd: HWND,
        pending: Option<POINT>,
        w: i32,
        h: i32,
        bits: *const u8,
    ) {
        let active = CAPTURES.with(|c| c.borrow().0);
        if !active || hwnd.is_null() || bits.is_null() || w <= 0 || h <= 0 {
            return;
        }
        GdiFlush();
        let mut wr: RECT = zeroed();
        windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut wr);
        let left = pending.map_or(wr.left, |p| p.x);
        let top = pending.map_or(wr.top, |p| p.y);
        let bgra = std::slice::from_raw_parts(bits, (w * h * 4) as usize).to_vec();
        CAPTURES.with(|c| {
            c.borrow_mut().1.insert(
                hwnd as isize,
                CapturedUpload {
                    rect: [left, top, left + w, top + h],
                    w,
                    h,
                    bgra,
                },
            );
        });
    }
}
