//! Premultiplied per-pixel window edges. No binary region and no screen capture.
use std::{
    mem::{size_of, zeroed},
    ptr::null_mut,
};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

pub fn distance(x: f32, y: f32, w: f32, h: f32, r: f32, inset: f32) -> f32 {
    let qx = (x - w / 2.).abs() - (w / 2. - inset - r);
    let qy = (y - h / 2.).abs() - (h / 2. - inset - r);
    qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - r
}
struct Rim {
    index: usize,
    sample: [f32; 2],
    light: f32,
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
    fusion_cover: Option<(u64, Vec<u8>)>,
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
impl Buffer {
    pub fn matches_geometry(&self, width: i32, height: i32, dpi: f32, radius: f32) -> bool {
        self.width == width && self.height == height && self.dpi == dpi && self.radius == radius
    }
    pub unsafe fn snapshot(&self) -> Vec<u8> {
        GdiFlush();
        std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize).to_vec()
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
        GdiFlush();
        let width = ((r[2] - r[0]) * self.dpi).round().max(1.) as u32;
        let height = ((r[3] - r[1]) * self.dpi).round().max(1.) as u32;
        let sigma = (diffusion * self.dpi).max(0.5);
        let pad = (sigma * 3.).ceil() as u32;
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
        let blurred = image::imageops::fast_blur(&small, sigma * 0.5);
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
                    pixels.push(((base * 0.5 + value * 0.5) * alpha / 255.).round() as u8);
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
        for y in 0..if shape { height } else { 0 } {
            for x in 0..width {
                let fx = x as f32 + 0.5;
                let fy = y as f32 + 0.5;
                if ((fx > inset + radius && fx < w - inset - radius)
                    && (fy > inset + 6. * dpi && fy < h - inset - 6. * dpi))
                    || ((fy > inset + radius && fy < h - inset - radius)
                        && (fx > inset + 6. * dpi && fx < w - inset - 6. * dpi))
                {
                    continue;
                }
                let d = distance(fx, fy, w, h, radius, inset);
                let mut coverage = (0.5 - d).clamp(0., 1.);
                if d.abs() < 1.
                    && (fx < inset + radius || fx > w - inset - radius)
                    && (fy < inset + radius || fy > h - inset - radius)
                {
                    let mut inside = 0;
                    for sy in 0..16 {
                        for sx in 0..16 {
                            if distance(
                                x as f32 + (sx as f32 + 0.5) / 16.,
                                y as f32 + (sy as f32 + 0.5) / 16.,
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
                    coverage = inside as f32 / 256.;
                }
                let index = (y * width + x) as usize;
                cover[index] = (coverage * 255.).round() as u8;
                if d <= 0. && d > -6. * dpi {
                    let nx = distance(fx + 0.5, fy, w, h, radius, inset)
                        - distance(fx - 0.5, fy, w, h, radius, inset);
                    let ny = distance(fx, fy + 0.5, w, h, radius, inset)
                        - distance(fx, fy - 0.5, w, h, radius, inset);
                    let length = nx.hypot(ny).max(0.001);
                    let nx = nx / length;
                    let ny = ny / length;
                    let depth = -d / dpi;
                    let lighting = (-nx * 0.6 - ny * 0.8).clamp(-1., 1.);
                    // One luminous boundary, with glints that fade away from the corners.
                    let tl = (-((fx / (230. * dpi)).powi(2) + (fy / (155. * dpi)).powi(2))).exp();
                    let br = (-(((w - fx) / (170. * dpi)).powi(2)
                        + ((h - fy) / (140. * dpi)).powi(2)))
                    .exp();
                    let light = (0.045
                        + tl * 0.48 * lighting.max(0.)
                        + br * 0.25
                        + 0.09
                            * (-(((w - fx) / (200. * dpi)).powi(2) + (fy / (150. * dpi)).powi(2)))
                                .exp())
                        * (1. - (depth - 0.55 - 1. / dpi) / (1.05 / dpi)).clamp(0., 1.);
                    let bend = (1. - depth / 6.).max(0.).powi(2) * 2.5 * dpi;
                    let sx = (x as f32 - nx * bend).clamp(0., w - 1.);
                    let sy = (y as f32 - ny * bend).clamp(0., h - 1.);
                    rim.push(Rim {
                        index: (y * width + x) as usize * 4,
                        sample: [sx, sy],
                        light,
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
        self.bevel_inner(None);
    }
    /// Internal seams do not get a second bright rim. The fused signed-distance
    /// outline controls both the bevel and the final antialiased coverage.
    pub unsafe fn bevel_for(&mut self, hwnd: HWND) {
        crate::fusion::prepare(hwnd, self.radius);
        GdiFlush();
        let source = std::slice::from_raw_parts(self.bits, (self.width * self.height * 4) as usize);
        crate::fusion::publish_material(
            hwnd,
            crate::fusion::material::Material::from_bgra(
                self.width as usize,
                self.height as usize,
                source,
            ),
        );
        let mut rect: RECT = zeroed();
        GetWindowRect(hwnd, &mut rect);
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
                    for y in 0..self.height {
                        for x in 0..self.width {
                            let gx = rect.left as f32 + x as f32 + 0.5;
                            let gy = rect.top as f32 + y as f32 + 0.5;
                            let mut weight = 0f32;
                            for p in panes
                                .iter()
                                .filter(|p| crate::fusion::geometry::adjacent(own, p))
                            {
                                let a = own.rect;
                                let b = p.rect;
                                let scale = own.dpi.max(p.dpi);
                                let (normal, tangent) = if crate::fusion::geometry::touching_edges(a[2],b[0],scale)
                                    || crate::fusion::geometry::touching_edges(b[2],a[0],scale)
                                {
                                    let seam = if crate::fusion::geometry::touching_edges(a[2],b[0],scale) {
                                        (a[2]+b[0])*0.5
                                    } else {
                                        (a[0]+b[2])*0.5
                                    };
                                    (
                                        (gx - seam).abs(),
                                        (a[1].max(b[1]) - gy).max(gy - a[3].min(b[3])).max(0.),
                                    )
                                } else {
                                    let seam = if crate::fusion::geometry::touching_edges(a[3],b[1],scale) {
                                        (a[3]+b[1])*0.5
                                    } else {
                                        (a[1]+b[3])*0.5
                                    };
                                    (
                                        (gy - seam).abs(),
                                        (a[0].max(b[0]) - gx).max(gx - a[2].min(b[2])).max(0.),
                                    )
                                };
                                weight = weight.max(crate::fusion::geometry::material_weight(
                                    normal,
                                    tangent,
                                    own.dpi.max(p.dpi),
                                ));
                            }
                            if weight > 0. {
                                let i = ((y * self.width + x) * 4) as usize;
                                let color = material.sample(gx, gy);
                                for c in 0..3 {
                                    pixels[i + c] =
                                        (pixels[i + c] as f32 * (1. - weight) + color[c] * weight)
                                            .round() as u8;
                                }
                            }
                        }
                    }
                }
            }
        }
        self.bevel_inner(group.map(|p| (p, [rect.left as f32, rect.top as f32])));
    }
    unsafe fn bevel_inner(
        &mut self,
        group: Option<(Vec<crate::fusion::geometry::Pane>, [f32; 2])>,
    ) {
        GdiFlush();
        let bytes = std::slice::from_raw_parts_mut(
            self.bits,
            self.width as usize * self.height as usize * 4,
        );
        let source = bytes.to_vec();
        if let Some((ref panes, origin)) = group {
            // The new concave boundary can pass through an old transparent
            // corner, so it must not reuse just the old rounded-rect rim list.
            let band = (self.radius + 10.) * self.dpi;
            for y in 0..self.height {
                for x in 0..self.width {
                    if x as f32 > band
                        && y as f32 > band
                        && ((self.width - x) as f32) > band
                        && ((self.height - y) as f32) > band
                    {
                        continue;
                    }
                    let gx = origin[0] + x as f32 + 0.5;
                    let gy = origin[1] + y as f32 + 0.5;
                    let d = crate::fusion::shape(panes, gx, gy);
                    if !(-2.5 * self.dpi..=1.).contains(&d) {
                        continue;
                    }
                    let light = crate::fusion::geometry::rim_light(panes, gx, gy, d, self.dpi);
                    let i = ((y * self.width + x) * 4) as usize;
                    for c in 0..3 {
                        bytes[i + c] =
                            (source[i + c] as f32 + (255. - source[i + c] as f32) * light) as u8;
                    }
                }
            }
            return;
        }
        for rim in &self.rim {
            let [x, y] = rim.sample;
            let x0 = x as usize;
            let y0 = y as usize;
            let x1 = (x0 + 1).min(self.width as usize - 1);
            let y1 = (y0 + 1).min(self.height as usize - 1);
            let fx = x - x0 as f32;
            let fy = y - y0 as f32;
            for channel in 0..3 {
                let at =
                    |x: usize, y: usize| source[(y * self.width as usize + x) * 4 + channel] as f32;
                let sample = (at(x0, y0) * (1. - fx) + at(x1, y0) * fx) * (1. - fy)
                    + (at(x0, y1) * (1. - fx) + at(x1, y1) * fx) * fy;
                let lit = if rim.light > 0. {
                    sample + (255. - sample) * rim.light
                } else {
                    sample * (1. + rim.light)
                };
                bytes[rim.index + channel] = lit.clamp(0., 255.) as u8;
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
        // Cover the entire buffer: clipping rounds device coordinates, while
        // DIP viewport boundaries can fall between two physical rows.
        for y in 0..self.height as usize {
            let dip = (y as f32 + 0.5) / self.dpi;
            let t = ((dip - top) / 14.).min((bottom - dip) / 26.).clamp(0., 1.);
            if t >= 1. {
                continue;
            }
            let alpha = t * t * (3. - 2. * t);
            for x in 0..self.width as usize {
                let i = (y * self.width as usize + x) * 4;
                for c in 0..4 {
                    pixels[i + c] = (background[i + c] as f32 * (1. - alpha)
                        + pixels[i + c] as f32 * alpha)
                        .round() as u8;
                }
            }
        }
    }
    /// Dock-style smooth edges confined to a widget's scrolling viewport.
    #[cfg(test)]
    pub unsafe fn fade_list_region(&mut self, base:&Self, top:f32, bottom:f32){self.fade_list_edges(base,top,bottom,true,true);}
    pub unsafe fn fade_list_edges(&mut self, base:&Self, top:f32, bottom:f32, fade_top:bool, fade_bottom:bool) {
        GdiFlush();
        let size = (self.width * self.height * 4) as usize;
        let pixels = std::slice::from_raw_parts_mut(self.bits, size);
        let background = std::slice::from_raw_parts(base.bits, size);
        for y in 0..self.height as usize {
            let dip = (y as f32 + 0.5) / self.dpi;
            if dip < top || dip > bottom {
                continue;
            }
            let t=(if fade_top {(dip-top)/10.}else{1.}).min(if fade_bottom {(bottom-dip)/18.}else{1.}).clamp(0.,1.);
            let a = t * t * (3. - 2. * t);
            if a >= 1. {
                continue;
            }
            for x in 0..self.width as usize {
                let i = (y * self.width as usize + x) * 4;
                for c in 0..4 {
                    pixels[i + c] = (background[i + c] as f32 * (1. - a) + pixels[i + c] as f32 * a)
                        .round() as u8;
                }
            }
        }
    }
    pub unsafe fn fade_horizontal_edges(&mut self,base:&Self,r:[f32;4],fade_left:bool,fade_right:bool){
        GdiFlush();let [left,top,right,bottom]=r;
        let pixels=std::slice::from_raw_parts_mut(self.bits,(self.width*self.height*4) as usize);
        let bg=std::slice::from_raw_parts(base.bits,(base.width*base.height*4) as usize);
        for y in 0..self.height {let py=(y as f32+0.5)/self.dpi;if py<top||py>bottom{continue;}
            for x in 0..self.width {let px=(x as f32+0.5)/self.dpi;if px<left||px>right{continue;}
                let t=(if fade_left{(px-left)/18.}else{1.}).min(if fade_right{(right-px)/18.}else{1.}).clamp(0.,1.);
                if t>=1.{continue;}let a=t*t*(3.-2.*t);let i=((y*self.width+x)*4) as usize;
                for c in 0..4{pixels[i+c]=(bg[i+c] as f32*(1.-a)+pixels[i+c] as f32*a).round() as u8;}
            }
        }
    }
    pub unsafe fn fade_control_region(&mut self,base:&Self,r:[f32;4],amount:f32){
        GdiFlush();let a=amount.clamp(0.,1.);if a>=1.{return;}
        let pixels=std::slice::from_raw_parts_mut(self.bits,(self.width*self.height*4) as usize);
        let bg=std::slice::from_raw_parts(base.bits,(base.width*base.height*4) as usize);
        let [l,t,r,b]=r.map(|v|(v*self.dpi) as i32);
        for y in t.max(0)..b.min(self.height){for x in l.max(0)..r.min(self.width){let i=((y*self.width+x)*4) as usize;for c in 0..4{pixels[i+c]=(bg[i+c] as f32*(1.-a)+pixels[i+c] as f32*a).round() as u8;}}}
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
    /// Restore an independent glass card from the owned, fully opaque backing.
    /// No desktop capture and no per-card bitmap allocation.
    pub unsafe fn restore_glass_card(&self, dc: HDC, r: [f32; 4], radius: f32) {
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
        let radius = radius * self.dpi;
        for y in (top.floor() as i32).max(0).max(clip.top)
            ..(bottom.ceil() as i32).min(height).min(clip.bottom)
        {
            for x in (left.floor() as i32).max(0).max(clip.left)
                ..(right.ceil() as i32).min(width).min(clip.right)
            {
                let alpha = ((0.5
                    - distance(
                        x as f32 + 0.5 - left,
                        y as f32 + 0.5 - top,
                        right - left,
                        bottom - top,
                        radius,
                        0.,
                    ))
                .clamp(0., 1.)
                    * 255.)
                    .round() as u32;
                let src = ((y * self.width + x) * 4) as usize;
                let dst = ((y * dib.dsBm.bmWidth + x) * 4) as usize;
                for c in 0..4 {
                    let value = if c == 3 { 255 } else { source[src + c] as u32 };
                    dest[dst + c] =
                        ((value * alpha + dest[dst + c] as u32 * (255 - alpha) + 127) / 255) as u8;
                }
            }
        }
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
        let revision = crate::fusion::revision(hwnd);
        if self
            .fusion_cover
            .as_ref()
            .is_none_or(|(r, _)| *r != revision)
        {
            self.fusion_cover = crate::fusion::group(hwnd).map(|panes| {
                let mut r: RECT = zeroed();
                GetWindowRect(hwnd, &mut r);
                let mut cover = self.cover.clone();
                for (i, alpha) in cover.iter_mut().enumerate() {
                    let px = (i % self.width as usize) as f32 + 0.5;
                    let py = (i / self.width as usize) as f32 + 0.5;
                    let x = px + r.left as f32;
                    let y = py + r.top as f32;
                    *alpha = if crate::fusion::geometry::owner(&panes, x, y) != Some(hwnd as usize)
                    {
                        0
                    } else {
                        (crate::fusion::geometry::coverage(&panes, x, y) * 255.).round() as u8
                    };
                }
                (revision, cover)
            });
        }
        GdiFlush();
        let pixels = std::slice::from_raw_parts_mut(
            self.bits,
            self.width as usize * self.height as usize * 4,
        );
        for (i, p) in pixels.chunks_exact_mut(4).enumerate() {
            let a = self
                .fusion_cover
                .as_ref()
                .map_or(self.cover[i], |(_, c)| c[i]) as u32;
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
        let origin = POINT { x: 0, y: 0 };
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
            std::ptr::null(),
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
    hwnd: HWND,
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
    /// Follow the native move before the owner repaint starts. Reuse the
    /// uploaded bitmap; moving a shadow must not wait for wallpaper rendering.
    pub unsafe fn follow_position(&mut self, position: &WINDOWPOS, dpi: f32) {
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
            null_mut(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
        self.rect[0] = x;
        self.rect[1] = y;
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
            owner,
            null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return None;
        }
        Some(Self {
            hwnd,
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
        crate::fusion::opacity(owner, opacity);
        if crate::fusion::fused(owner) {
            ShowWindow(self.hwnd, SW_HIDE);
            return;
        }
        if IsWindowVisible(owner) == 0 || IsIconic(owner) != 0 {
            ShowWindow(self.hwnd, SW_HIDE);
            return;
        }
        let mut r: RECT = zeroed();
        GetWindowRect(owner, &mut r);
        let pad = (40. * dpi).ceil() as i32;
        let width = r.right - r.left + pad * 2;
        let height = r.bottom - r.top + pad * 2;
        let rect = [r.left - pad, r.top - pad, width, height];
        if self.rect == rect && self.dpi == dpi && self.radius == radius && self.opacity == opacity
        {
            if IsWindowVisible(self.hwnd) == 0 {
                ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
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
    fn widget_list_fade_softens_both_edges_without_touching_header(){unsafe {
        let base=Buffer::new(20,120,1.,0.).unwrap();let mut frame=Buffer::new(20,120,1.,0.).unwrap();
        std::slice::from_raw_parts_mut(base.bits,20*120*4).fill(20);
        std::slice::from_raw_parts_mut(frame.bits,20*120*4).fill(220);
        frame.fade_list_region(&base,40.,110.);let p=frame.snapshot();let at=|y:usize|p[y*20*4];
        assert_eq!(at(20),220);assert!(at(40)<at(45));assert_eq!(at(60),220);assert!(at(109)<at(100));assert_eq!(at(115),220);
    }}
    #[test]
    fn resting_scroll_edges_leave_the_first_and_last_items_intact(){unsafe {
        let base=Buffer::new(20,120,1.,0.).unwrap();let mut frame=Buffer::new(20,120,1.,0.).unwrap();
        std::slice::from_raw_parts_mut(base.bits,20*120*4).fill(20);
        std::slice::from_raw_parts_mut(frame.bits,20*120*4).fill(220);
        frame.fade_list_edges(&base,40.,110.,false,false);let p=frame.snapshot();
        assert_eq!(p[40*20*4],220);assert_eq!(p[109*20*4],220);
    }}
    #[test]
    fn horizontal_fade_preserves_resting_edge_header_and_scrollbar(){unsafe{
        let base=Buffer::new(120,100,1.,0.).unwrap();let mut frame=Buffer::new(120,100,1.,0.).unwrap();
        std::slice::from_raw_parts_mut(base.bits,120*100*4).fill(20);
        std::slice::from_raw_parts_mut(frame.bits,120*100*4).fill(220);
        frame.fade_horizontal_edges(&base,[14.,54.,106.,88.],false,true);
        let p=frame.snapshot();let at=|x:usize,y:usize|p[(y*120+x)*4];
        assert_eq!(at(14,60),220);assert_eq!(at(60,60),220);assert!(at(105,60)<at(97,60));assert_eq!(at(105,30),220);assert_eq!(at(105,95),220);
        std::slice::from_raw_parts_mut(frame.bits,120*100*4).fill(220);
        frame.fade_horizontal_edges(&base,[14.,54.,106.,88.],true,false);let p=frame.snapshot();
        assert!(p[(60*120+14)*4]<p[(60*120+23)*4]);assert_eq!(p[(60*120+105)*4],220);
    }}
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
}
