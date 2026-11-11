use super::*;
use crate::render::Canvas;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Browse,
    Library,
    Close,
    Settings,
    Queue,
    Previous,
    Toggle,
    Next,
    Seek,
    Track(String),
}
pub struct Button {
    pub rect: [f32; 4],
    pub action: Action,
}
pub fn contains(r: [f32; 4], p: [f32; 2]) -> bool {
    p[0] >= r[0] && p[0] <= r[2] && p[1] >= r[1] && p[1] <= r[3]
}
fn visible_hit_rect(r: [f32; 4], scale: f32, w: f32, h: f32, inset: f32) -> Option<[f32; 4]> {
    let mut r = r.map(|v| v * scale);
    r[0] = r[0].max(inset);
    r[1] = r[1].max(inset);
    r[2] = r[2].min(w - inset);
    r[3] = r[3].min(h - inset);
    (r[2] > r[0] && r[3] > r[1]).then_some(r)
}
fn queue_capacity(height: f32, top: f32) -> usize {
    ((height - top - 12.).max(0.) / 38.).floor() as usize
}

struct TinyLayout {
    art: [f32; 4],
    title: [f32; 4],
    artist: [f32; 4],
    play: [f32; 4],
}
fn tiny_layout(w: f32, h: f32) -> TinyLayout {
    if w < 220. && h >= 220. {
        let side = (w - 40.).min(h - 160.).min(128.);
        let top = 54. + ((h - side - 160.) / 2.).max(0.);
        let y = top + side + 10.;
        TinyLayout {
            art: [(w - side) / 2., top, (w + side) / 2., top + side],
            title: [18., y, w - 18., y + 22.],
            artist: [18., y + 24., w - 18., y + 40.],
            play: [w / 2. - 18., y + 48., w / 2. + 18., y + 84.],
        }
    } else {
        let y = (h - 50.).min(112.);
        let horizontal = w >= 220.;
        TinyLayout {
            art: if horizontal {
                [18., y, 54., y + 36.]
            } else {
                [18., 14., 50., 46.]
            },
            title: [if horizontal { 66. } else { 18. }, y, w - 66., y + 22.],
            artist: [
                if horizontal { 66. } else { 18. },
                y + 23.,
                w - 66.,
                y + 36.,
            ],
            play: [w - 54., y, w - 18., y + 36.],
        }
    }
}

/// Keep a design's composition down to half its natural content size. The
/// toolbar keeps its physical size and gets an independent reserved top band.
pub(super) fn design_scale(design: &str, w: f32, h: f32, queue: bool) -> Option<f32> {
    if w < 184. || h < 128. {
        return None;
    }
    let (width, body) = match design {
        "coverflow" => (320., if queue { 300. } else { 244. }),
        "orbit" => (320., if queue { 408. } else { 352. }),
        "disc" => (560., if queue { 302. } else { 222. }),
        "sky" => (420., 422.),
        "sleeve" | "turntable" => (300., 370.),
        _ => (240., 172.),
    };
    let scale = (w / (width + 1.)).min((h - 54.) / (body + 1.)).min(1.);
    (scale >= 0.5).then_some(scale)
}
pub(super) fn layout_plan(design: &str, w: f32, h: f32, queue: bool) -> Option<(f32, bool)> {
    design_scale(design, w, h, queue)
        .map(|scale| (scale, queue))
        .or_else(|| {
            queue
                .then(|| design_scale(design, w, h, false))
                .flatten()
                .map(|scale| (scale, false))
        })
}

impl State {
    pub unsafe fn content(&mut self, dc: HDC, w: f32, h: f32) {
        self.buttons.clear();
        self.control_plates.clear();
        if let Some((scale, draw_queue)) = layout_plan(&self.config.design, w, h, self.config.queue)
        {
            let dpi = self.dpi;
            let queue = self.config.queue;
            let bg_dpi = self.background.as_ref().map(|b| b.dpi);
            self.content_scale = scale;
            self.config.queue = draw_queue;
            self.dpi = dpi * scale;
            if let Some(bg) = &mut self.background {
                bg.dpi = self.dpi;
            }
            self.design_content(dc, w / scale, h / scale);
            self.dpi = dpi;
            self.content_scale = 1.;
            self.config.queue = queue;
            if let (Some(bg), Some(previous)) = (&mut self.background, bg_dpi) {
                bg.dpi = previous;
            }
            for (r, radius) in &mut self.control_plates {
                *r = r.map(|v| v * scale);
                *radius *= scale;
            }
            self.buttons.retain_mut(|button| {
                let inset = if matches!(button.action, Action::Track(_)) {
                    0.
                } else {
                    12.
                };
                if let Some(rect) = visible_hit_rect(button.rect, scale, w, h, inset) {
                    button.rect = rect;
                    true
                } else {
                    false
                }
            });
        } else {
            self.tiny_content(&Canvas::new(dc, self.dpi), dc, w, h);
        }
        self.draw_header(dc, w);
    }
    pub(super) fn content_top(&self, w: f32, h: f32) -> f32 {
        if self.content_scale < 1. {
            54. / self.content_scale
        } else {
            crate::settings_ui::widget_content_top(w, h)
        }
    }
    unsafe fn design_content(&mut self, dc: HDC, w: f32, h: f32) {
        let c = Canvas::new(dc, self.dpi);
        if matches!(self.config.design.as_str(), "orbit" | "disc" | "sky") {
            self.reference_theme(&c, dc, w, h);
            return;
        }
        if self.config.design == "coverflow" && h >= 300. && w >= 280. {
            self.coverflow_content(&c, dc, w, h);
            return;
        }
        if h < 380. || w < 300. || self.config.design == "lumen" {
            self.compact_content(&c, dc, w, h);
            return;
        }
        if self.content_scale >= 1. {
            self.button(&c, [w - 120., 12., w - 88., 44.], Action::Queue, "≡", true);
        }
        let queue_h = if self.config.queue {
            ((h - 300.) / 110.).floor().clamp(1., 5.) * 38. + 12.
        } else {
            0.
        };
        let controls_y = h - queue_h - 76.;
        let seek_y = controls_y - 44.;
        let content_top = self.content_top(w, h);
        if self.config.design == "turntable" {
            let side = ((w - 72.) * 0.52).min(seek_y - 82.).max(64.);
            let r = [24., content_top + 12., 24. + side, content_top + 12. + side];
            c.soft_shadow(r, side / 2., 0.55, 12., 6.);
            c.round(r, side / 2., 0x191917, 255, false, false);
            for inset in [5., 10., 16., 23., 30.] {
                if side > inset * 2. {
                    c.round(
                        [r[0] + inset, r[1] + inset, r[2] - inset, r[3] - inset],
                        side / 2.,
                        self.ink,
                        30,
                        true,
                        false,
                    );
                }
            }
            let pad = side * 0.29;
            self.rounded_cover(
                &c,
                self.playback.art.as_deref().unwrap_or(""),
                [r[0] + pad, r[1] + pad, r[2] - pad, r[3] - pad],
                side * 0.21,
            );
            c.round(
                [
                    r[0] + side * 0.47,
                    r[1] + side * 0.47,
                    r[0] + side * 0.53,
                    r[1] + side * 0.53,
                ],
                side * 0.03,
                0xd5d5d5,
                255,
                false,
                false,
            );
            c.line(
                &[
                    (r[2] + 12., r[1]),
                    (r[2] - 3., r[1] + side * 0.58),
                    (r[2] - side * 0.15, r[1] + side * 0.7),
                ],
                self.muted,
                230,
                4.,
            );
            let x = r[2] + 20.;
            let y = (r[1] + side / 2. - 48.).max(48.);
            c.text(
                "STEREO / SIDE A",
                [x, y, w - 20., y + 18.],
                9.,
                false,
                self.muted,
                0,
            );
            c.text(
                self.track_title(),
                [x, y + 22., w - 88., y + 56.],
                20.,
                false,
                self.ink,
                0,
            );
            c.text(
                &self.playback.artist,
                [x, y + 60., w - 88., y + 82.],
                12.,
                false,
                self.muted,
                0,
            );
        } else {
            let side = (seek_y - content_top - 76.).min(280.).min(w - 64.).max(56.);
            let r = [
                (w - side) / 2.,
                content_top + 4.,
                (w + side) / 2.,
                content_top + 4. + side,
            ];
            c.soft_shadow(r, 22., 0.35, 18., 8.);
            self.rounded_cover(&c, self.playback.art.as_deref().unwrap_or(""), r, 22.);
            let y = r[3] + 12.;
            c.text_medium(
                self.track_title(),
                [32., y, w - 88., y + 28.],
                21.,
                self.ink,
                0,
            );
            c.text(
                &self.playback.artist,
                [32., y + 29., w - 88., y + 51.],
                13.,
                false,
                self.muted,
                0,
            );
        }
        // Browse is an ordinary media action, beside metadata rather than a
        // second full-width text row under the transport.
        self.media_button(
            &c,
            [w - 76., seek_y - 66., w - 32., seek_y - 22.],
            Action::Browse,
            true,
        );

        if let Some(bg) = &self.background {
            bg.restore_glass_card(dc, [20., controls_y - 8., w - 20., controls_y + 56.], 28.);
        }
        self.timeline(&c, [30., seek_y, w - 30., seek_y + 18.], true);
        self.transport(&c, w, controls_y, 48.);
        if queue_h > 0. {
            self.compact_queue(&c, w, h - queue_h, ((queue_h - 8.) / 38.) as usize);
        }
    }
    unsafe fn draw_header(&mut self, dc: HDC, w: f32) {
        use crate::settings_ui::{widget_header_hits, HeaderAction};
        // Match the visual stacking order: chrome is drawn and hit-tested last,
        // so compact Coverflow cards never intercept Settings/Hide.
        for (action, rect) in widget_header_hits(w) {
            self.buttons.push(Button {
                rect,
                action: match action {
                    HeaderAction::Settings => Action::Settings,
                    HeaderAction::Hide => Action::Close,
                },
            });
        }
        let action = |a: &Action| match a {
            Action::Settings => Some(crate::settings_ui::HeaderAction::Settings),
            Action::Close => Some(crate::settings_ui::HeaderAction::Hide),
            _ => None,
        };
        crate::settings_ui::draw_widget_header(
            dc,
            self.hwnd,
            self.dpi,
            w,
            "",
            self.ink,
            self.hover.as_ref().and_then(action),
            self.pressed.as_ref().and_then(action),
        );
    }
    fn track_title(&self) -> &str {
        if self.playback.title.is_empty() {
            "Your music"
        } else {
            &self.playback.title
        }
    }
    unsafe fn tiny_content(&mut self, c: &Canvas, dc: HDC, w: f32, h: f32) {
        let l = tiny_layout(w, h);
        self.rounded_cover(c, self.playback.art.as_deref().unwrap_or(""), l.art, 9.);
        c.text_medium(self.track_title(), l.title, 13., self.ink, 0);
        c.text(&self.playback.artist, l.artist, 11., false, self.muted, 0);
        if let Some(bg) = &self.background {
            bg.restore_glass_card(dc, l.play, 18.);
        }
        self.media_button(c, l.play, Action::Toggle, true);
    }
    pub(super) unsafe fn compact_content(&mut self, c: &Canvas, dc: HDC, w: f32, h: f32) {
        let body = self.content_top(w, h);
        let bump = if h >= 220. && (!self.config.queue || h >= 270.) {
            20.
        } else {
            0.
        };
        let rows = if self.config.queue && h >= 232. {
            queue_capacity(h, body + 164. + bump)
        } else {
            0
        };
        let reserve = if rows > 0 {
            rows as f32 * 38. + 12.
        } else {
            0.
        };
        let extra =
            if self.config.design == "lumen" && crate::settings_ui::widget_title_visible(w, h) {
                ((h - body - 164. - bump - reserve - 12.) / 2.).max(0.)
            } else {
                0.
            };
        let top = body + extra;
        let right = w - if crate::settings_ui::widget_title_visible(w, h) {
            24.
        } else {
            96.
        };
        if self.config.queue && h < 232. {
            self.compact_queue(c, right + 16., top + 4., 1);
        } else {
            let side = 40. + bump * 0.6;
            self.rounded_cover(
                c,
                self.playback.art.as_deref().unwrap_or(""),
                [20., top + 4., 20. + side, top + 4. + side],
                10.,
            );
            let x = 32. + side;
            c.text_medium(
                self.track_title(),
                [x, top + 3., right, top + 28.],
                15.,
                self.ink,
                0,
            );
            c.text(
                if self.error.is_empty() {
                    &self.playback.artist
                } else {
                    &self.error
                },
                [x, top + 29., right, top + 47.],
                11.,
                false,
                self.muted,
                0,
            );
        }
        if let Some(bg) = &self.background {
            bg.restore_glass_card(dc, [20., top + 52. + bump, w - 20., top + 156. + bump], 22.);
        }
        self.timeline(c, [22., top + 54. + bump, w - 22., top + 74. + bump], true);
        self.transport(c, w, top + 82. + bump, 40.);
        self.button(
            c,
            [20., top + 128. + bump, w - 70., top + 154. + bump],
            Action::Browse,
            "Browse music",
            true,
        );
        self.media_button(
            c,
            [w - 62., top + 120. + bump, w - 22., top + 154. + bump],
            Action::Queue,
            true,
        );
        if rows > 0 {
            self.compact_queue(c, w, top + 164. + bump, rows);
        }
    }
    pub(super) unsafe fn timeline(&mut self, c: &Canvas, r: [f32; 4], times: bool) {
        let y = (r[1] + r[3]) / 2.;
        c.round(
            [r[0], y - 1.5, r[2], y + 1.5],
            1.5,
            self.ink,
            38,
            false,
            false,
        );
        let fraction = if self.playback.duration > 0. {
            (self.position() / self.playback.duration).clamp(0., 1.) as f32
        } else {
            0.
        };
        if fraction > 0. {
            c.round(
                [r[0], y - 1.5, r[0] + (r[2] - r[0]) * fraction, y + 1.5],
                1.5,
                self.ink,
                230,
                false,
                false,
            );
        }
        if self.playback.can_seek {
            self.buttons.push(Button {
                rect: r,
                action: Action::Seek,
            });
        }
        if times {
            c.text(
                &model::time(self.position()),
                [r[0], r[3] - 7., r[0] + 60., r[3] + 8.],
                9.,
                false,
                self.muted,
                0,
            );
            c.text(
                &model::time(self.playback.duration),
                [r[2] - 60., r[3] - 7., r[2], r[3] + 8.],
                9.,
                false,
                self.muted,
                2,
            );
        }
    }
    pub(super) unsafe fn transport(&mut self, c: &Canvas, w: f32, y: f32, height: f32) {
        self.media_button(
            c,
            [w / 2. - 94., y, w / 2. - 50., y + height],
            Action::Previous,
            self.playback.available && self.playback.can_previous,
        );
        self.media_button(
            c,
            [w / 2. - height / 2., y, w / 2. + height / 2., y + height],
            Action::Toggle,
            true,
        );
        self.media_button(
            c,
            [w / 2. + 50., y, w / 2. + 94., y + height],
            Action::Next,
            self.playback.available && self.playback.can_next,
        );
    }
    pub(super) unsafe fn compact_queue(&mut self, c: &Canvas, w: f32, top: f32, rows: usize) {
        let rows = self.background.as_ref().map_or(rows, |bg| {
            rows.min(queue_capacity(bg.height as f32 / self.dpi, top))
        });
        if rows == 0 {
            return;
        }
        self.queue_offset = self
            .queue_offset
            .min(self.playback.queue.len().saturating_sub(rows));
        if self.playback.queue.is_empty() {
            c.text(
                "The player queue appears here",
                [18., top, w - 18., top + 36.],
                11.,
                false,
                self.muted,
                1,
            );
        }
        for (n, track) in self
            .playback
            .queue
            .iter()
            .skip(self.queue_offset)
            .take(rows)
            .cloned()
            .enumerate()
        {
            let y = top + n as f32 * 38.;
            let r = [16., y, w - 16., y + 35.];
            if track.selected {
                c.round(r, 9., self.ink, 20, false, false);
            }
            self.rounded_cover(c, &track.thumb, [20., y + 3., 48., y + 31.], 6.);
            c.text(
                &track.title,
                [58., y, w - 24., y + 20.],
                11.,
                true,
                self.ink,
                0,
            );
            c.text(
                &track.artist,
                [58., y + 20., w - 24., y + 34.],
                9.,
                false,
                self.muted,
                0,
            );
            self.buttons.push(Button {
                rect: r,
                action: Action::Track(track.video_id),
            });
        }
        if self.playback.queue.len() > rows {
            let height = rows as f32 * 38. - 5.;
            let thumb = (height * rows as f32 / self.playback.queue.len() as f32).max(8.);
            let offset = (height - thumb) * self.queue_offset as f32
                / self.playback.queue.len().saturating_sub(rows).max(1) as f32;
            c.round(
                [w - 12., top + offset, w - 9., top + offset + thumb],
                1.5,
                self.ink,
                100,
                false,
                false,
            );
        }
    }
    pub(super) unsafe fn cover(&self, c: &Canvas, url: &str, r: [f32; 4], alpha: f32) {
        if let Some(image) = self.cover_image(url) {
            let [iw, ih] = image.pixel_size();
            let scale = ((r[2] - r[0]) / iw.max(1) as f32).max((r[3] - r[1]) / ih.max(1) as f32);
            let (width, height) = (iw as f32 * scale, ih as f32 * scale);
            let (cx, cy) = ((r[0] + r[2]) / 2., (r[1] + r[3]) / 2.);
            c.clipped_image_alpha(
                image,
                [
                    cx - width / 2.,
                    cy - height / 2.,
                    cx + width / 2.,
                    cy + height / 2.,
                ],
                r,
                alpha,
            );
        } else {
            c.round(r, 12., 0x634b46, 150, false, false);
            c.text("♪", r, (r[2] - r[0]) * 0.3, false, self.muted, 1);
        }
    }
    pub(super) unsafe fn button(
        &mut self,
        c: &Canvas,
        r: [f32; 4],
        action: Action,
        text: &str,
        enabled: bool,
    ) {
        if action == Action::Queue
            || action == Action::Toggle
            || action == Action::Next
            || action == Action::Previous
        {
            self.media_button(c, r, action, enabled);
            return;
        }
        let hover = self.hover.as_ref() == Some(&action);
        if hover || matches!(action, Action::Toggle | Action::Browse) {
            self.control_plates.push((r, 15.));
        }
        let down = self.pressed.as_ref() == Some(&action) && hover;
        if enabled && (hover || action == Action::Toggle) {
            c.hover_surface(
                r,
                15.,
                if down {
                    0.9
                } else if hover {
                    0.65
                } else {
                    0.25
                },
                self.light,
            );
        }
        if action == Action::Browse {
            c.round(
                r,
                (r[3] - r[1]) / 2.,
                self.ink,
                if hover { 18 } else { 8 },
                false,
                false,
            );
        }
        c.text_alpha(
            text,
            r,
            if text.chars().count() < 4 { 23. } else { 12. },
            false,
            self.ink,
            1,
            if enabled { 255 } else { 69 },
        );
        if enabled {
            self.buttons.push(Button { rect: r, action });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_queue_does_not_force_a_row_beyond_scaled_viewport() {
        let (scale, _) = layout_plan("lumen", 240., 180., true).unwrap();
        let height = 180. / scale;
        let top = 54. / scale + 164.;
        assert_eq!(queue_capacity(height, top), 0);
        for height in [180., 247., 320., 600.] {
            for top in [18., 164., 238., 500.] {
                let rows = queue_capacity(height, top);
                if rows > 0 {
                    assert!(top + rows as f32 * 38. <= height - 12.);
                }
            }
        }
        assert!(visible_hit_rect([16., top, 300., top + 35.], scale, 240., 180., 0.).is_some());
        assert_eq!(
            visible_hit_rect(
                [16., height + 1., 300., height + 36.],
                scale,
                240.,
                180.,
                0.
            ),
            None
        );
        let clipped = visible_hit_rect(
            [16., height - 10., 300., height + 25.],
            scale,
            240.,
            180.,
            0.,
        )
        .unwrap();
        assert_eq!(clipped[3], 180.);
    }
    #[test]
    fn themes_scale_in_place_until_pocket_size_and_collapse_queue_first() {
        for design in model::DESIGNS {
            for queue in [false, true] {
                let (scale, _) = layout_plan(design, 320., 340., queue).unwrap();
                assert!(scale >= 0.5 && scale <= 1., "{design}");
                assert!(layout_plan(design, 168., 104., queue).is_none());
            }
        }
        let (scale, queue) = layout_plan("orbit", 240., 240., true).unwrap();
        assert!(scale >= 0.5 && !queue);
        assert!(layout_plan("coverflow", 240., 180., false).is_some());
        assert!(layout_plan("disc", 300., 200., false).is_some());
        assert!(layout_plan("sky", 280., 300., false).is_some());
    }
    #[test]
    fn tiny_layout_preserves_readable_content_and_clear_header_targets() {
        for [w, h] in [
            [168., 104.],
            [180., 110.],
            [219., 179.],
            [168., 220.],
            [168., 1000.],
            [900., 104.],
        ] {
            let l = tiny_layout(w, h);
            let overlaps =
                |a: [f32; 4], b: [f32; 4]| a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
            let boxes = [l.art, l.title, l.artist, l.play];
            for (i, r) in boxes.iter().enumerate() {
                assert!(
                    r[0] >= 12. && r[1] >= 12. && r[2] <= w - 12. && r[3] <= h - 12.,
                    "{w}x{h}: {r:?}"
                );
                assert!(r[2] > r[0] && r[3] > r[1]);
                for (_, header) in crate::settings_ui::widget_header_hits(w) {
                    assert!(!overlaps(*r, header), "content/header collision {w}x{h}");
                }
                for other in &boxes[i + 1..] {
                    assert!(!overlaps(*r, *other));
                }
            }
            assert!(l.title[2] - l.title[0] >= 80.);
            assert!(l.play[2] - l.play[0] >= 36.);
        }
    }
    #[test]
    fn hit_targets_have_stable_edges() {
        assert!(contains([0., 0., 40., 40.], [20., 20.]));
        assert!(!contains([0., 0., 40., 40.], [41., 20.]));
    }
}
