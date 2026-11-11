//! Coverflow proportions follow CoverflowPlayer.tsx / coverflow.css, with an
//! app-owned card cache for rounded artwork, type and soft Y perspective.
use super::*;
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
    let wrapped = w < 480.;
    let deck_height = if wrapped { 70. } else { 48. };
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
    let deck_width = if wrapped {
        w - 56.
    } else {
        (w * 0.78).max(400.).min(w - 44.).min(680.)
    };
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
    let now = if wrapped {
        [deck[0] + 10., deck[1] + 38., deck[2] - 10., deck[3] - 6.]
    } else {
        [deck[0] + 132., deck[1] + 5., deck[2] - 96., deck[3] - 5.]
    };
    Some(Layout {
        deck,
        now,
        card_width,
        center_y: gallery_top + gallery_height / 2.,
        queue_rows,
    })
}
fn deck_controls(w: f32, d: [f32; 4]) -> [[f32; 4]; 5] {
    let by = d[1] + if w < 480. { 7. } else { 10. };
    std::array::from_fn(|i| {
        if w < 480. {
            let inner = d[2] - d[0] - 32.;
            let gap = (inner - 28. * 5.) / 4.;
            let x = d[0] + 16. + i as f32 * (28. + gap);
            [x, by, x + 28., by + 28.]
        } else {
            let x = match i {
                0 => d[0] + 20.,
                1 => d[0] + 56.,
                2 => d[0] + 92.,
                3 => d[2] - 84.,
                _ => d[2] - 48.,
            };
            [x, by, x + 28., by + 28.]
        }
    })
}
fn card_center(w: f32, card_width: f32, slot: i32, inset: f32) -> f32 {
    let outer = (card_width * 0.94).min((w / 2. - inset - card_width * 0.78 * 0.82 / 2.).max(0.));
    let travel = match slot.abs() {
        0 => 0.,
        1 => (card_width * 0.55).min(outer * 0.65),
        _ => outer,
    };
    w / 2. + slot.signum() as f32 * travel
}
impl State {
    pub(super) unsafe fn coverflow_content(&mut self, c: &Canvas, dc: HDC, w: f32, h: f32) {
        let Some(layout) = layout_with_top(w, h, self.config.queue, self.content_top(w, h)) else {
            self.compact_content(c, dc, w, h);
            return;
        };
        let selected = self.playback.queue.iter().position(|t| {
            t.selected
                || (!self.playback.video_id.is_empty() && t.video_id == self.playback.video_id)
        });
        for slot in [-2i32, 2, -1, 1, 0] {
            let scale = match slot.abs() {
                0 => 1.,
                1 => 0.91,
                _ => 0.78,
            };
            let squeeze = match slot.abs() {
                0 => 1.,
                1 => 0.89,
                _ => 0.82,
            };
            let card_w = layout.card_width * scale;
            let card_h = (layout.card_width + 52.) * scale;
            let cx = card_center(w, layout.card_width, slot, 24. / self.content_scale);
            let y = layout.center_y - card_h / 2. + slot.abs() as f32 * 2.;
            let r = [
                cx - card_w * squeeze / 2.,
                y,
                cx + card_w * squeeze / 2.,
                y + card_h,
            ];
            let (id, title, artist, art) = if slot == 0 {
                (
                    self.playback.video_id.clone(),
                    self.playback.title.clone(),
                    self.playback.artist.clone(),
                    self.playback.art.clone().unwrap_or_default(),
                )
            } else {
                let Some(index) = selected.map(|i| i as i32 + slot).filter(|i| *i >= 0) else {
                    continue;
                };
                let Some(track) = self.playback.queue.get(index as usize) else {
                    continue;
                };
                (
                    track.video_id.clone(),
                    track.title.clone(),
                    track.artist.clone(),
                    track.thumb.clone(),
                )
            };
            let key = format!(
                "card:{card_w:.2}:{card_h:.2}:{}:{slot}:{art}:{title}:{artist}:{}:{}",
                self.dpi,
                self.surface_color(),
                self.light
            );
            if !self.card_cache.contains_key(&key) {
                if let Some(image) = self.album_card(
                    r,
                    [card_w, card_h],
                    scale,
                    squeeze,
                    slot.signum(),
                    &art,
                    &title,
                    &artist,
                    w,
                ) {
                    if self.card_cache.len() >= 24 {
                        self.card_cache.clear();
                    }
                    self.card_cache.insert(key.clone(), image);
                }
            }
            if let Some(image) = self.card_cache.get(&key) {
                let pad = shadow_padding(scale, self.dpi) as f32 / self.dpi;
                c.image(image, [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad]);
            }
            if slot == 0 && id.is_empty() {
                self.buttons.push(Button {
                    rect: r,
                    action: Action::Toggle,
                });
            } else if !id.is_empty() {
                self.buttons.push(Button {
                    rect: r,
                    action: Action::Track(id),
                });
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
        // decks distribute all five controls evenly instead of crowding edges.
        let controls = deck_controls(w, d);
        let control = |i: usize| controls[i];
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
        let n = layout.now;
        c.round(
            n,
            16.,
            self.ink,
            if self.light { 7 } else { 12 },
            false,
            false,
        );
        let thumb = if w < 480. { 20. } else { 24. };
        let art_top = n[1] + if w < 480. { 2. } else { 5. };
        self.rounded_cover(
            c,
            self.playback.art.as_deref().unwrap_or(""),
            [n[0] + 4., art_top, n[0] + 4. + thumb, art_top + thumb],
            6.,
        );
        c.text(
            if self.playback.title.is_empty() {
                "Open your player"
            } else {
                &self.playback.title
            },
            [n[0] + 36., n[1], n[2] - 6., n[1] + 19.],
            12.,
            true,
            self.ink,
            0,
        );
        if w >= 480. {
            c.text(
                if self.playback.artist.is_empty() {
                    "Browse music / account"
                } else {
                    &self.playback.artist
                },
                [n[0] + 36., n[1] + 18., n[2] - 6., n[1] + 31.],
                10.,
                false,
                self.muted,
                0,
            );
        }
        self.timeline(c, [n[0] + 36., n[3] - 5., n[2] - 6., n[3] + 3.], false);
        self.media_button(c, control(3), Action::Browse, true);
        self.media_button(c, control(4), Action::Queue, true);
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
        let buffer = Buffer::plain(pw, ph, self.dpi)?;
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
        let dw = (pw as f32 * squeeze).round().max(1.) as usize;
        let sh = ph as usize;
        let sw = pw as usize;
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
        assert_eq!(l.deck[3] - l.deck[1], 48.);
        assert!(l.deck[2] - l.deck[0] < 560. - 100.);
        assert!((l.deck[0] + l.deck[2] - 560.).abs() < 0.01);
        assert!(l.now[0] > 132. && l.now[2] < 560. - 94.);
        assert!(l.center_y + (l.card_width + 52.) / 2. < l.deck[1]);
        let narrow = layout(320., 400., false).unwrap();
        assert_eq!(narrow.deck[3] - narrow.deck[1], 70.);
        assert!(narrow.now[1] >= narrow.deck[1] + 38.);
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
                    assert!(r[2] <= l.now[0] || l.now[2] <= r[0] || r[3] <= l.now[1]);
                }
                assert!(l.center_y + (l.card_width + 52.) / 2. <= l.deck[1] - 20.);
            }
        }
    }
}
