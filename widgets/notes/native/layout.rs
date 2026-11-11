//! Pure adaptive chrome geometry, also used by native hit-testing.
#[derive(Clone, Copy)]
pub struct Chrome {
    pub w: f32,
    pub h: f32,
    pub search_open: bool,
}
impl Chrome {
    pub fn tiny(self) -> bool {
        self.w < 240. || self.h < 220.
    }
    pub fn search_icon(self) -> bool {
        self.tiny() && !self.search_open
    }
    pub fn large(self) -> bool {
        crate::settings_ui::widget_title_visible(self.w, self.h)
    }
    pub fn add(self) -> [f32; 4] {
        if self.tiny() {
            return [12., 12., 40., 40.];
        }
        let y = if self.large() { 46. } else { 16. };
        [18., y, 54., y + 36.]
    }
    pub fn search(self) -> [f32; 4] {
        if self.search_icon() {
            return [44., 12., 72., 40.];
        }
        if self.tiny() {
            return [12., 50., self.w - 12., 78.];
        }
        let y = if self.large() { 48. } else { 18. };
        if self.w < 320. && self.search_open {
            return [20., y + 40., self.w - 20., y + 72.];
        }
        [
            66.,
            y,
            self.w - if self.large() { 24. } else { 96. },
            y + 32.,
        ]
    }
    pub fn content_top(self) -> f32 {
        if self.search_icon() {
            52.
        } else {
            self.search()[3] + if self.tiny() { 8. } else { 16. }
        }
    }
    pub fn viewport(self) -> [f32; 2] {
        let top = self.content_top();
        let bottom = self.h - if self.tiny() { 42. } else { 58. };
        if bottom - top < 52. {
            return [top, top];
        }
        [top - 2., bottom]
    }
    pub fn card_height(self, base: f32) -> f32 {
        if self.w < 360. || self.h < 360. {
            base.min((self.viewport()[1] - self.content_top()).max(52.))
        } else {
            base
        }
    }
    pub fn card_inset(self, design: &str) -> f32 {
        if self.tiny() {
            12.
        } else if (self.w < 360. || self.h < 360.) && design != "notebook" {
            16.
        } else {
            24.
        }
    }
    pub fn folder(self) -> [f32; 4] {
        if self.tiny() {
            [self.w - 40., self.h - 36., self.w - 12., self.h - 8.]
        } else {
            [self.w - 128., self.h - 46., self.w - 20., self.h - 14.]
        }
    }
    pub fn undo(self) -> [f32; 4] {
        if self.tiny() {
            [self.w - 72., self.h - 36., self.w - 44., self.h - 8.]
        } else {
            [self.w - 164., self.h - 46., self.w - 132., self.h - 14.]
        }
    }
}
pub fn palette(card: [f32; 4], viewport: [f32; 2]) -> [f32; 4] {
    let cols = if card[2] - card[0] < 200. { 4. } else { 7. };
    let height = if cols == 4. { 60. } else { 36. };
    let y = (card[1] + 34.).clamp(
        viewport[0] + 2.,
        (viewport[1] - height - 2.).max(viewport[0] + 2.),
    );
    [card[0] + 8., y, card[0] + 24. * cols + 24., y + height]
}
pub fn palette_columns(r: [f32; 4]) -> usize {
    ((r[2] - r[0] - 16.) / 24.).round() as usize
}
pub fn palette_hit(r: [f32; 4], p: [f32; 2]) -> Option<usize> {
    let x = p[0] - r[0] - 8.;
    let y = p[1] - r[1] - 6.;
    if x < 0. || y < 0. {
        return None;
    }
    let cols = palette_columns(r);
    let col = (x / 24.).floor() as usize;
    let index = (y / 24.).floor() as usize * cols + col;
    (col < cols && index < 7).then_some(index)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn overlaps(a: [f32; 4], b: [f32; 4]) -> bool {
        a[0] < b[2] && a[2] > b[0] && a[1] < b[3] && a[3] > b[1]
    }
    #[test]
    fn tiny_chrome_keeps_search_typing_and_footer_targets_separate() {
        for [w, h] in [
            [160., 160.],
            [160., 320.],
            [200., 160.],
            [240., 240.],
            [300., 650.],
            [880., 160.],
            [880., 540.],
        ] {
            for search_open in [false, true] {
                let c = Chrome { w, h, search_open };
                let mut targets = vec![c.add(), c.search(), c.folder(), c.undo()];
                targets.extend(crate::settings_ui::widget_header_hits(w).map(|(_, r)| r));
                for (i, r) in targets.iter().enumerate() {
                    assert!(
                        r[0] >= 8.
                            && r[1] >= 8.
                            && r[2] <= w - 8.
                            && r[3] <= h - 8.
                            && r[2] > r[0]
                            && r[3] > r[1]
                    );
                    for b in &targets[i + 1..] {
                        assert!(!overlaps(*r, *b), "Overlapping controls at {w}x{h}");
                    }
                }
                let v = c.viewport();
                assert!(v[0] <= v[1] && v[1] <= c.undo()[1]);
                if !search_open {
                    assert!(v[1] - c.content_top() >= 60.);
                }
            }
        }
    }
    #[test]
    fn tiny_cards_keep_a_real_text_line_in_every_design() {
        for design in super::super::model::DESIGNS {
            for [w, h] in [[160., 160.], [160., 320.], [320., 160.]] {
                let c = Chrome {
                    w,
                    h,
                    search_open: false,
                };
                let metrics = super::super::typography::Metrics::for_board(design, w, h);
                let height = c.card_height(metrics.height);
                let text = metrics.text_rect([w - c.card_inset(design) * 2., height]);
                assert!(
                    text[2] - text[0] >= 80. && text[3] - text[1] >= 20.,
                    "{design} must remain editable at {w}x{h}"
                );
                assert!(c.content_top() + height <= c.viewport()[1]);
                assert!(!c.large());
            }
        }
    }
    #[test]
    fn narrow_palette_keeps_all_seven_colors_inside_the_card() {
        let card = [12., 52., 148., 118.];
        let r = palette(card, [50., 118.]);
        assert!(r[0] >= card[0] && r[2] <= card[2] && r[1] >= 50. && r[3] <= 118.);
        let cols = palette_columns(r);
        for i in 0..7 {
            assert_eq!(
                palette_hit(
                    r,
                    [
                        r[0] + 16. + (i % cols) as f32 * 24.,
                        r[1] + 14. + (i / cols) as f32 * 24.
                    ]
                ),
                Some(i)
            );
        }
    }
}
