//! Coverflow proportions follow CoverflowPlayer.tsx / coverflow.css, with an
//! app-owned card cache for rounded artwork, type and soft Y perspective.
use super::*;
use std::time::Instant;

pub(super) const ANIM_DURATION: f32 = 0.34;

#[derive(Clone, Copy)]
pub(super) struct Transition {
    pub offset: f32,
    pub started: Instant,
}
impl Transition {
    pub fn progress(self) -> f32 {
        (self.started.elapsed().as_secs_f32() / ANIM_DURATION).clamp(0., 1.)
    }
    pub fn finished(self) -> bool {
        self.progress() >= 1.
    }
    pub fn remaining(self) -> f32 {
        let t = self.progress();
        let ease = 1. - (1. - t).powi(3);
        self.offset * (1. - ease)
    }
}
pub(super) struct Layout {
    pub deck: [f32; 4],
    pub now: [f32; 4],
    pub card_width: f32,
    pub center_y: f32,
    pub queue_rows: usize,
}
pub(super) fn layout(w: f32, h: f32, queue: bool) -> Option<Layout> {
    layout_with_top(w, h, queue, crate::settings_ui::widget_content_top(w, h))
}
fn layout_with_top(w: f32, h: f32, queue: bool, gallery_top: f32) -> Option<Layout> {
    let deck_height = 64.;
    let queue_rows = if queue {
        ((h - 390.) / 85.).floor().clamp(1., 5.) as usize
    } else {
        0
    };
    let queue_height = if queue {
        queue_rows as f32 * 38. + 16.
    } else {
        0.
    };
    let deck_width = (w * 0.50).clamp(220., 340.).min(w - 48.);
    let deck = [
        (w - deck_width) / 2.,
        h - 24. - deck_height - queue_height,
        (w + deck_width) / 2.,
        h - 24. - queue_height,
    ];
    let gallery_height = deck[1] - 20. - gallery_top;
    if gallery_height < 124. {
        return None;
    }
    let card_width = (w * 0.39).min(gallery_height - 52.).min(360.);
    let now = [deck[0] + 20., deck[1] + 8., deck[2] - 20., deck[1] + 20.];
    Some(Layout {
        deck,
        now,
        card_width,
        center_y: gallery_top + gallery_height / 2.,
        queue_rows,
    })
}
fn deck_controls(_w: f32, d: [f32; 4]) -> [[f32; 4]; 4] {
    let by = d[1] + 26.;
    let cx = (d[0] + d[2]) / 2.;
    let side = 28.;
    let gap = 12.;
    [
        [cx - side / 2. - (side + gap), by, cx + side / 2. - (side + gap), by + side],
        [cx - side / 2., by, cx + side / 2., by + side],
        [cx - side / 2. + (side + gap), by, cx + side / 2. + (side + gap), by + side],
        [d[2] - 16. - side, by, d[2] - 16., by + side],
    ]
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn card_metrics(w: f32, card_width: f32, v: f32, inset: f32) -> (f32, f32, f32) {
    let mag = v.abs();
    let scale = if mag <= 1. {
        lerp(1., 0.91, mag)
    } else if mag <= 2. {
        lerp(0.91, 0.78, mag - 1.)
    } else {
        lerp(0.78, 0.68, (mag - 2.).min(1.))
    };
    let squeeze = if mag <= 1. {
        lerp(1., 0.89, mag)
    } else if mag <= 2. {
        lerp(0.89, 0.82, mag - 1.)
    } else {
        lerp(0.82, 0.76, (mag - 2.).min(1.))
    };
    let outer = (card_width * 0.94).min((w / 2. - inset - card_width * 0.78 * 0.82 / 2.).max(0.));
    let t1 = (card_width * 0.55).min(outer * 0.65);
    let travel = if mag <= 1. {
        t1 * mag
    } else if mag <= 2. {
        lerp(t1, outer, mag - 1.)
    } else {
        outer + (mag - 2.) * (card_width * 0.18)
    };
    let cx = if v == 0. {
        w / 2.
    } else {
        w / 2. + v.signum() * travel
    };
    (cx, scale, squeeze)
}
fn card_center(w: f32, card_width: f32, slot: i32, inset: f32) -> f32 {
    card_metrics(w, card_width, slot as f32, inset).0
}
pub(super) fn neighbor_index(selected: Option<usize>, len: usize, slot: i32) -> Option<usize> {
    let index = selected? as i64 + slot as i64;
    (index >= 0 && index < len as i64).then_some(index as usize)
}
pub(super) fn selected_index(playback: &Playback) -> Option<usize> {
    if !playback.video_id.is_empty() {
        playback
            .queue
            .iter()
            .position(|t| t.video_id == playback.video_id)
    } else {
        playback.queue.iter().position(|t| t.selected)
    }
}
impl State {
    pub(super) unsafe fn coverflow_content(&mut self, c: &Canvas, dc: HDC, w: f32, h: f32) {
        let Some(layout) = layout_with_top(w, h, self.config.queue, self.content_top(w, h)) else {
            self.compact_content(c, dc, w, h);
            return;
        };
        let shift = self
            .coverflow_anim
            .filter(|a| !a.finished())
            .map_or(0., |a| a.remaining());
        let animating = shift.abs() > 0.001;
        let selected = selected_index(&self.playback);
        let range = if animating { -4..=4 } else { -2..=2 };
        let mut cards = Vec::with_capacity(9);
        for slot in range {
            let v = slot as f32 + shift;
            if v.abs() > 2.65 {
                continue;
            }
            let (title, artist, art) = if slot == 0 {
                (
                    self.playback.title.clone(),
                    self.playback.artist.clone(),
                    self.playback.art.clone().unwrap_or_default(),
                )
            } else if let Some(track) = neighbor_index(selected, self.playback.queue.len(), slot)
                .and_then(|i| self.playback.queue.get(i))
            {
                (
                    track.title.clone(),
                    track.artist.clone(),
                    track.thumb.clone(),
                )
            } else {
                continue; // No real queue neighbor: do not invent a duplicate sleeve.
            };
            cards.push((slot, v, title, artist, art));
        }
        // Always paint farthest cards first and center-most card last so cards
        // never intersect or pop in Z-order while flowing between slots.
        cards.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        for (slot, v, title, artist, art) in &cards {
            let (slot, v) = (*slot, *v);
            let (cx, scale, squeeze) =
                card_metrics(w, layout.card_width, v, 24. / self.content_scale);
            let card_w = layout.card_width * scale;
            let card_h = (layout.card_width + 52.) * scale;
            let y = layout.center_y - card_h / 2. + v.abs() * 2.;
            let r = [
                cx - card_w * squeeze / 2.,
                y,
                cx + card_w * squeeze / 2.,
                y + card_h,
            ];
            let lean = if !animating {
                slot.signum()
            } else if v.abs() < 0.25 {
                0
            } else if v < 0. {
                -1
            } else {
                1
            };
            let liquid = crate::glass::liquid_glass();
            let mut key = format!(
                "card:{card_w:.2}:{card_h:.2}:{}:{lean}:{art}:{title}:{artist}:{}:{}:{liquid}",
                self.dpi,
                self.surface_color(),
                self.light
            );
            // Reuse the existing sleeve raster while dragging or animating.
            if self.moving || animating {
                let suffix = format!(
                    ":{lean}:{art}:{title}:{artist}:{}:{}:{liquid}",
                    self.surface_color(),
                    self.light
                );
                let any_lean = format!(
                    ":{art}:{title}:{artist}:{}:{}:{liquid}",
                    self.surface_color(),
                    self.light
                );
                if let Some(cached) = self
                    .card_cache
                    .keys()
                    .find(|k| k.ends_with(&suffix))
                    .or_else(|| self.card_cache.keys().find(|k| k.ends_with(&any_lean)))
                {
                    key = cached.clone();
                }
            }
            if !self.card_cache.contains_key(&key) {
                if let Some(image) = self.album_card(
                    r,
                    [card_w, card_h],
                    scale,
                    squeeze,
                    lean,
                    art,
                    title,
                    artist,
                    w,
                ) {
                    if self.card_cache.len() >= 64 {
                        self.card_cache.clear();
                    }
                    self.card_cache.insert(key.clone(), image);
                }
            }
            if let Some(image) = self.card_cache.get(&key) {
                let pad = shadow_padding(scale, self.dpi) as f32 / self.dpi;
                c.image(image, [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad]);
            }
            if slot == 0 {
                self.buttons.push(Button {
                    rect: r,
                    action: Action::Toggle,
                });
            }
        }
        // Pre-warm transition lean variants when idle so Next/Previous transitions
        // never stall mid-flight at v = ±0.25 to rasterize or blur a new sleeve.
        if !animating && !self.moving {
            for (slot, _, title, artist, art) in &cards {
                let target_slots: &[i32] = match *slot {
                    0 => &[-1, 1],
                    -1 | 1 => &[0],
                    _ => continue,
                };
                for &target_slot in target_slots {
                    let target_lean = target_slot.signum();
                    let liquid = crate::glass::liquid_glass();
                    let suffix = format!(
                        ":{target_lean}:{art}:{title}:{artist}:{}:{}:{liquid}",
                        self.surface_color(),
                        self.light
                    );
                    if self.card_cache.keys().any(|k| k.ends_with(&suffix)) {
                        continue;
                    }
                    let (cx, scale, squeeze) = card_metrics(
                        w,
                        layout.card_width,
                        target_slot as f32,
                        24. / self.content_scale,
                    );
                    let card_w = layout.card_width * scale;
                    let card_h = (layout.card_width + 52.) * scale;
                    let y = layout.center_y - card_h / 2. + (target_slot.abs() as f32) * 2.;
                    let r = [
                        cx - card_w * squeeze / 2.,
                        y,
                        cx + card_w * squeeze / 2.,
                        y + card_h,
                    ];
                    let key = format!(
                        "card:{card_w:.2}:{card_h:.2}:{}:{target_lean}:{art}:{title}:{artist}:{}:{}:{liquid}",
                        self.dpi,
                        self.surface_color(),
                        self.light
                    );
                    if let Some(image) = self.album_card(
                        r,
                        [card_w, card_h],
                        scale,
                        squeeze,
                        target_lean,
                        art,
                        title,
                        artist,
                        w,
                    ) {
                        if self.card_cache.len() >= 64 {
                            self.card_cache.clear();
                        }
                        self.card_cache.insert(key, image);
                    }
                }
            }
        }
        let d = layout.deck;
        if let Some(bg) = &self.background {
            bg.restore_glass_card(dc, d, 24.);
        }
        c.soft_shadow(d, 24., 24., 10., 4.);
        c.round(d, 24., self.ink, 24, false, false);
        c.coating(d, 24.);
        // Keep the lens and its soft shadow inside the rounded deck. Narrow
        // decks distribute all visible controls evenly instead of crowding edges.
        let controls = deck_controls(w, d);
        let control = |i: usize| controls[i];
        self.timeline(c, layout.now, false);
        self.media_button(
            c,
            control(0),
            Action::Previous,
            self.playback.available && self.playback.can_previous,
        );
        self.media_button(c, control(1), Action::Toggle, true);
        self.media_button(
            c,
            control(2),
            Action::Next,
            self.playback.available && self.playback.can_next,
        );
        self.media_button(c, control(3), Action::Browse, true);
        if layout.queue_rows > 0 {
            self.compact_queue(c, w, d[3] + 12., layout.queue_rows);
        }
    }
    unsafe fn album_card(
        &self,
        r: [f32; 4],
        size: [f32; 2],
        scale: f32,
        squeeze: f32,
        lean: i32,
        url: &str,
        title: &str,
        artist: &str,
        viewport_width: f32,
    ) -> Option<Image> {
        let (w, h) = (size[0], size[1]);
        let (pw, ph) = (
            (w * self.dpi).round().max(1.) as i32,
            (h * self.dpi).round().max(1.) as i32,
        );
        let mut buffer = Buffer::plain(pw, ph, self.dpi)?;
        let batch = render::AlphaBatch::new(buffer.dc, self.dpi);
        let c = Canvas::new(buffer.dc, self.dpi);
        if let Some(frost) = self
            .background
            .as_ref()
            .and_then(|bg| bg.frosted_piece(r, 20. * scale, 3., [0., 0.], self.surface_color()))
        {
            c.image(&frost, [0., 0., w, h]);
        } else {
            c.round(
                [0., 0., w, h],
                20. * scale,
                self.surface_color(),
                255,
                false,
                false,
            );
        }
        c.coating([0., 0., w, h], 20. * scale);
        render::flush(buffer.dc);
        if crate::glass::liquid_glass() {
            crate::glass::apply_liquid_glass_premul_bgra(
                buffer.pixels_mut(),
                pw as usize,
                ph as usize,
                20. * scale * self.dpi,
                self.dpi * scale,
            );
        }
        let backing = buffer.snapshot();
        let art = [6. * scale, 6. * scale, w - 6. * scale, w - 6. * scale];
        self.cover(&c, url, art, 1.);
        if url.is_empty() {
            c.text(
                "LUMEN / MUSIC",
                [10. * scale, w * 0.69, w - 10. * scale, w * 0.79],
                7. * scale,
                true,
                self.muted,
                1,
            );
        }
        c.text(
            if title.is_empty() {
                "Your music"
            } else {
                title
            },
            [8. * scale, w + 3. * scale, w - 8. * scale, w + 26. * scale],
            13. * scale,
            false,
            self.ink,
            1,
        );
        c.text(
            artist,
            [8. * scale, w + 26. * scale, w - 8. * scale, h - 6. * scale],
            11. * scale,
            false,
            self.muted,
            1,
        );
        drop(c);
        drop(batch);
        let mut pixels = buffer.snapshot();
        // Round the sleeve against its owned glass, then the entire card. Work
        // in premultiplied channels so perspective never introduces black rims.
        for y in 0..ph {
            for x in 0..pw {
                let i = ((y * pw + x) * 4) as usize;
                let px = (x as f32 + 0.5) / self.dpi;
                let py = (y as f32 + 0.5) / self.dpi;
                if px >= art[0] && px < art[2] && py >= art[1] && py < art[3] {
                    let alpha = (0.5
                        - crate::compositor::distance(
                            (px - art[0]) * self.dpi,
                            (py - art[1]) * self.dpi,
                            (art[2] - art[0]) * self.dpi,
                            (art[3] - art[1]) * self.dpi,
                            14. * scale * self.dpi,
                            0.,
                        ))
                    .clamp(0., 1.);
                    for ch in 0..4 {
                        pixels[i + ch] = (pixels[i + ch] as f32 * alpha
                            + backing[i + ch] as f32 * (1. - alpha))
                            .round() as u8;
                    }
                }
                let alpha = (0.5
                    - crate::compositor::distance(
                        x as f32 + 0.5,
                        y as f32 + 0.5,
                        pw as f32,
                        ph as f32,
                        20. * scale * self.dpi,
                        0.,
                    ))
                .clamp(0., 1.);
                for ch in 0..4 {
                    pixels[i + ch] = (pixels[i + ch] as f32 * alpha).round() as u8;
                }
                if lean != 0 {
                    let brightness = if squeeze > 0.85 { 0.96 } else { 0.90 };
                    for ch in 0..3 {
                        pixels[i + ch] = (pixels[i + ch] as f32 * brightness).round() as u8;
                    }
                }
                pixels.swap(i, i + 2);
            }
        }
        let sh = ph as usize;
        let sw = pw as usize;
        let dw = if lean == 0 {
            sw
        } else {
            (pw as f32 * squeeze).round().max(1.) as usize
        };
        let out = if lean == 0 {
            pixels
        } else {
            project_card(&pixels, sw, sh, dw, lean)
        };
        let pad = shadow_padding(scale, self.dpi);
        let pixels = shadowed_card(
            &out,
            dw,
            sh,
            pad,
            scale * self.dpi,
            Some((r[0], viewport_width, self.dpi)),
        );
        Image::from_rgba((dw + pad * 2) as u32, (sh + pad * 2) as u32, pixels)
    }
}

fn shadow_padding(scale: f32, dpi: f32) -> usize {
    (64. * scale * dpi).ceil() as usize
}

fn project_card(source: &[u8], sw: usize, sh: usize, dw: usize, lean: i32) -> Vec<u8> {
    let mut out = vec![0u8; dw * sh * 4];
    for y in 0..sh {
        for x in 0..dw {
            let t = (x as f32 + 0.5) / dw as f32;
            let margin = sh as f32 * 0.055 * if lean < 0 { 1. - t } else { t };
            let sy = (y as f32 + 0.5 - margin) * sh as f32 / (sh as f32 - 2. * margin) - 0.5;
            let sx = t * sw as f32 - 0.5;
            let x0 = sx.floor() as i32;
            let y0 = sy.floor() as i32;
            let fx = sx - sx.floor();
            let fy = sy - sy.floor();
            for ch in 0..4 {
                // Transparent taps outside the source antialias the slanted
                // bottom instead of discarding its last row with a hard cut.
                let at = |xx: i32, yy: i32| {
                    if xx < 0 || yy < 0 || xx >= sw as i32 || yy >= sh as i32 {
                        0.
                    } else {
                        source[(yy as usize * sw + xx as usize) * 4 + ch] as f32
                    }
                };
                out[(y * dw + x) * 4 + ch] = ((at(x0, y0) * (1. - fx) + at(x0 + 1, y0) * fx)
                    * (1. - fy)
                    + (at(x0, y0 + 1) * (1. - fx) + at(x0 + 1, y0 + 1) * fx) * fy)
                    .round() as u8;
            }
        }
    }
    out
}

fn shadowed_card(
    card: &[u8],
    w: usize,
    h: usize,
    pad: usize,
    scale: f32,
    viewport: Option<(f32, f32, f32)>,
) -> Vec<u8> {
    let width = w + pad * 2;
    let height = h + pad * 2;
    let dx = (4. * scale).round() as usize;
    let dy = (16. * scale).round() as usize;
    let mut mask = image::RgbaImage::new(width as u32, height as u32);
    for y in 0..h {
        for x in 0..w {
            mask.put_pixel(
                (pad + x + dx) as u32,
                (pad + y + dy) as u32,
                image::Rgba([0, 0, 0, card[(y * w + x) * 4 + 3]]),
            );
        }
    }
    // Blur the actual rounded, projected silhouette. A rectangular shadow's
    // knockout leaves a conspicuous cut underneath the tilted side cards.
    let blurred = image::imageops::fast_blur(&mask, 14. * scale);
    let mut out = blurred.into_raw();
    for (i, pixel) in out.chunks_exact_mut(4).enumerate() {
        let fade = viewport.map_or(1., |(left, view_width, dpi)| {
            let gx = left + ((i % width) as f32 + 0.5 - pad as f32) / dpi;
            let t = ((gx - 8.).min(view_width - 8. - gx) / 32.).clamp(0., 1.);
            t * t * (3. - 2. * t)
        });
        // Feather only the shadow before compositing the crisp artwork. This
        // lets the large falloff die away before the widget bitmap crops it.
        pixel[3] = (pixel[3] as f32 * 0.46 * fade).round() as u8;
    }
    for y in 0..h {
        for x in 0..w {
            let from = (y * w + x) * 4;
            let to = ((pad + y) * width + pad + x) * 4;
            let inverse = 1. - card[from + 3] as f32 / 255.;
            for ch in 0..4 {
                out[to + ch] = (card[from + ch] as f32 + out[to + ch] as f32 * inverse)
                    .round()
                    .min(255.) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn album_neighbors_follow_playlist_order_without_wrapping() {
        assert_eq!(neighbor_index(Some(0), 8, -1), None);
        assert_eq!(neighbor_index(Some(0), 8, -2), None);
        assert_eq!(neighbor_index(Some(7), 8, 1), None);
        assert_eq!(neighbor_index(None, 8, -1), None);
        assert_eq!(neighbor_index(Some(0), 1, -2), None);
        assert_eq!(neighbor_index(None, 0, 1), None);
    }
    #[test]
    fn album_fan_keeps_physical_side_insets_without_shrinking_center_art() {
        for (w, card_width) in [(320., 124.8), (560., 218.4), (1000., 360.)] {
            for scale in [0.5, 0.75, 1.] {
                for slot in [-2_i32, -1, 0, 1, 2] {
                    let factor = match slot.abs() {
                        0 => 1.,
                        1 => 0.91 * 0.89,
                        _ => 0.78 * 0.82,
                    };
                    let center = card_center(w, card_width, slot, 24. / scale);
                    assert!((center - card_width * factor / 2.) * scale >= 24. - 0.01);
                    assert!((w - center - card_width * factor / 2.) * scale >= 24. - 0.01);
                }
            }
        }
    }
    #[test]
    fn tilted_card_keeps_bottom_edge_and_shadow_extends_beyond_its_silhouette() {
        let source = [80, 120, 160, 255].repeat(192 * 120);
        for lean in [-1, 1] {
            let card = project_card(&source, 192, 120, 160, lean);
            let outer_x = if lean < 0 { 159 } else { 0 };
            assert!(
                card[((119 * 160 + outer_x) * 4) + 3] > 0,
                "the last source row must survive projection"
            );
            assert!(
                card.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255),
                "slanted edges must have fractional coverage"
            );
            let shadow = shadowed_card(&card, 160, 120, 64, 1., None);
            let below = ((64 + 120 + 24) * (160 + 128) + 64 + 80) * 4;
            assert!(
                shadow[below + 3] > 3 && shadow[below + 3] < 100,
                "a broad soft shadow must remain below the card"
            );
            assert_eq!(shadow[3], 0, "shadow padding must end transparently");
            let feathered = shadowed_card(&card, 160, 120, 64, 1., Some((0., 160., 1.)));
            let row = (64 + 120 + 24) * (160 + 128);
            assert_eq!(feathered[(row + 64) * 4 + 3], 0);
            assert_eq!(feathered[(row + 64 + 159) * 4 + 3], 0);
            assert!(
                feathered[(row + 64 + 80) * 4 + 3] > 3,
                "viewport feathering keeps the broad shadow underneath the center"
            );
            assert!(shadow
                .chunks_exact(4)
                .all(|p| p[..3].iter().all(|c| *c <= p[3])));
        }
    }
    #[test]
    fn legacy_coverflow_uses_large_cards_and_a_nested_bottom_deck() {
        let l = layout(560., 480., false).unwrap();
        assert!((l.card_width - 218.4).abs() < 0.01);
        assert_eq!(l.deck[3] - l.deck[1], 64.);
        assert!(l.deck[2] - l.deck[0] <= 340.);
        assert!((l.deck[0] + l.deck[2] - 560.).abs() < 0.01);
        assert!(l.now[3] <= l.deck[1] + 22.);
        assert!(l.center_y + (l.card_width + 52.) / 2. < l.deck[1]);
        let narrow = layout(320., 400., false).unwrap();
        assert_eq!(narrow.deck[3] - narrow.deck[1], 64.);
        assert!(narrow.now[3] <= narrow.deck[1] + 22.);
        assert!(layout(240., 180., false).is_none());
        assert!(layout(900., 180., false).is_none());
        let queue = layout(560., 480., true).unwrap();
        assert!(queue.queue_rows > 0);
        assert!(queue.deck[3] + 12. + queue.queue_rows as f32 * 38. <= 480. - 12.);
    }
    #[test]
    fn slimmer_deck_keeps_controls_clear_and_gives_space_to_album_art() {
        for (w, h) in [
            (280., 300.),
            (320., 400.),
            (480., 420.),
            (560., 480.),
            (1000., 920.),
        ] {
            for queue in [false, true] {
                let Some(l) = layout(w, h, queue) else {
                    continue;
                };
                let controls = deck_controls(w, l.deck);
                for (i, r) in controls.iter().enumerate() {
                    assert!(r[0] >= l.deck[0] + 16. && r[2] <= l.deck[2] - 16.);
                    assert!(r[1] >= l.deck[1] + 7. && r[3] <= l.deck[3] - 7.);
                    for other in &controls[i + 1..] {
                        assert!(r[2] <= other[0] || other[2] <= r[0]);
                    }
                    assert!(l.now[3] <= r[1]);
                }
                assert!(l.center_y + (l.card_width + 52.) / 2. <= l.deck[1] - 20.);
            }
        }
    }
    #[test]
    fn deck_spacing_is_balanced_and_card_metrics_flow_monotonically() {
        let l = layout(560., 480., false).unwrap();
        let controls = deck_controls(560., l.deck);
        assert!(((controls[1][0] + controls[1][2]) / 2. - 280.).abs() < 0.01);
        assert!((l.deck[2] - controls[3][2] - 16.).abs() < 0.01);
        assert!(l.now[3] < controls[0][1]);
        let narrow = layout(320., 400., false).unwrap();
        let nc = deck_controls(320., narrow.deck);
        assert!(((nc[1][0] + nc[1][2]) / 2. - 160.).abs() < 0.01);
        assert!(nc[2][2] < nc[3][0]);
        let mut prev_cx = f32::NEG_INFINITY;
        for step in -25..=25 {
            let v = step as f32 * 0.1;
            let (cx, scale, squeeze) = card_metrics(560., 218.4, v, 24.);
            assert!(cx > prev_cx);
            assert!(scale > 0.6 && scale <= 1.0);
            assert!(squeeze > 0.7 && squeeze <= 1.0);
            prev_cx = cx;
        }
    }
}
