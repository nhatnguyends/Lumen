//! Theme detail port from CalendarApp.tsx, calendar.css and widget-designs.css.
use super::{artwork::Material, color, layout::Layout, typography::Style, Action, State, MONTHS};
use crate::render::Canvas;
use windows_sys::Win32::Graphics::Gdi::HDC;
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
impl State {
    /// Tiny windows spend their height on the calendar, not secondary theme cards.
    unsafe fn paint_tiny(&mut self, dc: HDC, w: f32, h: f32, ink: u32, ambient: u32) {
        let l = Layout::new(&self.config.theme, w, h);
        let c = Canvas::new(dc, self.dpi);
        let accent = match self.config.theme.as_str() {
            "blocks" => color(255, 144, 41),
            "pop" => color(255, 75, 50),
            "prism" => color(245, 100, 180),
            "grove" => color(180, 237, 56),
            "mist" => color(101, 137, 172),
            "agenda" | "soft" => color(235, 82, 79),
            _ => ambient,
        };
        let month = MONTHS[self.cursor.month as usize - 1];
        let title = if w < 240. {
            format!("{} {:02}", &month[..3], self.cursor.year % 100)
        } else {
            format!("{} {}", month, self.cursor.year)
        };
        self.typo
            .draw(dc, self.dpi, &title, l.label, Style::new(10.5, 650, ink), 0);
        for (rect, action) in [
            (l.previous, Action::Previous),
            (l.next, Action::Next),
            (l.today, Action::Today),
        ] {
            if self.hover == Some(action) || self.pressed == Some(action) {
                c.round(
                    rect,
                    6.,
                    ink,
                    if self.pressed == Some(action) {
                        40
                    } else {
                        (24. * self.hover_alpha) as u8
                    },
                    false,
                    false,
                );
            }
            if action == Action::Today {
                self.typo.draw(
                    dc,
                    self.dpi,
                    "Today",
                    rect,
                    Style::new(9., 600, ink).alpha(0.85),
                    1,
                );
            } else {
                let x = (rect[0] + rect[2]) * 0.5;
                let y = (rect[1] + rect[3]) * 0.5;
                let sign = if action == Action::Next { 1. } else { -1. };
                c.line(
                    &[
                        (x - sign * 2., y - 3.),
                        (x + sign * 2., y),
                        (x - sign * 2., y + 3.),
                    ],
                    ink,
                    210,
                    1.3,
                );
            }
        }
        let cw = (l.weekdays[2] - l.weekdays[0]) / 7.;
        for i in 0..7 {
            let day = (i + self.config.week_starts_on as usize) % 7;
            let label = &WEEKDAYS[day][..if w < 224. { 1 } else { 2 }];
            self.typo.draw(
                dc,
                self.dpi,
                label,
                [
                    l.weekdays[0] + i as f32 * cw,
                    l.weekdays[1],
                    l.weekdays[0] + (i + 1) as f32 * cw,
                    l.weekdays[3],
                ],
                Style::new(8.5, 600, ink).alpha(0.6),
                1,
            );
        }
        let days = self.cursor.matrix(self.config.week_starts_on == 1);
        for (i, date) in days.iter().enumerate() {
            let r = l.day(i);
            let action = Action::Day(i);
            let selected = *date == self.selected;
            let today = *date == self.today;
            let opacity = if date.month == self.cursor.month {
                1.
            } else {
                0.32
            };
            if selected {
                c.round(r, 4., accent, 255, false, false);
            } else if self.hover == Some(action) {
                c.round(r, 4., ink, (28. * self.hover_alpha) as u8, false, false);
            }
            if self.pressed == Some(action) {
                c.round(r, 4., ink, 40, false, false);
            }
            if today && !selected {
                self.artwork.border(dc, self.dpi, r, 4., accent, 0.8, 1.);
            }
            let fg = if selected {
                if self.config.theme == "blocks" || self.config.theme == "grove" {
                    color(20, 24, 20)
                } else {
                    0xffffff
                }
            } else {
                ink
            };
            self.typo.draw(
                dc,
                self.dpi,
                &date.day.to_string(),
                r,
                Style::new(
                    ((r[3] - r[1]) * 0.8).clamp(8., 11.),
                    if selected { 650 } else { 500 },
                    fg,
                )
                .alpha(opacity),
                1,
            );
            if selected && self.keyboard_day {
                self.artwork.border(dc, self.dpi, r, 4., ink, 0.8, 1.);
            }
        }
        let header_action = |a| match a {
            Some(Action::Settings) => Some(crate::settings_ui::HeaderAction::Settings),
            Some(Action::Hide) => Some(crate::settings_ui::HeaderAction::Hide),
            _ => None,
        };
        crate::settings_ui::draw_widget_header(
            dc,
            self.hwnd,
            self.dpi,
            w,
            "Calendar",
            ink,
            header_action(self.hover),
            header_action(self.pressed),
        );
    }
    pub(super) unsafe fn paint_calendar(
        &mut self,
        dc: HDC,
        w: f32,
        h: f32,
        ink: u32,
        ambient: u32,
    ) {
        if super::layout::tiny(w, h) {
            self.paint_tiny(dc, w, h, ink, ambient);
            return;
        }
        if super::layout::is_reference(&self.config.theme) {
            self.paint_reference(dc, w, h);
            return;
        }
        let theme = self.config.theme.clone();
        let t = theme.as_str();
        let native_surface = t == "lumen"
            && self.config.palette == "wallpaper-auto"
            && self.config.lumen_surface != "glass";
        let l = Layout::new(t, w, h);
        let c = Canvas::new(dc, self.dpi);
        let accent = match t {
            "blocks" => color(255, 144, 41),
            "pop" => color(255, 75, 50),
            "prism" => color(245, 100, 180),
            "grove" => color(180, 237, 56),
            _ => ambient,
        };
        let selected = self.selected;
        if let Some(r) = l.date {
            if l.strip {
                let material = match t {
                    "blocks" => Material::BlocksDate,
                    "pop" => Material::PopDate,
                    "prism" => Material::PrismDate,
                    "grove" => Material::GroveDate,
                    _ => Material::Almanac,
                };
                self.artwork.draw(dc, self.dpi, r, 10., material, 1.);
                let text_ink = if t == "almanac" || t == "blocks" {
                    0x0b1114
                } else if t == "grove" {
                    0x7cf1c7
                } else {
                    0xffffff
                };
                self.typo.draw(
                    dc,
                    self.dpi,
                    &format!("{} {}", selected.day, MONTHS[selected.month as usize - 1]),
                    [r[0] + 10., r[1], r[2] - 42., r[3]],
                    Style::new(11., 600, text_ink),
                    0,
                );
                if t != "almanac" {
                    self.typo.draw(
                        dc,
                        self.dpi,
                        &format!(
                            "{}%",
                            (selected.day as f32 / selected.days_in_month() as f32 * 100.).round()
                                as i32
                        ),
                        [r[2] - 42., r[1], r[2] - 8., r[3]],
                        Style::new(10., 600, text_ink),
                        2,
                    );
                }
            } else if t == "almanac" {
                let size = if h <= 520. {
                    56.
                } else {
                    (w * 0.28).clamp(70., 135.)
                };
                let group = 14. + size + 12.;
                let y = (r[1] + r[3] - group) * 0.5;
                self.typo.draw(
                    dc,
                    self.dpi,
                    &WEEKDAYS[selected.weekday()].to_uppercase(),
                    [r[0], y, r[2] - 22., y + 16.],
                    Style::new(11., 400, ink).spacing(2.),
                    0,
                );
                self.typo.draw(
                    dc,
                    self.dpi,
                    &format!("{:02}", selected.day),
                    [r[0] - 3., y + 15., r[2] - 23., y + 15. + size],
                    Style::new(size, 350, color(175, 72, 47))
                        .serif()
                        .spacing(-6.),
                    0,
                );
                let vertical_text =
                    format!("{} {}", MONTHS[selected.month as usize - 1], selected.year);
                let vertical_style = Style::new(12., 400, ink).spacing(1.).alpha(0.73);
                let bottom = r[3] - 22.;
                let extent = self
                    .typo
                    .measure(self.dpi, &vertical_text, vertical_style)
                    .min(bottom - r[1]);
                self.typo.vertical(
                    dc,
                    self.dpi,
                    &vertical_text,
                    [r[2] - 19., bottom - extent, r[2] - 1., bottom],
                    vertical_style,
                );
                if l.wide {
                    c.line(&[(r[2] + 9., r[1]), (r[2] + 9., r[3])], ink, 45, 1.);
                } else {
                    c.line(&[(r[0], r[3]), (r[2], r[3])], ink, 45, 1.);
                }
            } else {
                let material = match t {
                    "blocks" => Material::BlocksDate,
                    "pop" => Material::PopDate,
                    "prism" => Material::PrismDate,
                    _ => Material::GroveDate,
                };
                self.artwork
                    .draw(dc, self.dpi, r, l.card_radius, material, 1.);
                if t == "pop" {
                    self.artwork
                        .border(dc, self.dpi, r, l.card_radius, 0xffffff, 24. / 255., 2.);
                }
                if t == "grove" {
                    self.artwork.border(
                        dc,
                        self.dpi,
                        r,
                        l.card_radius,
                        color(193, 242, 130),
                        26. / 255.,
                        1.,
                    );
                }
                let number_ink = match t {
                    "blocks" => color(20, 17, 11),
                    "grove" => color(199, 241, 124),
                    _ => 0xffffff,
                };
                let label_ink = if t == "grove" {
                    color(225, 246, 195)
                } else {
                    number_ink
                };
                let small_ink = if t == "prism" {
                    color(74, 32, 61)
                } else {
                    label_ink
                };
                let size = if h <= 520. {
                    if t == "blocks" {
                        50.
                    } else {
                        48.
                    }
                } else if t == "blocks" {
                    (w * 0.22).clamp(55., 105.)
                } else {
                    (w * 0.23).clamp(60., 100.)
                };
                let month_gap = if t == "blocks" { 9. } else { 6. };
                let group = 14. + size + month_gap + 12.;
                let y = (r[1] + r[3] - group) * 0.5;
                let pad = if l.small || h <= 520. { 12. } else { 18. };
                let left = r[0] + pad;
                let right = r[2] - pad;
                self.typo.draw(
                    dc,
                    self.dpi,
                    WEEKDAYS[selected.weekday()],
                    [left, y, right, y + 14.],
                    if t == "prism" {
                        Style::new(12., 400, label_ink).glow(r)
                    } else {
                        Style::new(if t == "blocks" { 10. } else { 12. }, 400, label_ink)
                    },
                    0,
                );
                let weight = match t {
                    "pop" => 650,
                    "prism" => 250,
                    "grove" => 500,
                    _ => 350,
                };
                let spacing = if t == "blocks" {
                    -6.
                } else if h <= 520. {
                    -2.
                } else {
                    -5.
                };
                let number_style = Style::new(size, weight, number_ink).spacing(spacing);
                self.typo.draw(
                    dc,
                    self.dpi,
                    &format!("{:02}", selected.day),
                    [left - 2., y + 14., right, y + 14. + size],
                    if t == "prism" {
                        number_style.glow(r)
                    } else {
                        number_style
                    },
                    0,
                );
                self.typo.draw(
                    dc,
                    self.dpi,
                    &format!("{} {}", MONTHS[selected.month as usize - 1], selected.year),
                    [
                        left,
                        y + size + 14. + month_gap,
                        right,
                        y + size + 26. + month_gap,
                    ],
                    Style::new(
                        if t == "blocks" { 9. } else { 10. },
                        if t == "prism" { 650 } else { 400 },
                        small_ink,
                    ),
                    0,
                );
            }
        }
        if let Some(r) = l.progress {
            let material = match t {
                "blocks" => Material::BlocksProgress,
                "pop" => Material::PopProgress,
                "prism" => Material::PrismProgress,
                _ => Material::GroveProgress,
            };
            self.artwork.draw(
                dc,
                self.dpi,
                r,
                if t == "blocks" { 22. } else { l.card_radius },
                material,
                1.,
            );
            if t == "grove" {
                self.artwork.border(
                    dc,
                    self.dpi,
                    r,
                    l.card_radius,
                    color(125, 238, 192),
                    38. / 255.,
                    1.,
                );
            }
            let fg = match t {
                "pop" => color(19, 19, 19),
                "prism" => color(244, 255, 217),
                "grove" => color(87, 223, 175),
                _ => 0xffffff,
            };
            let value = selected.day as f32 / selected.days_in_month() as f32;
            let percent = (value * 100.).round() as i32;
            let size = (w * 0.11).clamp(28., 54.);
            let gap = if h <= 520. { 5. } else { 10. };
            let trackheight = if t == "grove" { 8. } else { 6. };
            let group = 14. + gap + size + gap + trackheight + gap + 12.;
            let y = (r[1] + r[3] - group) * 0.5;
            let pad = if l.small || h <= 520. { 14. } else { 18. };
            let left = r[0] + pad;
            let right = r[2] - pad;
            self.typo.draw(
                dc,
                self.dpi,
                "Through the month",
                [left, y, right, y + 14.],
                Style::new(10., 400, fg),
                0,
            );
            let number = percent.to_string();
            let value_w = self
                .typo
                .measure(self.dpi, &number, Style::new(size, 450, fg).spacing(-2.))
                .min(right - left - size * 0.42 - 3.);
            let yy = y + 14. + gap;
            self.typo.draw(
                dc,
                self.dpi,
                &number,
                [left - 1., yy, left + value_w, yy + size],
                Style::new(size, 450, fg).spacing(-2.),
                0,
            );
            self.typo.draw(
                dc,
                self.dpi,
                "%",
                [left + value_w + 3., yy + size * 0.56, right, yy + size],
                Style::new(size * 0.4, 450, fg),
                0,
            );
            let bar = [left, yy + size + gap, right, yy + size + gap + trackheight];
            if t == "grove" {
                let mut x = left;
                let filled = left + (right - left) * value;
                while x < right {
                    c.round(
                        [x, bar[1], (x + 4.).min(right), bar[3]],
                        0.,
                        0xffffff,
                        22,
                        false,
                        false,
                    );
                    if x < filled {
                        c.round(
                            [x, bar[1], (x + 4.).min(filled).min(right), bar[3]],
                            0.,
                            color(67, 226, 168),
                            255,
                            false,
                            false,
                        );
                    }
                    x += 6.;
                }
            } else {
                c.round(bar, 3., fg, if t == "pop" { 32 } else { 38 }, false, false);
                c.round(
                    [left, bar[1], left + (right - left) * value, bar[3]],
                    3.,
                    fg,
                    255,
                    false,
                    false,
                );
            }
            self.typo.draw(
                dc,
                self.dpi,
                &format!("Day {} of {}", selected.day, selected.days_in_month()),
                [left, bar[3] + gap, right, bar[3] + gap + 12.],
                Style::new(10., 400, fg).alpha(0.75),
                0,
            );
        }
        // The month grid belongs directly to the continuous widget material.
        // Date/progress cards remain intentional; there is no second month-panel rim.
        let header_action = |a: Option<Action>| match a {
            Some(Action::Settings) => Some(crate::settings_ui::HeaderAction::Settings),
            Some(Action::Hide) => Some(crate::settings_ui::HeaderAction::Hide),
            _ => None,
        };
        crate::settings_ui::draw_widget_header(
            dc,
            self.hwnd,
            self.dpi,
            w,
            "Calendar",
            ink,
            header_action(self.hover),
            header_action(self.pressed),
        );
        // Month typography differs from content-theme typography by design.
        let month = MONTHS[self.cursor.month as usize - 1];
        let label = l.label;
        let size = if t == "almanac" && !l.strip {
            19.
        } else {
            13.5
        };
        let mut style = Style::new(size, 600, ink);
        if t == "almanac" && !l.strip {
            style = style.serif();
        }
        let measured = self.typo.measure(self.dpi, month, style);
        let show_year = measured + 32. < label[2] - label[0];
        let month_end = if show_year {
            label[0] + measured + 2.
        } else {
            label[2]
        };
        self.typo.draw(
            dc,
            self.dpi,
            month,
            [label[0], label[1], month_end, label[3]],
            style,
            0,
        );
        if show_year {
            self.typo.draw(
                dc,
                self.dpi,
                &self.cursor.year.to_string(),
                [month_end + 3., label[1] + 2., label[2], label[3]],
                Style::new(10., 500, ink).alpha(0.53),
                0,
            );
        }
        let line = l.weekdays[1] - 5.;
        c.line(&[(l.weekdays[0], line), (l.weekdays[2], line)], ink, 30, 1.);
        for i in 0..7 {
            let cell = (l.weekdays[2] - l.weekdays[0]) / 7.;
            let x = l.weekdays[0] + i as f32 * cell;
            let day = (i + self.config.week_starts_on as usize) % 7;
            self.typo.draw(
                dc,
                self.dpi,
                &WEEKDAYS[day][..2].to_uppercase(),
                [x, l.weekdays[1], x + cell, l.weekdays[3]],
                Style::new(8.5, 650, ink).spacing(0.765).alpha(0.53),
                1,
            );
        }
        let days = self.cursor.matrix(self.config.week_starts_on == 1);
        for button in self.buttons() {
            let hover = self.hover == Some(button.action);
            let pressed = self.pressed == Some(button.action);
            match button.action {
                Action::Settings
                | Action::Hide
                | Action::WeekDay(_)
                | Action::Weekly(_)
                | Action::Month(_) => {}
                Action::Previous | Action::Next => {
                    if hover {
                        c.round(
                            button.rect,
                            9.,
                            ink,
                            (self.hover_alpha * 18.) as u8,
                            false,
                            false,
                        );
                    }
                    let x = (button.rect[0] + button.rect[2]) * 0.5;
                    let y = (button.rect[1] + button.rect[3]) * 0.5;
                    let sign = if button.action == Action::Previous {
                        -1.
                    } else {
                        1.
                    };
                    c.line(
                        &[
                            (x - sign * 2., y - 5.),
                            (x + sign * 2., y),
                            (x - sign * 2., y + 5.),
                        ],
                        ink,
                        if hover { 255 } else { 186 },
                        1.5,
                    );
                }
                Action::Today => {
                    c.round(
                        button.rect,
                        12.,
                        ink,
                        if hover { 33 } else { 18 },
                        false,
                        false,
                    );
                    c.round(button.rect, 12., ink, 30, true, false);
                    self.typo.draw(
                        dc,
                        self.dpi,
                        "Today",
                        button.rect,
                        Style::new(9.5, 600, ink).alpha(if hover { 1. } else { 0.73 }),
                        1,
                    );
                }
                Action::Day(i) => {
                    let date = days[i];
                    let selected = date == self.selected;
                    let today = date == self.today;
                    let outside = date.month != self.cursor.month;
                    let opacity = if outside { 0.45 } else { 1. };
                    let circular = matches!(t, "pop" | "prism" | "grove");
                    let r = if circular {
                        l.circular_day(i)
                    } else {
                        button.rect
                    };
                    let radius = if circular {
                        (r[2] - r[0]) / 2.
                    } else if t == "blocks" {
                        12.
                    } else {
                        11.
                    };
                    if hover && !selected && !(native_surface && today) {
                        c.round(
                            r,
                            radius,
                            ink,
                            (18. * self.hover_alpha * opacity) as u8,
                            false,
                            false,
                        );
                    }
                    if pressed && !selected {
                        c.round(r, radius, accent, (30. * opacity) as u8, false, false);
                    }
                    if native_surface {
                        if today {
                            let circle = l.circular_day(i);
                            c.round(
                                circle,
                                (circle[2] - circle[0]) / 2.,
                                color(255, 73, 63),
                                (255. * opacity) as u8,
                                false,
                                false,
                            );
                        } else if selected {
                            c.round(r, radius, ink, (33. * opacity) as u8, false, false);
                            self.artwork
                                .border(dc, self.dpi, r, radius, ink, 0.53 * opacity, 1.);
                        }
                    } else if selected {
                        if t == "lumen" || t == "almanac" {
                            c.soft_shadow(r, radius, opacity * 0.3, 7., 3.);
                            self.artwork.draw(
                                dc,
                                self.dpi,
                                r,
                                radius,
                                Material::Selection(accent),
                                opacity,
                            );
                            c.line(
                                &[(r[0] + radius, r[1] + 0.7), (r[2] - radius, r[1] + 0.7)],
                                0xffffff,
                                (76. * opacity) as u8,
                                1.,
                            );
                        } else {
                            c.round(r, radius, accent, (255. * opacity) as u8, false, false);
                        }
                    } else if today {
                        self.artwork
                            .border(dc, self.dpi, r, radius, accent, 0.75 * opacity, 1.);
                        if t == "lumen" || t == "almanac" {
                            let x = (r[0] + r[2]) / 2.;
                            c.round(
                                [x - 1.5, r[3] - 7., x + 1.5, r[3] - 4.],
                                1.5,
                                accent,
                                (255. * opacity) as u8,
                                false,
                                false,
                            );
                        }
                    }
                    let fg = if native_surface {
                        if today {
                            0xffffff
                        } else {
                            ink
                        }
                    } else if selected {
                        match t {
                            "blocks" => color(20, 17, 11),
                            "prism" => color(38, 12, 35),
                            "grove" => color(21, 33, 9),
                            _ => 0xffffff,
                        }
                    } else {
                        ink
                    };
                    let mut style = Style::new(12.5, if selected { 650 } else { 500 }, fg).alpha(
                        opacity
                            * if selected || today {
                                1.
                            } else if outside {
                                0.53
                            } else {
                                0.73
                            },
                    );
                    if t == "almanac" {
                        style = style.serif();
                    }
                    self.typo
                        .draw(dc, self.dpi, &date.day.to_string(), r, style, 1);
                    if circular && selected && self.keyboard_day {
                        let pad = if r[3] - r[1] < 26. { 1. } else { 3. };
                        self.artwork.border(
                            dc,
                            self.dpi,
                            [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad],
                            radius + pad,
                            ink,
                            1.,
                            2.,
                        );
                    }
                }
            }
        }
    }
}
