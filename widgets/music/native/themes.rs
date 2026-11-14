//! Reference-inspired layouts; all controls use the existing player actions.
use super::*;

fn orbit_disc_rect(w: f32, h: f32, top: f32) -> [f32; 4] {
    let y = top + 12.;
    let diameter = (w - 48.).min(h - 262. - y).max(48.);
    [(w - diameter) * 0.5, y, (w + diameter) * 0.5, y + diameter]
}

impl State {
    pub(super) unsafe fn reference_theme(&mut self, c: &Canvas, dc: HDC, w: f32, h: f32) {
        let top = self.content_top(w, h);
        let full = match self.config.design.as_str() {
            "orbit" => w >= 320. && h - top >= if self.config.queue { 406. } else { 350. },
            "disc" => w >= 560. && h - top >= if self.config.queue { 300. } else { 220. },
            _ => false,
        };
        if !full {
            self.compact_content(c, dc, w, h);
            return;
        }
        let saved = (self.ink, self.muted, self.light);
        match self.config.design.as_str() {
            "orbit" => self.orbit_theme(c, dc, w, h, top),
            "disc" => {
                self.ink = 0x292521;
                self.muted = 0x77716b;
                self.light = true;
                self.disc_theme(c, dc, w, h, top, saved);
            }
            _ => (),
        }
        (self.ink, self.muted, self.light) = saved;
    }

    unsafe fn milk_card(&self, c: &Canvas, dc: HDC, r: [f32; 4], radius: f32) {
        c.soft_shadow(r, radius, 35., 14., 6.);
        if let Some(bg) = &self.background {
            bg.restore_glass_card(dc, r, radius);
        }
        c.round(r, radius, 0xf6f5f3, 216, false, false);
        c.coating(r, radius);
    }

    unsafe fn orbit_theme(&mut self, c: &Canvas, dc: HDC, w: f32, h: f32, top: f32) {
        // Orbit is a layout on the shared outer material, not a second panel.
        let panel = [16., top, w - 16., h - 16.];
        let queue_rows = if self.config.queue {
            ((h - top - 354.) / 38.).floor().clamp(1., 3.) as usize
        } else {
            0
        };
        let h = h - queue_rows as f32 * 38.;
        let disc = orbit_disc_rect(w, h, top);
        // Explicit clipping also applies to CPU-composited images.
        self.disc_art(c, disc, panel, 34.);
        let y = h - 217.;
        for (i, height) in [10., 18., 13., 22., 14.].iter().enumerate() {
            let x = w / 2. - 17. + i as f32 * 8.;
            c.round(
                [x, y - 22. - height / 2., x + 3., y - 22. + height / 2.],
                1.5,
                self.muted,
                150,
                false,
                false,
            );
        }
        c.text(
            &self.playback.artist,
            [36., y, w - 36., y + 26.],
            17.,
            false,
            self.muted,
            1,
        );
        c.text(
            self.theme_title(),
            [32., y + 32., w - 32., y + 66.],
            22.,
            true,
            self.ink,
            1,
        );
        self.timeline(c, [48., h - 128., w - 48., h - 110.], true);
        if let Some(bg) = &self.background {
            bg.restore_glass_card(dc, [20., h - 92., w - 20., h - 22.], 32.);
        }
        self.transport(c, w, h - 78., 44.);
        self.media_button(c, [24., h - 73., 58., h - 39.], Action::Queue, true);
        self.media_button(
            c,
            [w - 58., h - 73., w - 24., h - 39.],
            Action::Browse,
            true,
        );
        if queue_rows > 0 {
            self.compact_queue(c, w - 12., h - 18., queue_rows);
        }
    }

    unsafe fn disc_theme(
        &mut self,
        c: &Canvas,
        dc: HDC,
        w: f32,
        h: f32,
        top: f32,
        outer_inks: (u32, u32, bool),
    ) {
        let queue_rows = if self.config.queue {
            ((h - top - 230.) / 38.).floor().clamp(1., 4.) as usize
        } else {
            0
        };
        let bottom = h;
        let h = h - queue_rows as f32 * 38.;
        let diameter = (h - top - 28.).min(w * 0.33).min(252.);
        let cy = top + (h - top) / 2.;
        let disc = [20., cy - diameter / 2., 20. + diameter, cy + diameter / 2.];
        let card = [
            20. + diameter * 0.64,
            cy - diameter * 0.43,
            w - 20.,
            cy + diameter * 0.43,
        ];
        self.milk_card(c, dc, card, 26.);
        c.soft_shadow(disc, diameter / 2., 54., 14., 5.);
        self.disc_art(c, disc, [8., top, w - 8., h - 8.], 0.);
        let left = disc[2] + 22.;
        let right = w - 40.;
        c.text_medium(
            self.theme_title(),
            [left, card[1] + 18., right - 82., card[1] + 44.],
            21.,
            self.ink,
            0,
        );
        c.text(
            &self.playback.artist,
            [left, card[1] + 47., right - 40., card[1] + 67.],
            13.,
            false,
            self.muted,
            0,
        );
        self.media_button(
            c,
            [right - 32., card[1] + 16., right, card[1] + 48.],
            Action::Browse,
            true,
        );
        self.media_button(
            c,
            [right - 74., card[1] + 16., right - 42., card[1] + 48.],
            Action::Queue,
            true,
        );
        let ink = self.ink;
        self.ink = 0xb52886;
        self.timeline(c, [left, card[1] + 80., right, card[1] + 92.], false);
        self.ink = ink;
        let y = card[3] - 54.;
        let center = (left + right) / 2.;
        for (offset, action, enabled) in [
            (
                -65.,
                Action::Previous,
                self.playback.available && self.playback.can_previous,
            ),
            (0., Action::Toggle, true),
            (
                65.,
                Action::Next,
                self.playback.available && self.playback.can_next,
            ),
        ] {
            self.media_button(
                c,
                [center + offset - 20., y, center + offset + 20., y + 40.],
                action,
                enabled,
            );
        }
        if queue_rows > 0 {
            (self.ink, self.muted, self.light) = outer_inks;
            self.compact_queue(c, w, bottom - queue_rows as f32 * 38. - 14., queue_rows);
        }
    }

    fn theme_title(&self) -> &str {
        if self.playback.title.is_empty() {
            "Your music"
        } else {
            &self.playback.title
        }
    }

    /// Printed photo disc, metallic hub and a transparent spindle hole. The
    /// cache tracks the decoded cover identity so late artwork replaces it.
    pub(super) unsafe fn disc_art(
        &self,
        c: &Canvas,
        r: [f32; 4],
        clip: [f32; 4],
        clip_radius: f32,
    ) {
        let size = r[2] - r[0];
        let px = (size * self.dpi).round().max(1.) as i32;
        let url = self.playback.art.as_deref().unwrap_or("");
        let identity = self.cover_image(url).map_or(0, Image::identity);
        let key = format!(
            "disc:{identity}:{px}:{}:{r:?}:{clip:?}:{clip_radius}",
            self.dpi
        );
        if !self.rounded_art.borrow().contains_key(&key) {
            if let Some(buffer) = Buffer::plain(px, px, self.dpi) {
                let batch = render::AlphaBatch::new(buffer.dc, self.dpi);
                self.cover(
                    &Canvas::new(buffer.dc, self.dpi),
                    url,
                    [0., 0., size, size],
                    1.,
                );
                drop(batch);
                let mut pixels = buffer.snapshot();
                for y in 0..px {
                    for x in 0..px {
                        let i = ((y * px + x) * 4) as usize;
                        let dx = (x as f32 + 0.5) / px as f32 - 0.5;
                        let dy = (y as f32 + 0.5) / px as f32 - 0.5;
                        let d = (dx * dx + dy * dy).sqrt();
                        let panel_mask = (0.5
                            - crate::compositor::distance(
                                (r[0] - clip[0]) * self.dpi + x as f32 + 0.5,
                                (r[1] - clip[1]) * self.dpi + y as f32 + 0.5,
                                (clip[2] - clip[0]) * self.dpi,
                                (clip[3] - clip[1]) * self.dpi,
                                clip_radius * self.dpi,
                                0.,
                            ))
                        .clamp(0., 1.);
                        let mask = panel_mask
                            * ((0.5 - d) * px as f32 + 0.5).clamp(0., 1.)
                            * ((d - 0.035) * px as f32 + 0.5).clamp(0., 1.);
                        if d < 0.145 {
                            let angle = dy.atan2(dx);
                            let ring = ((d * 350.).sin() * 7. + (angle + 2.2).cos() * 23. + 191.)
                                .clamp(100., 240.) as u8;
                            pixels[i..i + 4].copy_from_slice(&[
                                ring.saturating_add(6),
                                ring,
                                ring,
                                255,
                            ]);
                        }
                        for channel in 0..4 {
                            pixels[i + channel] = (pixels[i + channel] as f32 * mask).round() as u8;
                        }
                        pixels.swap(i, i + 2);
                    }
                }
                if let Some(image) = Image::from_rgba(px as u32, px as u32, pixels) {
                    let mut cache = self.rounded_art.borrow_mut();
                    if cache.len() >= 24 {
                        cache.clear();
                    }
                    cache.insert(key.clone(), image);
                }
            }
        }
        if let Some(image) = self.rounded_art.borrow().get(&key) {
            c.clipped_image(image, r, clip);
        }
    }
}

#[cfg(test)]
mod spacing_tests {
    use super::*;
    #[test]
    fn orbit_artwork_keeps_chrome_and_equalizer_clearance() {
        for (w, h, top) in [
            (320., 364., 14.),
            (400., 480., 14.),
            (560., 620., 54.),
            (800., 800., 54.),
        ] {
            let r = orbit_disc_rect(w, h, top);
            assert!(r[0] >= 24. && r[2] <= w - 24. && r[1] >= top + 12.);
            assert!(r[3] <= h - 262. + 0.01);
            assert_eq!(r[2] - r[0], r[3] - r[1]);
        }
    }
}
