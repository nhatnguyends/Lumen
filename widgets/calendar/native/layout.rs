//! CalendarApp.tsx + calendar.css/widget-designs.css geometry in panel DIPs.
//! Keep drawing and hit testing on this single layout, including new tiny sizes.
pub type Rect = [f32; 4];
/// Must match WindowShadow's shared 34-DIP silhouette. Theme radii apply only to content.
pub const CHROME_RADIUS: f32 = 34.;
/// Unmasked material backing. Only the compositor clips the outer silhouette.
pub fn background_rect(w: f32, h: f32) -> Rect {
    [0., 0., w, h]
}
/// Below this breakpoint, secondary cards and reference-only controls yield to dates.
pub fn tiny(w: f32, h: f32) -> bool {
    w < 240. || h < 280.
}
#[derive(Clone, Debug)]
pub struct Layout {
    pub month: Rect,
    pub date: Option<Rect>,
    pub progress: Option<Rect>,
    pub strip: bool,
    pub wide: bool,
    pub outer_radius: f32,
    pub card_radius: f32,
    pub previous: Rect,
    pub next: Rect,
    pub today: Rect,
    pub label: Rect,
    pub weekdays: Rect,
    pub grid: Rect,
    pub small: bool,
}
impl Layout {
    pub fn new(theme: &str, w: f32, h: f32) -> Self {
        if tiny(w, h) {
            return Self::tiny(theme, w, h);
        }
        if is_reference(theme) {
            return ReferenceLayout::new(theme, w, h).calendar;
        }
        let small = w <= 320.;
        let compact = !crate::settings_ui::widget_title_visible(w, h);
        let strip = compact || h < 400. || theme == "almanac" && w < 560. && h < 440.;
        let wide = w >= 560. && !strip;
        let outer_radius = CHROME_RADIUS;
        let card_radius = if small {
            18.
        } else if theme == "blocks" {
            22.
        } else {
            24.
        };
        let (mut date, mut progress) = (None, None);
        let mut month = [20., 54., w - 20., h - 20.];
        if theme != "lumen" {
            if strip {
                date = Some([16., 49., w - 16., 77.]);
                month = [16., 82., w - 16., h - 16.];
            } else if theme == "almanac" {
                if wide {
                    let col = (w - 48. - 20.) * 0.7 / 1.7;
                    date = Some([24., 54., 24. + col, h - 20.]);
                    month = [24. + col + 20., 54., w - 24., h - 20.];
                } else {
                    let hero = if h <= 520. {
                        56.
                    } else {
                        (w * 0.28).clamp(70., 135.)
                    };
                    let bottom = 54. + (14. + hero + 22.).max(134.);
                    date = Some([24., 54., w - 24., bottom]);
                    month = [24., bottom + 14., w - 24., h - 20.];
                }
            } else if wide && theme != "blocks" {
                let left = (w - 40. - 14.) / 2.8;
                let middle = 54. + (h - 54. - 20. - 14.) / 2.;
                date = Some([20., 54., 20. + left, middle]);
                progress = Some([20., middle + 14., 20. + left, h - 20.]);
                month = [20. + left + 14., 54., w - 20., h - 20.];
            } else {
                let pad = if theme == "blocks" {
                    16.
                } else if small {
                    16.
                } else {
                    20.
                };
                let gap = if theme == "blocks" {
                    12.
                } else if small {
                    8.
                } else {
                    10.
                };
                let available = w - pad * 2. - gap;
                let left = if theme == "blocks" {
                    available / 2.
                } else {
                    available * 1.15 / 2.15
                };
                let hero = if h <= 520. {
                    if theme == "blocks" {
                        50.
                    } else {
                        48.
                    }
                } else if theme == "blocks" {
                    (w * 0.22).clamp(55., 105.)
                } else {
                    (w * 0.23).clamp(60., 100.)
                };
                let card_h = if h <= 520. {
                    if theme == "blocks" {
                        104.
                    } else {
                        100.
                    }
                } else {
                    hero + 68.
                };
                date = Some([pad, 50., pad + left, 50. + card_h]);
                progress = Some([pad + left + gap, 50., w - pad, 50. + card_h]);
                month = [pad, 50. + card_h + gap, w - pad, h - 16.];
            }
        } else if small || strip {
            month = [16., 52., w - 16., h - 16.];
        }
        if compact {
            if theme == "lumen" {
                month[1] = 16.;
            } else {
                date = Some([16., 16., w - 100., 42.]);
                progress = None;
                month = [16., 48., w - 16., h - 16.];
            }
        }
        let card = theme != "lumen" && theme != "almanac" && !strip;
        let inner = if card { 8. } else { 0. };
        let top = month[1] + if card { 12. } else { 2. };
        let x = month[0] + inner;
        let right = month[2] - inner;
        let two_rows = compact && theme == "lumen" && w < 360.;
        let nav_top = if two_rows { top + 30. } else { top };
        let nav_right = if compact && theme == "lumen" && !two_rows {
            right.min(w - 100.)
        } else {
            right
        };
        let previous = [x, nav_top, x + 28., nav_top + 30.];
        let today = [nav_right - 54., nav_top + 3., nav_right, nav_top + 27.];
        let next = if two_rows {
            [x + 32., nav_top, x + 60., nav_top + 30.]
        } else {
            [today[0] - 32., nav_top, today[0] - 4., nav_top + 30.]
        };
        let label = if two_rows {
            [x, top, w - 100., top + 28.]
        } else {
            [previous[2] + 4., top, next[0] - 4., top + 30.]
        };
        let line = nav_top + if strip { 32. } else { 39. };
        let weekdays = [x, line + 5., right, line + 20.];
        let grid = [
            x,
            line + if strip { 20. } else { 26. },
            right,
            month[3] - if card { 8. } else { 0. },
        ];
        Self {
            month,
            date,
            progress,
            strip,
            wide,
            outer_radius,
            card_radius,
            previous,
            next,
            today,
            label,
            weekdays,
            grid,
            small,
        }
    }
    fn tiny(_theme: &str, w: f32, h: f32) -> Self {
        Self {
            month: [14., 46., w - 14., h - 14.],
            date: None,
            progress: None,
            strip: true,
            wide: false,
            outer_radius: CHROME_RADIUS,
            card_radius: 12.,
            previous: [16., 46., 40., 70.],
            next: [44., 46., 68., 70.],
            today: [w - 60., 46., w - 16., 70.],
            label: [16., 16., w - 100., 40.],
            weekdays: [14., 74., w - 14., 90.],
            grid: [14., 92., w - 14., h - 14.],
            small: true,
        }
    }
    pub fn day(&self, i: usize) -> Rect {
        let r = self.grid;
        let w = (r[2] - r[0]) / 7.;
        let h = (r[3] - r[1]) / 6.;
        let x = r[0] + i as f32 % 7. * w;
        let y = r[1] + (i / 7) as f32 * h;
        [x + 1., y + 1., x + w - 1., y + h - 1.]
    }
    pub fn circular_day(&self, i: usize) -> Rect {
        let r = self.day(i);
        let size = (r[2] - r[0]).min(r[3] - r[1]).min(30.);
        let x = (r[0] + r[2]) * 0.5;
        let y = (r[1] + r[3]) * 0.5;
        [x - size / 2., y - size / 2., x + size / 2., y + size / 2.]
    }
}
pub fn is_reference(theme: &str) -> bool {
    matches!(theme, "mist" | "agenda" | "soft")
}

/// Reference-theme controls use the same geometry for pixels and pointer input.
pub struct ReferenceLayout {
    pub calendar: Layout,
    pub hero: Rect,
    pub sidebar: Option<Rect>,
    pub weekly: Rect,
    pub monthly: Rect,
    pub months: [Rect; 12],
    pub progress: Rect,
}
impl ReferenceLayout {
    /// Reserve a full font line box, including descent, before the weekday label.
    pub fn agenda_date_text(&self) -> (Rect, f32, Rect) {
        let r = self.hero;
        let size = ((r[2] - r[0]) * 0.72)
            .min(92.)
            .min((r[3] - r[1] - 36.) / 1.6)
            .max(16.);
        let bottom = r[1] + size * 1.6 + 8.;
        (
            [r[0], r[1], r[2], bottom],
            size,
            [r[0], bottom + 4., r[2], bottom + 28.],
        )
    }
    pub fn new(theme: &str, w: f32, h: f32) -> Self {
        if tiny(w, h) {
            return Self {
                calendar: Layout::tiny(theme, w, h),
                hero: [0.; 4],
                sidebar: None,
                weekly: [0.; 4],
                monthly: [0.; 4],
                months: [[0.; 4]; 12],
                progress: [0.; 4],
            };
        }
        let top = crate::settings_ui::widget_content_top(w, h);
        let side = theme == "agenda" && w >= 400. && h >= 360.;
        let sx = (w * 0.37).clamp(140., 240.);
        let x = if side { sx + 16. } else { 20. };
        let right = w - 20.;
        let nav_y = match theme {
            "mist" => {
                top + if h < 360. {
                    52.
                } else if h >= 400. {
                    112.
                } else {
                    80.
                }
            }
            "agenda" if side => top + 2.,
            "agenda" => top + 44.,
            _ => top + (h * 0.23).clamp(62., 106.),
        };
        let previous = [x, nav_y, x + 24., nav_y + 24.];
        let today = [right - 45., nav_y, right, nav_y + 24.];
        let next = [right - 73., nav_y, right - 49., nav_y + 24.];
        let label = [x + 27., nav_y, right - 76., nav_y + 24.];
        let week_y = nav_y + if theme == "agenda" { 54. } else { 30. };
        let footer = 20.;
        let grid = [x, week_y + 22., right, h - footer];
        let calendar = Layout {
            month: [x, nav_y, right, h - footer],
            date: None,
            progress: None,
            strip: false,
            wide: side,
            outer_radius: CHROME_RADIUS,
            card_radius: 24.,
            previous,
            next,
            today,
            label,
            weekdays: [x, week_y, right, week_y + 20.],
            grid,
            small: w <= 320.,
        };
        let tab_right = (w - 100.).min(320.);
        let weekly = [20., top, (20. + tab_right) * 0.5, top + 28.];
        let monthly = [weekly[2], top, tab_right, top + 28.];
        let hero = match theme {
            "mist" => [20., top + 33., right, nav_y - 2.],
            "agenda" if side => [22., top + 12., sx - 12., top + 196.],
            "agenda" => [20., top, w - 100., top + 42.],
            _ => [28., top + 6., w - 28., nav_y - 10.],
        };
        let months = std::array::from_fn(|i| {
            let cw = (right - x) / 12.;
            [
                x + i as f32 * cw,
                nav_y + 28.,
                x + (i + 1) as f32 * cw,
                nav_y + 49.,
            ]
        });
        Self {
            calendar,
            hero,
            sidebar: side.then_some([0., 0., sx, h]),
            weekly,
            monthly,
            months,
            progress: [28., nav_y - 8., w - 28., nav_y - 4.],
        }
    }
    pub fn week_day(&self, i: usize) -> Rect {
        let r = self.calendar.grid;
        let cw = (r[2] - r[0]) / 7.;
        let bend = ((i as f32 - 3.) / 3.).powi(2) * 12.;
        let cy = (r[1] + r[3]) * 0.5 + bend;
        [
            r[0] + i as f32 * cw,
            cy - 22.,
            r[0] + (i + 1) as f32 * cw,
            cy + 22.,
        ]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const THEMES: [&str; 6] = ["lumen", "almanac", "blocks", "pop", "prism", "grove"];
    #[test]
    fn themes_share_the_chrome_and_shadow_contour_at_every_size_and_dpi() {
        for theme in [
            "lumen", "almanac", "blocks", "pop", "prism", "grove", "mist", "agenda", "soft",
        ] {
            for [w, h] in [
                [168., 190.],
                [240., 280.],
                [420., 460.],
                [800., 400.],
                [800., 680.],
            ] {
                let l = Layout::new(theme, w, h);
                assert_eq!(
                    l.outer_radius, 34.,
                    "{theme} must match shared WindowShadow"
                );
                for dpi in [1., 1.25, 1.5, 2.] {
                    for [x, y] in [[8., 8.], [16., 16.], [24., 8.], [34., 4.], [10., 30.]] {
                        let actual = crate::compositor::distance(
                            x * dpi,
                            y * dpi,
                            w * dpi,
                            h * dpi,
                            l.outer_radius * dpi,
                            4. * dpi,
                        );
                        let expected = crate::compositor::distance(
                            x * dpi,
                            y * dpi,
                            w * dpi,
                            h * dpi,
                            34. * dpi,
                            4. * dpi,
                        );
                        assert!((actual - expected).abs() < 0.001);
                    }
                }
            }
        }
    }
    #[test]
    fn agenda_number_has_full_line_height_and_clearance_above_weekday() {
        for [w, h] in [[400., 360.], [420., 460.], [800., 400.], [800., 680.]] {
            let r = ReferenceLayout::new("agenda", w, h);
            let (number, size, weekday) = r.agenda_date_text();
            assert!(number[3] - number[1] >= size * 1.6 + 7.9);
            assert!(weekday[1] >= number[3] + 4. && weekday[3] <= r.hero[3] + 0.01);
            assert!(number[0] >= r.hero[0] && number[2] <= r.hero[2]);
        }
    }
    #[test]
    fn tiny_calendar_retains_42_days_and_separates_all_controls_from_header() {
        let disjoint =
            |a: Rect, b: Rect| a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1];
        for theme in [
            "lumen", "almanac", "blocks", "pop", "prism", "grove", "mist", "agenda", "soft",
        ] {
            for [w, h] in [
                [168., 190.],
                [180., 200.],
                [220., 240.],
                [239., 280.],
                [240., 279.],
                [168., 680.],
                [800., 190.],
            ] {
                let l = Layout::new(theme, w, h);
                assert!(tiny(w, h) && l.date.is_none() && l.progress.is_none());
                let mut controls = vec![l.label, l.previous, l.next, l.today, l.weekdays];
                controls.extend(crate::settings_ui::widget_header_hits(w).map(|(_, rect)| rect));
                controls.extend((0..42).map(|i| l.day(i)));
                for (i, &a) in controls.iter().enumerate() {
                    assert!(
                        a[0] >= 14.
                            && a[1] >= 12.
                            && a[2] <= w - 14.
                            && a[3] <= h - 12.
                            && a[2] > a[0]
                            && a[3] > a[1],
                        "{theme} {w}x{h} {a:?}"
                    );
                    for &b in &controls[i + 1..] {
                        assert!(disjoint(a, b), "{theme}: {a:?} overlaps {b:?}");
                    }
                }
                for i in 0..42 {
                    let day = l.day(i);
                    assert!(day[2] - day[0] >= 18. && day[3] - day[1] >= 12.);
                }
                if is_reference(theme) {
                    let r = ReferenceLayout::new(theme, w, h);
                    assert!(r.sidebar.is_none());
                }
            }
            let restored = Layout::new(theme, 420., 460.);
            assert!(!tiny(240., 280.));
            assert!(!tiny(420., 460.));
            if !is_reference(theme) && theme != "lumen" {
                assert!(restored.date.is_some());
            }
        }
    }
    #[test]
    fn all_designs_keep_content_insets_separate_from_full_bleed_background() {
        for [w, h] in [
            [240., 280.],
            [320., 400.],
            [420., 460.],
            [560., 400.],
            [800., 280.],
            [800., 680.],
        ] {
            assert_eq!(background_rect(w, h), [0., 0., w, h]);
            for theme in [
                "lumen", "almanac", "blocks", "pop", "prism", "grove", "mist", "agenda", "soft",
            ] {
                let l = Layout::new(theme, w, h);
                let mut content = vec![l.previous, l.next, l.today, l.label, l.weekdays, l.grid];
                content.extend(l.date);
                content.extend(l.progress);
                if is_reference(theme) {
                    let r = ReferenceLayout::new(theme, w, h);
                    content.push(r.hero);
                    if theme == "mist" {
                        content.extend([r.weekly, r.monthly]);
                    }
                    if theme == "agenda" {
                        content.extend(r.months);
                    }
                    if let Some(side) = r.sidebar {
                        // A split background reaches the frame, while its labels/actions don't.
                        assert_eq!([side[0], side[1], side[3]], [0., 0., h]);
                        assert!(r.hero[0] - side[0] >= 16. && side[2] - r.hero[2] >= 12.);
                    }
                }
                for rect in content {
                    assert!(
                        rect[0] >= 16.
                            && rect[1] >= 14.
                            && rect[2] <= w - 16.
                            && rect[3] <= h - 16.,
                        "{theme} {w}x{h}: content crowds outer chrome {rect:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn reference_controls_and_dates_fit_compact_wide_and_large_windows() {
        let disjoint =
            |a: Rect, b: Rect| a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1];
        for theme in ["mist", "agenda", "soft"] {
            for [w, h] in [
                [240., 280.],
                [240., 520.],
                [320., 400.],
                [420., 460.],
                [560., 400.],
                [800., 280.],
                [800., 680.],
            ] {
                let r = ReferenceLayout::new(theme, w, h);
                let l = &r.calendar;
                let mut controls = vec![l.previous, l.next, l.today, l.label];
                if theme == "mist" {
                    controls.extend([r.weekly, r.monthly]);
                }
                if theme == "agenda" {
                    controls.extend(r.months);
                }
                for (i, &a) in controls.iter().enumerate() {
                    assert!(
                        a[0] >= 10.
                            && a[2] <= w - 10.
                            && a[1] >= 10.
                            && a[3] <= h - 10.
                            && a[2] > a[0]
                            && a[3] > a[1],
                        "{theme} {w}x{h}: {a:?}"
                    );
                    for &b in &controls[i + 1..] {
                        assert!(disjoint(a, b), "{theme} {w}x{h}: {a:?} overlaps {b:?}");
                    }
                    for (_, header) in crate::settings_ui::widget_header_hits(w) {
                        assert!(disjoint(a, header), "{theme}: header covers {a:?}");
                    }
                }
                for i in 0..42 {
                    let day = l.day(i);
                    assert!(
                        day[2] - day[0] >= 24. && day[3] - day[1] >= 12.,
                        "{theme} {w}x{h}: {day:?}"
                    );
                    assert!(
                        day[0] >= 10. && day[2] <= w - 10. && day[1] >= 48. && day[3] <= h - 10.
                    );
                    for &control in &controls {
                        assert!(disjoint(day, control), "{theme}: date overlaps control");
                    }
                }
                if theme == "mist" {
                    for i in 0..7 {
                        let day = r.week_day(i);
                        assert!(day[1] > l.weekdays[3] && day[3] <= h - 16.);
                        for &control in &controls {
                            assert!(disjoint(day, control));
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn old_theme_breakpoints_and_card_positions_are_preserved() {
        let narrow = Layout::new("almanac", 420., 680.);
        assert!(narrow.date.unwrap()[3] < narrow.month[1]);
        let wide = Layout::new("almanac", 800., 680.);
        assert!(wide.date.unwrap()[2] < wide.month[0]);
        assert!(wide.progress.is_none());
        assert_eq!(wide.outer_radius, CHROME_RADIUS);
        for theme in ["pop", "prism", "grove"] {
            let wide = Layout::new(theme, 800., 680.);
            let d = wide.date.unwrap();
            let p = wide.progress.unwrap();
            assert_eq!(d[0], p[0]);
            assert!(d[3] < p[1]);
            assert!(d[2] < wide.month[0]);
        }
        let blocks = Layout::new("blocks", 800., 680.);
        assert_eq!(blocks.date.unwrap()[1], blocks.progress.unwrap()[1]);
        assert!(blocks.date.unwrap()[3] < blocks.month[1]);
        let lumen = Layout::new("lumen", 800., 680.);
        assert!(lumen.date.is_none() && lumen.progress.is_none());
    }
    #[test]
    fn all_themes_keep_dates_and_navigation_disjoint_at_supported_sizes() {
        for theme in THEMES {
            for [w, h] in [
                [240., 280.],
                [240., 520.],
                [320., 400.],
                [420., 460.],
                [560., 400.],
                [800., 280.],
                [800., 680.],
            ] {
                let l = Layout::new(theme, w, h);
                let disjoint =
                    |a: Rect, b: Rect| a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1];
                let navigation = [l.previous, l.label, l.next, l.today];
                for (i, &a) in navigation.iter().enumerate() {
                    for &b in &navigation[i + 1..] {
                        assert!(disjoint(a, b), "{theme} {w}x{h}: navigation overlap");
                    }
                    for (_, header) in crate::settings_ui::widget_header_hits(w) {
                        assert!(
                            disjoint(a, header),
                            "{theme} {w}x{h}: header covers navigation"
                        );
                    }
                }
                for i in 0..42 {
                    let r = l.day(i);
                    assert!(
                        r[0] >= 10. && r[2] <= w - 10. && r[1] >= 48. && r[3] <= h - 10.,
                        "{theme} {w}x{h} {r:?}"
                    );
                    assert!(
                        r[2] - r[0] >= 22. && r[3] - r[1] >= 19.,
                        "{theme} {w}x{h} {r:?}"
                    );
                }
            }
        }
    }
}
