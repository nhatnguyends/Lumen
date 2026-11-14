//! Native-DIP layouts: artwork adapts; controls retain useful physical sizes.
use super::*;
pub(super) fn advance_spin(angle: f32, seconds: f32) -> f32 {
    (angle + seconds.clamp(0., 0.1) * 12.).rem_euclid(360.)
}

impl State {
    pub(super) unsafe fn sync_spin_timer(&mut self) {
        let mut animate = 1i32;
        SystemParametersInfoW(0x1042, 0, &mut animate as *mut i32 as _, 0);
        let enabled = animate != 0
            && self.playback.playing
            && !self.moving
            && IsWindowVisible(self.hwnd) != 0
            && matches!(self.config.design.as_str(), "orbit" | "disc" | "turntable");
        if enabled != self.spin_active {
            self.spin_active = enabled;
            self.spin_updated = Instant::now();
            if enabled {
                if self.anim_clock.is_none() {
                    self.anim_clock = Some(crate::motion::FrameClock::start());
                }
                SetTimer(self.hwnd, 93, 10, None);
            } else {
                KillTimer(self.hwnd, 93);
                if self.coverflow_anim.is_none() {
                    self.anim_clock = None;
                }
            }
        }
    }
    pub(super) unsafe fn responsive_content(&mut self, dc: HDC, w: f32, h: f32) {
        let c = Canvas::new(dc, self.dpi);
        let top = 54.;
        let small = w < 240. || h < 260.;
        let design = self.config.design.clone();
        if small {
            self.pocket_graphic(&c, dc, w, h, &design);
            return;
        }
        match design.as_str() {
            "compact" => {
                if w >= 340. && w > h * 1.25 {
                    let side = (h - top - 20.).min(w * 0.40).clamp(84., 236.);
                    let y = top + ((h - top - 16. - side) / 2.).max(0.);
                    let r = [24., y, 24. + side, y + side];
                    c.soft_shadow(r, 20., 42., 15., 7.);
                    self.rounded_cover(&c, self.playback.art.as_deref().unwrap_or(""), r, 20.);
                    let right_left = r[2] + 24.;
                    let right_w = w - 24. - right_left;
                    let block_y = top + ((h - top - 16. - 124.) / 2.).max(0.);
                    self.labels(&c, [right_left, block_y, w - 68., block_y + 48.], 18.);
                    let browse_rect = [w - 56., block_y + 8., w - 24., block_y + 40.];
                    if let Some(bg) = &self.background {
                        bg.restore_glass_card(dc, browse_rect, 16.);
                    }
                    self.media_button(&c, browse_rect, Action::Browse, true);
                    self.timeline(&c, [right_left, block_y + 56., w - 24., block_y + 74.], true);
                    self.fixed_transport_at(
                        &c,
                        dc,
                        right_left + right_w / 2.,
                        block_y + 86.,
                        38.,
                    );
                } else {
                    let side = (w - 48.).min(h - top - 168.).max(40.);
                    let y = top + ((h - top - 168. - side) / 2.).max(0.);
                    let r = [(w - side) / 2., y, (w + side) / 2., y + side];
                    c.soft_shadow(r, 20., 42., 15., 7.);
                    self.rounded_cover(&c, self.playback.art.as_deref().unwrap_or(""), r, 20.);
                    self.labels(&c, [24., r[3] + 10., w - 66., r[3] + 60.], 18.);
                    self.media_button(&c, [w - 56., r[3] + 14., w - 24., r[3] + 46.], Action::Browse, true);
                    self.timeline(&c, [24., h - 104., w - 24., h - 86.], true);
                    self.fixed_transport(&c, dc, w, h - 66., 40.);
                }
            }
            "turntable" => {
                let plinth = [16., top, w - 16., h - 16.];
                let browse_rect = [
                    plinth[2] - 42.,
                    plinth[1] + 10.,
                    plinth[2] - 12.,
                    plinth[1] + 40.,
                ];
                if w >= 340. {
                    let area_bottom = h - 22.;
                    let side = (area_bottom - top - 16.).min(w * 0.44).clamp(64., 236.);
                    let y = top + ((area_bottom - top - side) / 2.).max(0.);
                    let r = [28., y, 28. + side, y + side];
                    self.turntable_plinth(&c, dc, plinth);
                    self.vinyl(&c, dc, r);
                    let right_left = r[2] + 24.;
                    let right_w = w - 28. - right_left;
                    let block_y = y + ((side - 124.) / 2.).max(0.);
                    let label_right = if block_y < browse_rect[3] + 4. {
                        browse_rect[0] - 8.
                    } else {
                        w - 28.
                    };
                    self.labels(
                        &c,
                        [right_left, block_y, label_right, block_y + 46.],
                        17.,
                    );
                    self.timeline(&c, [right_left, block_y + 52., w - 28., block_y + 70.], true);
                    self.analog_transport_at(
                        &c,
                        dc,
                        right_left + right_w / 2.,
                        block_y + 82.,
                        38.,
                    );
                } else {
                    let side = (w - 56.).min(h - top - 162.).clamp(64., 200.);
                    let y = top + ((h - top - 156. - side) / 2.).max(0.);
                    let r = [(w - side) / 2., y, (w + side) / 2., y + side];
                    self.turntable_plinth(&c, dc, plinth);
                    self.vinyl(&c, dc, r);
                    self.labels(&c, [26., r[3] + 10., w - 26., r[3] + 56.], 17.);
                    self.timeline(&c, [26., h - 94., w - 26., h - 76.], true);
                    self.analog_transport_at(&c, dc, w / 2., h - 60., 38.);
                }
                if let Some(bg) = &self.background {
                    bg.restore_glass_card(dc, browse_rect, 15.);
                }
                self.media_button(&c, browse_rect, Action::Browse, true);
            }
            "orbit" => {
                if w >= 340. && w > h * 1.25 {
                    let side = (h - top - 20.).min(w * 0.40).clamp(84., 236.);
                    let y = top + ((h - top - 16. - side) / 2.).max(0.);
                    let r = [24., y, 24. + side, y + side];
                    c.soft_shadow(r, side / 2., 48., 18., 8.);
                    self.spinning_disc(dc, r, w, h);
                    let right_left = r[2] + 24.;
                    let right_w = w - 24. - right_left;
                    let block_y = top + ((h - top - 16. - 124.) / 2.).max(0.);
                    self.labels(&c, [right_left, block_y, w - 68., block_y + 48.], 18.);
                    let browse_rect = [w - 56., block_y + 8., w - 24., block_y + 40.];
                    if let Some(bg) = &self.background {
                        bg.restore_glass_card(dc, browse_rect, 16.);
                    }
                    self.media_button(&c, browse_rect, Action::Browse, true);
                    self.timeline(&c, [right_left, block_y + 56., w - 24., block_y + 74.], true);
                    self.fixed_transport_at(
                        &c,
                        dc,
                        right_left + right_w / 2.,
                        block_y + 86.,
                        38.,
                    );
                } else {
                    // The large printed disc is the focal point, behind a light
                    // overlap with the title rather than a tiny ornament above it.
                    let bottom = h - 124.;
                    let side = (w - 40.).min(bottom - top + 20.).max(48.);
                    let y = top + ((bottom - top - side) / 2.).max(0.);
                    let r = [(w - side) / 2., y, (w + side) / 2., y + side];
                    c.soft_shadow(r, side / 2., 48., 18., 8.);
                    self.spinning_disc(dc, r, w, h);
                    let label_y = (r[3] - 8.).min(h - 160.);
                    self.labels(&c, [28., label_y + 3., w - 68., label_y + 51.], 18.);
                    let browse_rect = [w - 56., label_y + 10., w - 24., label_y + 42.];
                    if let Some(bg) = &self.background {
                        bg.restore_glass_card(dc, browse_rect, 16.);
                    }
                    self.media_button(&c, browse_rect, Action::Browse, true);
                    self.timeline(&c, [28., h - 104., w - 28., h - 86.], true);
                    self.fixed_transport(&c, dc, w, h - 66., 40.);
                }
            }
            "disc" => {
                if w >= 340. {
                    let side = (w * 0.36)
                        .min(h - top - 20.)
                        .clamp(110., 224.);
                    let cy = top + (h - top) / 2.;
                    let disc = [20., cy - side / 2., 20. + side, cy + side / 2.];
                    let card_h = (side * 0.86).clamp(118., 176.).min(h - top - 16.);
                    let card = [
                        disc[0] + side * 0.56,
                        cy - card_h / 2.,
                        w - 20.,
                        cy + card_h / 2.,
                    ];
                    let radius = 24.;
                    c.soft_shadow(card, radius, if self.light { 0.16 } else { 0.28 }, 14., 5.);
                    if let Some(bg) = &self.background {
                        bg.restore_glass_card(dc, card, radius);
                    }
                    let card_fill = if self.light { 0xF6F5F3 } else { 0x1D222B };
                    c.round(
                        card,
                        radius,
                        card_fill,
                        if self.light { 216 } else { 145 },
                        false,
                        false,
                    );
                    c.coating(card, radius);
                    c.round(
                        card,
                        radius,
                        if self.light { 0xFFFFFF } else { 0xDCE6F5 },
                        if self.light { 128 } else { 36 },
                        true,
                        false,
                    );
                    c.soft_shadow(disc, side / 2., 52., 15., 6.);
                    self.spinning_disc(dc, disc, w, h);

                    let content_left = disc[2] + 18.;
                    let content_right = card[2] - 16.;
                    let browse_side = 28.;
                    let browse_rect = [
                        content_right - browse_side,
                        card[1] + 10.,
                        content_right,
                        card[1] + 10. + browse_side,
                    ];
                    if let Some(bg) = &self.background {
                        bg.restore_glass_card(dc, browse_rect, browse_side / 2.);
                    }
                    self.media_button(&c, browse_rect, Action::Browse, true);
                    let font = if w >= 420. { 17. } else { 15. };
                    self.labels(
                        &c,
                        [
                            content_left,
                            card[1] + 10.,
                            browse_rect[0] - 8.,
                            card[1] + 48.,
                        ],
                        font,
                    );
                    let btn_side = if w >= 420. { 36. } else { 32. };
                    let btn_y = card[3] - 10. - btn_side;
                    let seek_y = (card[1] + 52.).min(btn_y - 20.);
                    self.timeline(
                        &c,
                        [content_left, seek_y, content_right, seek_y + 16.],
                        card_h >= 138.,
                    );
                    self.fixed_transport_at(
                        &c,
                        dc,
                        (content_left + content_right) / 2.,
                        btn_y,
                        btn_side,
                    );
                } else {
                    let side = (w - 56.).min(h - top - 148.).clamp(64., 180.);
                    let y = top + ((h - top - 142. - side) / 2.).max(0.);
                    let disc = [(w - side) / 2., y, (w + side) / 2., y + side];
                    let card = [18., disc[3] - 14., w - 18., h - 18.];
                    let radius = 22.;
                    c.soft_shadow(card, radius, if self.light { 0.16 } else { 0.28 }, 12., 4.);
                    if let Some(bg) = &self.background {
                        bg.restore_glass_card(dc, card, radius);
                    }
                    let card_fill = if self.light { 0xF6F5F3 } else { 0x1D222B };
                    c.round(
                        card,
                        radius,
                        card_fill,
                        if self.light { 216 } else { 145 },
                        false,
                        false,
                    );
                    c.coating(card, radius);
                    c.soft_shadow(disc, side / 2., 48., 14., 6.);
                    self.spinning_disc(dc, disc, w, h);
                    let browse_rect = [card[2] - 42., card[1] + 18., card[2] - 14., card[1] + 46.];
                    if let Some(bg) = &self.background {
                        bg.restore_glass_card(dc, browse_rect, 14.);
                    }
                    self.media_button(&c, browse_rect, Action::Browse, true);
                    self.labels(
                        &c,
                        [card[0] + 14., card[1] + 18., browse_rect[0] - 8., card[1] + 62.],
                        16.,
                    );
                    self.timeline(&c, [card[0] + 14., h - 86., card[2] - 14., h - 68.], true);
                    self.fixed_transport_at(&c, dc, w / 2., h - 56., 32.);
                }
            }
            _ => (),
        }
    }

    unsafe fn labels(&self, c: &Canvas, r: [f32; 4], size: f32) {
        c.text_medium(
            if self.playback.title.is_empty() {
                "Your music"
            } else {
                &self.playback.title
            },
            [r[0], r[1], r[2], r[1] + 28.],
            size,
            self.ink,
            0,
        );
        c.text(
            &self.playback.artist,
            [r[0], r[1] + 30., r[2], r[3]],
            12.,
            false,
            self.muted,
            0,
        );
    }
    unsafe fn corner_open(&mut self, c: &Canvas, w: f32) {
        self.media_button(c, [w - 124., 12., w - 92., 44.], Action::Browse, true);
    }
    unsafe fn label_backing(&self, c: &Canvas, dc: HDC, r: [f32; 4]) {
        if let Some(bg) = &self.background {
            bg.restore_glass_card(dc, r, 20.);
        }
        c.round(r, 20., self.ink, 10, false, false);
    }
    unsafe fn fixed_transport(&mut self, c: &Canvas, dc: HDC, w: f32, y: f32, side: f32) {
        self.fixed_transport_at(c, dc, w / 2., y, side);
    }
    unsafe fn fixed_transport_at(
        &mut self,
        c: &Canvas,
        dc: HDC,
        center: f32,
        y: f32,
        side: f32,
    ) {
        let gap = 12.;
        for (i, action, enabled) in [
            (
                -1.,
                Action::Previous,
                self.playback.available && self.playback.can_previous,
            ),
            (0., Action::Toggle, true),
            (
                1.,
                Action::Next,
                self.playback.available && self.playback.can_next,
            ),
        ] {
            let x = center + i * (side + gap);
            let r = [x - side / 2., y, x + side / 2., y + side];
            if let Some(bg) = &self.background {
                bg.restore_glass_card(dc, r, side / 2.);
            }
            self.media_button(c, r, action, enabled);
        }
    }

    unsafe fn turntable_plinth(&self, c: &Canvas, dc: HDC, r: [f32; 4]) {
        if r[2] - r[0] < 40. || r[3] - r[1] < 40. {
            return;
        }
        let radius = 20.;
        c.soft_shadow(r, radius, if self.light { 0.14 } else { 0.28 }, 12., 4.);
        if let Some(bg) = &self.background {
            bg.restore_glass_card(dc, r, radius);
        }
        let deck_fill = if self.light { 0xF6F7FA } else { 0x161920 };
        c.round(
            r,
            radius,
            deck_fill,
            if self.light { 205 } else { 122 },
            false,
            false,
        );
        c.coating(r, radius);
        c.round(
            r,
            radius,
            if self.light { 0xFFFFFF } else { 0xE6EDF7 },
            if self.light { 122 } else { 36 },
            true,
            false,
        );
        let led = [r[0] + 14., r[1] + 14., r[0] + 20., r[1] + 20.];
        let led_color = if self.playback.playing { 0x34D399 } else { 0xF59E0B };
        c.round(led, 3., led_color, 217, false, false);
    }

    unsafe fn analog_transport_at(
        &mut self,
        c: &Canvas,
        dc: HDC,
        center: f32,
        y: f32,
        side: f32,
    ) {
        let gap = 12.;
        for (i, action, enabled) in [
            (
                -1.,
                Action::Previous,
                self.playback.available && self.playback.can_previous,
            ),
            (0., Action::Toggle, true),
            (
                1.,
                Action::Next,
                self.playback.available && self.playback.can_next,
            ),
        ] {
            let x = center + i * (side + gap);
            let r = [x - side / 2., y, x + side / 2., y + side];
            self.analog_button(c, dc, r, action, enabled);
        }
    }

    unsafe fn analog_button(
        &mut self,
        c: &Canvas,
        dc: HDC,
        r: [f32; 4],
        action: Action,
        enabled: bool,
    ) {
        let rad = (r[2] - r[0]) / 2.;
        let hovered = enabled && self.hover.as_ref() == Some(&action);
        let pressed = enabled && self.pressed.as_ref() == Some(&action);
        let primary = matches!(action, Action::Toggle);
        let well = [r[0] - 2., r[1] - 2., r[2] + 2., r[3] + 2.];
        c.round(
            well,
            rad + 2.,
            if self.light { 0x94A3B8 } else { 0x05070B },
            if self.light { 71 } else { 143 },
            false,
            false,
        );
        if let Some(bg) = &self.background {
            bg.restore_glass_card(dc, r, rad);
        }
        if !pressed {
            c.soft_shadow(r, rad, if self.light { 0.18 } else { 0.36 }, 6., 2.);
        }
        let cap_color = if primary {
            if self.light { 0xF8FAFC } else { 0x2A313D }
        } else if self.light {
            0xE5E9F0
        } else {
            0x1E242E
        };
        let cap_alpha = if !enabled {
            115
        } else if pressed {
            245
        } else if hovered {
            235
        } else {
            214
        };
        c.round(r, rad, cap_color, cap_alpha, false, false);
        let inner = [r[0] + 1., r[1] + 1., r[2] - 1., r[3] - 1.];
        c.round(
            inner,
            (rad - 1.).max(1.),
            if self.light { 0xFFFFFF } else { 0xD8E2F0 },
            if pressed {
                20
            } else if self.light {
                158
            } else {
                46
            },
            true,
            false,
        );
        c.round(
            r,
            rad,
            if self.light { 0x64748B } else { 0x090C11 },
            if self.light { 64 } else { 166 },
            true,
            false,
        );
        let cx = (r[0] + r[2]) / 2.;
        let cy = (r[1] + r[3]) / 2. + if pressed { 1. } else { 0. };
        let kind = match action {
            Action::Previous => icons::Kind::Previous,
            Action::Next => icons::Kind::Next,
            Action::Toggle => {
                if self.playback.playing {
                    icons::Kind::Pause
                } else {
                    icons::Kind::Play
                }
            }
            _ => return,
        };
        let icon_size = if primary { 15. } else { 13. };
        self.media_icons.draw(
            c,
            self.dpi,
            kind,
            [cx, cy],
            icon_size,
            self.ink,
            if enabled { 235 } else { 92 },
        );
        if enabled {
            self.buttons.push(Button { rect: r, action });
        }
    }
    unsafe fn vinyl(&self, c: &Canvas, dc: HDC, r: [f32; 4]) {
        let side = r[2] - r[0];
        c.soft_shadow(r, side / 2., 48., 15., 6.);
        // Spinning center album label
        let p = side * 0.29;
        let label_r = [r[0] + p, r[1] + p, r[2] - p, r[3] - p];
        let art = Canvas::new(dc, self.dpi);
        art.transform_about(
            [(r[0] + r[2]) / 2., (r[1] + r[3]) / 2.],
            1.,
            self.spin_angle,
        );
        self.rounded_cover(&art, self.playback.art.as_deref().unwrap_or(""), label_r, side * 0.21);
        drop(art);
        // Cached glossy vinyl body with concentric grooves & conical light reflection
        let px = (side * self.dpi).round().max(16.) as u32;
        let key = format!("vinyl:{px}");
        if !self.rounded_art.borrow().contains_key(&key) {
            if let Some(img) = build_vinyl_disc(px) {
                self.rounded_art.borrow_mut().insert(key.clone(), img);
            }
        }
        if let Some(img) = self.rounded_art.borrow().get(&key) {
            c.image(img, r);
        } else {
            c.round(r, side / 2., 0x191917, 255, false, false);
        }
        // 3D metallic center spindle
        let cx = (r[0] + r[2]) * 0.5;
        let cy = (r[1] + r[3]) * 0.5;
        let sp = (side * 0.036).max(2.5);
        c.round([cx - sp, cy - sp, cx + sp, cy + sp], sp, 0x202226, 220, false, false);
        c.round([cx - sp * 0.72, cy - sp * 0.72, cx + sp * 0.72, cy + sp * 0.72], sp * 0.72, 0xd9dde4, 245, false, false);
        c.round([cx - sp * 0.45, cy - sp * 0.45, cx + sp * 0.25, cy + sp * 0.25], sp * 0.35, 0xffffff, 220, false, false);
        // Detailed 3D tonearm
        draw_tonearm(c, r);
    }
    unsafe fn spinning_disc(&self, dc: HDC, r: [f32; 4], w: f32, h: f32) {
        let art = Canvas::new(dc, self.dpi);
        art.transform_about(
            [(r[0] + r[2]) / 2., (r[1] + r[3]) / 2.],
            1.,
            self.spin_angle,
        );
        self.disc_art(&art, r, [4., 4., w - 4., h - 4.], 0.);
    }
    unsafe fn pocket_graphic(&mut self, c: &Canvas, dc: HDC, w: f32, h: f32, design: &str) {
        let tall = w < 240. && h >= 210.;
        let button_side = if w < 200. {
            28.
        } else if h < 140. {
            30.
        } else {
            32.
        };
        let by = h - button_side - if h < 140. { 14. } else { 16. };
        if h < 140. {
            if w >= 300. {
                let side = 36.;
                let y = (h - 14. - side).max(48.);
                let r = [20., y, 20. + side, y + side];
                match design {
                    "turntable" => self.vinyl(c, dc, r),
                    "orbit" | "disc" => self.spinning_disc(dc, r, w, h),
                    _ => self.rounded_cover(c, self.playback.art.as_deref().unwrap_or(""), r, 10.),
                }
                let gap = 8.;
                let browse_inline = matches!(design, "compact" | "orbit");
                let right_edge = w - 22. - button_side / 2.;
                let next_cx = if browse_inline {
                    right_edge - (button_side + gap)
                } else {
                    right_edge
                };
                let center_cx = next_cx - (button_side + gap);
                let prev_cx = center_cx - (button_side + gap);
                let text_left = r[2] + 12.;
                let text_right = prev_cx - button_side / 2. - 14.;
                c.text_medium(
                    if self.playback.title.is_empty() {
                        "Your music"
                    } else {
                        &self.playback.title
                    },
                    [text_left, by - 4., text_right, by + 13.],
                    13.,
                    self.ink,
                    0,
                );
                self.timeline(c, [text_left, by + 16., text_right, by + 26.], false);
                let mut row_buttons = vec![
                    (
                        prev_cx,
                        Action::Previous,
                        self.playback.available && self.playback.can_previous,
                    ),
                    (center_cx, Action::Toggle, true),
                    (
                        next_cx,
                        Action::Next,
                        self.playback.available && self.playback.can_next,
                    ),
                ];
                if browse_inline {
                    row_buttons.push((right_edge, Action::Browse, true));
                }
                for (x, action, enabled) in row_buttons {
                    let br = [
                        x - button_side / 2.,
                        by,
                        x + button_side / 2.,
                        by + button_side,
                    ];
                    if let Some(bg) = &self.background {
                        bg.restore_glass_card(dc, br, button_side / 2.);
                    }
                    self.media_button(c, br, action, enabled);
                }
                if !browse_inline {
                    self.corner_open(c, w);
                }
                return;
            }
            if h >= 120. {
                let side = 28.;
                let r = [18., 46., 18. + side, 46. + side];
                match design {
                    "turntable" => self.vinyl(c, dc, r),
                    "orbit" | "disc" => self.spinning_disc(dc, r, w, h),
                    _ => self.rounded_cover(c, self.playback.art.as_deref().unwrap_or(""), r, 8.),
                }
                c.text_medium(
                    if self.playback.title.is_empty() {
                        "Your music"
                    } else {
                        &self.playback.title
                    },
                    [r[2] + 8., 44., w - 18., 62.],
                    12.,
                    self.ink,
                    0,
                );
            }
            self.timeline(c, [18., by - 14., w - 18., by - 4.], false);
            let browse_inline = matches!(design, "compact" | "orbit");
            let step = button_side + if browse_inline { 6. } else { 8. };
            let offsets: &[(f32, Action, bool)] = if browse_inline {
                &[
                    (
                        -1.5,
                        Action::Previous,
                        self.playback.available && self.playback.can_previous,
                    ),
                    (-0.5, Action::Toggle, true),
                    (
                        0.5,
                        Action::Next,
                        self.playback.available && self.playback.can_next,
                    ),
                    (1.5, Action::Browse, true),
                ]
            } else {
                &[
                    (
                        -1.,
                        Action::Previous,
                        self.playback.available && self.playback.can_previous,
                    ),
                    (0., Action::Toggle, true),
                    (
                        1.,
                        Action::Next,
                        self.playback.available && self.playback.can_next,
                    ),
                ]
            };
            for &(i, ref action, enabled) in offsets {
                let x = w / 2. + i * step;
                let br = [
                    x - button_side / 2.,
                    by,
                    x + button_side / 2.,
                    by + button_side,
                ];
                if let Some(bg) = &self.background {
                    bg.restore_glass_card(dc, br, button_side / 2.);
                }
                self.media_button(c, br, action.clone(), enabled);
            }
            if !browse_inline {
                if w >= 220. {
                    self.corner_open(c, w);
                } else {
                    self.media_button(c, [18., 12., 46., 40.], Action::Browse, true);
                }
            }
            return;
        }
        let seek_top = by - if w >= 200. { 24. } else { 18. };
        let side = if tall {
            (w - 44.).min(seek_top - 118.).min(240.).max(40.)
        } else {
            (seek_top - 64.).clamp(40., 96.).min(w - 40.)
        };
        let y = if tall {
            54. + ((seek_top - 54. - side - 58.) / 2.).max(0.)
        } else {
            54. + ((seek_top - 54. - side) / 2.).max(0.)
        };
        let left = if tall { (w - side) / 2. } else { 20. };
        let r = [left, y, left + side, y + side];
        match design {
            "turntable" => self.vinyl(c, dc, r),
            "orbit" | "disc" => self.spinning_disc(dc, r, w, h),
            _ => self.rounded_cover(c, self.playback.art.as_deref().unwrap_or(""), r, 10.),
        }
        let browse_by_title = matches!(design, "compact" | "orbit");
        if tall {
            let text_right = if browse_by_title { w - 54. } else { w - 18. };
            self.labels(c, [18., r[3] + 10., text_right, r[3] + 58.], 14.);
            if browse_by_title {
                self.media_button(
                    c,
                    [w - 48., r[3] + 14., w - 18., r[3] + 44.],
                    Action::Browse,
                    true,
                );
            }
        } else {
            let label_y = y + (side - 44.).max(0.) / 2.;
            let text_right = if browse_by_title { w - 58. } else { w - 20. };
            self.labels(c, [r[2] + 14., label_y, text_right, label_y + 48.], 14.);
            if browse_by_title {
                self.media_button(
                    c,
                    [w - 50., label_y + 6., w - 20., label_y + 36.],
                    Action::Browse,
                    true,
                );
            }
        }
        self.timeline(c, [18., seek_top, w - 18., by - 6.], w >= 200.);
        let step = button_side + if w >= 240. { 10. } else { 8. };
        for (i, action, enabled) in [
            (
                -1.,
                Action::Previous,
                self.playback.available && self.playback.can_previous,
            ),
            (0., Action::Toggle, true),
            (
                1.,
                Action::Next,
                self.playback.available && self.playback.can_next,
            ),
        ] {
            let x = w / 2. + i * step;
            let br = [
                x - button_side / 2.,
                by,
                x + button_side / 2.,
                by + button_side,
            ];
            if let Some(bg) = &self.background {
                bg.restore_glass_card(dc, br, button_side / 2.);
            }
            self.media_button(c, br, action, enabled);
        }
        if !browse_by_title {
            if w >= 220. {
                self.corner_open(c, w);
            } else {
                self.media_button(c, [18., 12., 46., 40.], Action::Browse, true);
            }
        }
    }
}

fn build_vinyl_disc(px: u32) -> Option<Image> {
    let size = px as usize;
    let radius = px as f32 * 0.5;
    let mut pixels = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - radius;
            let dy = y as f32 + 0.5 - radius;
            let dist = dx.hypot(dy);
            let nr = dist / radius;
            if nr > 1.0 {
                continue;
            }
            let outer_alpha = (radius - 1.0 - dist + 0.5).clamp(0., 1.);
            let inner_alpha = ((nr - 0.30) * radius * 0.8 + 0.5).clamp(0., 1.);
            let alpha = outer_alpha * inner_alpha;
            if alpha <= 0. {
                continue;
            }
            let angle = dy.atan2(dx);
            let grooves = (nr * 135.).sin() * 3.5;
            let groove_fade = ((nr - 0.33) / 0.10).clamp(0., 1.) * ((0.97 - nr) / 0.08).clamp(0., 1.);
            let lobe1 = (-0.78 - angle).cos().max(0.).powi(5);
            let lobe2 = (2.36 - angle).cos().max(0.).powi(5);
            let sheen = (lobe1 * 0.58 + lobe2 * 0.42) * groove_fade;
            let rim = if nr > 0.945 {
                (-2.35 - angle).cos().max(0.) * 34.
            } else if nr < 0.335 {
                (-2.35 - angle).cos().max(0.) * 18.
            } else {
                0.
            };
            let base = 22. + grooves;
            let r = ((base + sheen * 120. + rim).clamp(0., 255.) * alpha).round() as u8;
            let g = ((base + sheen * 114. + rim).clamp(0., 255.) * alpha).round() as u8;
            let b = ((base + sheen * 108. + rim).clamp(0., 255.) * alpha).round() as u8;
            let a = (alpha * 255.).round() as u8;
            let i = (y * size + x) * 4;
            pixels[i] = r;
            pixels[i + 1] = g;
            pixels[i + 2] = b;
            pixels[i + 3] = a;
        }
    }
    Image::from_rgba(px, px, pixels)
}

fn draw_tonearm(c: &Canvas, r: [f32; 4]) {
    let side = r[2] - r[0];
    let px = r[2] + (side * 0.04).max(3.);
    let py = r[1] + side * 0.13;
    let rear = (px + side * 0.035, py - side * 0.075);
    let elbow = (px - side * 0.025, py + side * 0.44);
    let tip = (r[2] - side * 0.16, r[1] + side * 0.78);
    let arm_w = (side * 0.026).clamp(2.0, 5.0);
    let base_r = (side * 0.085).clamp(5.0, 16.0);
    // Pivot base 3D rings
    c.round(
        [px - base_r + 1.5, py - base_r + 2.5, px + base_r + 1.5, py + base_r + 2.5],
        base_r,
        0x000000,
        85,
        false,
        false,
    );
    c.round(
        [px - base_r, py - base_r, px + base_r, py + base_r],
        base_r,
        0x26282d,
        245,
        false,
        false,
    );
    let mid_r = base_r * 0.74;
    c.round(
        [px - mid_r, py - mid_r, px + mid_r, py + mid_r],
        mid_r,
        0xd4d8e0,
        250,
        false,
        false,
    );
    let top_r = base_r * 0.46;
    c.round(
        [px - top_r - 0.6, py - top_r - 0.6, px + top_r - 0.6, py + top_r - 0.6],
        top_r,
        0xffffff,
        215,
        false,
        false,
    );
    // Counterweight rear stub
    c.line(&[(px, py), rear], 0x35383f, 240, arm_w * 1.4);
    // Arm shadow + cylindrical silver/white shaft + top specular highlight
    c.line(
        &[(px + 2., py + 3.), (elbow.0 + 2., elbow.1 + 3.), (tip.0 + 2., tip.1 + 3.)],
        0x000000,
        80,
        arm_w + 1.2,
    );
    c.line(&[(px, py), elbow, tip], 0xd8dce4, 245, arm_w);
    c.line(
        &[(px - 0.6, py - 0.6), (elbow.0 - 0.6, elbow.1 - 0.6), (tip.0 - 0.6, tip.1 - 0.6)],
        0xffffff,
        215,
        (arm_w * 0.38).max(1.),
    );
    // Headshell & red stylus indicator
    let head = (tip.0 - side * 0.055, tip.1 + side * 0.065);
    c.line(&[(tip.0 + 2., tip.1 + 3.), (head.0 + 2., head.1 + 3.)], 0x000000, 90, arm_w * 2.1);
    c.line(&[tip, head], 0xe8ecf2, 250, arm_w * 1.9);
    let dot = (arm_w * 0.55).max(1.4);
    c.round([head.0 - dot, head.1 - dot, head.0 + dot, head.1 + dot], dot, 0xff3b30, 240, false, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spin_is_time_based_wraps_and_limits_resume_jumps() {
        assert_eq!(advance_spin(12., 0.), 12.);
        assert!((advance_spin(359.5, 0.1) - 0.7).abs() < 0.001);
        assert_eq!(advance_spin(10., 5.), advance_spin(10., 0.1));
    }
}
