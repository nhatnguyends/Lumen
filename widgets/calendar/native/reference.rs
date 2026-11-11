//! Three reference designs. Geometry is also consumed by State::buttons.
use super::{
    artwork::Material,
    color,
    layout::{Rect, ReferenceLayout},
    typography::Style,
    Action, State, MONTHS,
};
use crate::render::Canvas;
use windows_sys::Win32::Graphics::Gdi::HDC;
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

impl State {
    /// Agenda's split is a full-height color region of the same outer surface.
    /// No inner rounding, rim, shadow or separate white face is drawn here.
    pub(super) unsafe fn paint_agenda_sidebar(&mut self, dc: HDC, w: f32, h: f32) {
        if self.config.theme != "agenda" {
            return;
        }
        if let Some(side) = ReferenceLayout::new("agenda", w, h).sidebar {
            self.artwork
                .draw(dc, self.dpi, side, 0., Material::AgendaRed, 1.);
        }
    }
    unsafe fn reference_text(
        &mut self,
        dc: HDC,
        text: &str,
        r: Rect,
        size: f32,
        weight: u16,
        ink: u32,
        alpha: f32,
        align: i32,
    ) {
        self.typo.draw(
            dc,
            self.dpi,
            text,
            r,
            Style::new(size, weight, ink).alpha(alpha),
            align,
        );
    }
    pub(super) unsafe fn paint_reference(&mut self, dc: HDC, w: f32, h: f32) {
        let theme = self.config.theme.clone();
        let mist = theme == "mist";
        let agenda = theme == "agenda";
        let soft = theme == "soft";
        let r = ReferenceLayout::new(&theme, w, h);
        let l = &r.calendar;
        let c = Canvas::new(dc, self.dpi);
        let ink = if mist && !crate::surface::managed(&self.config.palette) {
            0xffffff
        } else {
            self.ink.unwrap_or(color(34, 37, 40))
        };
        let red = color(235, 82, 79);
        let accent = if mist { color(101, 137, 172) } else { red };
        let selected = self.selected;

        // Every visible control has a corresponding hit target.
        for button in self.buttons() {
            if matches!(
                button.action,
                Action::Settings | Action::Hide | Action::Day(_) | Action::WeekDay(_)
            ) {
                continue;
            }
            let pressed = self.pressed == Some(button.action);
            let hover = self.hover == Some(button.action);
            if pressed || hover {
                c.round(
                    button.rect,
                    8.,
                    ink,
                    if pressed {
                        38
                    } else {
                        (22. * self.hover_alpha) as u8
                    },
                    false,
                    false,
                );
            }
        }

        if mist {
            c.round(
                [
                    r.weekly[0] - 3.,
                    r.weekly[1] - 3.,
                    r.monthly[2] + 3.,
                    r.monthly[3] + 3.,
                ],
                11.,
                color(83, 107, 132),
                48,
                false,
                false,
            );
            let active = if self.config.mist_weekly {
                r.weekly
            } else {
                r.monthly
            };
            c.soft_shadow(active, 7., 0.1, 5., 2.);
            c.round(active, 7., 0xffffff, 238, false, false);
            for (text, rect, active) in [
                ("Weekly", r.weekly, self.config.mist_weekly),
                ("Monthly", r.monthly, !self.config.mist_weekly),
            ] {
                self.reference_text(
                    dc,
                    text,
                    rect,
                    if w < 300. { 10. } else { 12. },
                    600,
                    if active { color(34, 42, 51) } else { ink },
                    if active { 1. } else { 0.65 },
                    1,
                );
            }
            let size = ((r.hero[3] - r.hero[1]) * 0.95).min(w * 0.145).max(16.);
            self.reference_text(
                dc,
                &format!("{}  {}", MONTHS[selected.month as usize - 1], selected.day),
                r.hero,
                size,
                350,
                ink,
                1.,
                0,
            );
        } else if agenda {
            if r.sidebar.is_some() {
                let (number, size, weekday) = r.agenda_date_text();
                self.reference_text(
                    dc,
                    &selected.day.to_string(),
                    number,
                    size,
                    700,
                    0xffffff,
                    1.,
                    1,
                );
                self.reference_text(
                    dc,
                    &DAYS[selected.weekday()].to_uppercase(),
                    weekday,
                    12.,
                    450,
                    0xffffff,
                    0.93,
                    1,
                );
            } else {
                self.reference_text(
                    dc,
                    &format!(
                        "{:02}  {}",
                        selected.day,
                        &DAYS[selected.weekday()][..3].to_uppercase()
                    ),
                    r.hero,
                    24.,
                    650,
                    red,
                    1.,
                    0,
                );
            }
            for (i, rect) in r.months.iter().enumerate() {
                let active = self.cursor.month as usize == i + 1;
                if active {
                    c.round(*rect, 4., red, 22, false, false);
                }
                self.reference_text(
                    dc,
                    &MONTHS[i][..3],
                    *rect,
                    if w < 360. { 7. } else { 7.5 },
                    if active { 700 } else { 450 },
                    ink,
                    if active { 1. } else { 0.52 },
                    1,
                );
            }
        } else {
            let hero = r.hero;
            let left = [hero[0], hero[1], hero[2] - 62., hero[3]];
            let split = hero[1] + (hero[3] - hero[1]) * 0.52;
            self.reference_text(
                dc,
                &selected.day.to_string(),
                [left[0], left[1], left[2], split],
                ((split - left[1]) * 0.94).clamp(23., 35.),
                650,
                ink,
                1.,
                0,
            );
            self.reference_text(
                dc,
                MONTHS[selected.month as usize - 1],
                [left[0], split, left[2], left[3]],
                ((left[3] - split) * 0.7).clamp(13., 21.),
                400,
                ink,
                0.45,
                0,
            );
            let icon_size = if hero[1] < 50. { 28. } else { 44. };
            let icon_y = (hero[1] + 5.).max(44.);
            let icon = [
                hero[2] - icon_size - 3.,
                icon_y,
                hero[2] - 3.,
                icon_y + icon_size,
            ];
            c.soft_shadow(icon, 7., 0.15, 4., 2.);
            c.round(icon, 7., color(246, 246, 243), 255, false, false);
            c.round(
                [icon[0] + 2., icon[1] + 2., icon[2] - 2., icon[1] + 14.],
                3.,
                red,
                255,
                false,
                false,
            );
            for row in 0..3 {
                for col in 0..7 {
                    let scale = icon_size / 44.;
                    let x = icon[0] + (7. + col as f32 * 5.) * scale;
                    let y = icon[1] + (22. + row as f32 * 6.) * scale;
                    c.round([x, y, x + 1.8, y + 1.8], 0.9, ink, 70, false, false);
                }
            }
            let progress = selected.day as f32 / selected.days_in_month() as f32;
            let cw = (r.progress[2] - r.progress[0] - 9.) / 4.;
            for i in 0..4 {
                let x = r.progress[0] + i as f32 * (cw + 3.);
                c.round(
                    [x, r.progress[1], x + cw, r.progress[3]],
                    2.,
                    ink,
                    12,
                    false,
                    false,
                );
                let amount = (progress * 4. - i as f32).clamp(0., 1.);
                if amount > 0. {
                    c.round(
                        [x, r.progress[1], x + cw * amount, r.progress[3]],
                        2.,
                        red,
                        255,
                        false,
                        false,
                    );
                }
            }
        }

        // Navigation and current month stay usable in either date view.
        for (rect, rightward) in [(l.previous, false), (l.next, true)] {
            let x = (rect[0] + rect[2]) * 0.5;
            let y = (rect[1] + rect[3]) * 0.5;
            let sign = if rightward { 1. } else { -1. };
            c.line(
                &[
                    (x - sign * 2., y - 3.5),
                    (x + sign * 2., y),
                    (x - sign * 2., y + 3.5),
                ],
                ink,
                165,
                1.3,
            );
        }
        let nav_label = if agenda {
            self.cursor.year.to_string()
        } else {
            format!(
                "{} {}",
                &MONTHS[self.cursor.month as usize - 1][..3],
                self.cursor.year
            )
        };
        self.reference_text(dc, &nav_label, l.label, 10., 600, ink, 0.78, 0);
        c.round(l.today, 8., ink, 14, false, false);
        self.reference_text(dc, "Today", l.today, 9., 600, ink, 0.8, 1);

        let weekly = mist && self.config.mist_weekly;
        if weekly {
            let cy = (l.grid[1] + l.grid[3]) * 0.5;
            self.artwork.draw(
                dc,
                self.dpi,
                [l.grid[0], cy - 30., l.grid[2], cy + 48.],
                0.,
                Material::Ribbon,
                1.,
            );
        }
        let dates: Vec<_> = if weekly {
            selected.week(self.config.week_starts_on == 1).to_vec()
        } else {
            self.cursor.matrix(self.config.week_starts_on == 1).to_vec()
        };
        for i in 0..7 {
            let day = (i + self.config.week_starts_on as usize) % 7;
            let cw = (l.weekdays[2] - l.weekdays[0]) / 7.;
            let mut rect = [
                l.weekdays[0] + i as f32 * cw,
                l.weekdays[1],
                l.weekdays[0] + (i + 1) as f32 * cw,
                l.weekdays[3],
            ];
            if weekly {
                let cell = r.week_day(i);
                rect = [cell[0], cell[1] - 24., cell[2], cell[1] - 4.];
            }
            let label = if soft {
                &DAYS[day][..1]
            } else if agenda {
                &DAYS[day][..3]
            } else {
                &DAYS[day][..3]
            };
            self.reference_text(
                dc,
                &if agenda {
                    label.to_uppercase()
                } else {
                    label.into()
                },
                rect,
                if agenda { 8.5 } else { 11. },
                if agenda { 650 } else { 450 },
                ink,
                if mist { 0.84 } else { 0.44 },
                1,
            );
        }
        for (i, date) in dates.iter().enumerate() {
            let rect = if weekly { r.week_day(i) } else { l.day(i) };
            let current = *date == selected;
            let action = if weekly {
                Action::WeekDay(i)
            } else {
                Action::Day(i)
            };
            let outside = date.month != self.cursor.month;
            let center = [(rect[0] + rect[2]) * 0.5, (rect[1] + rect[3]) * 0.5];
            let size = (rect[2] - rect[0] - 4.)
                .min(rect[3] - rect[1] - 3.)
                .min(if weekly { 40. } else { 28. });
            let box_rect = [
                center[0] - size / 2.,
                center[1] - size / 2.,
                center[0] + size / 2.,
                center[1] + size / 2.,
            ];
            let radius = if soft { 5. } else { size / 2. };
            if current {
                c.round(box_rect, radius, accent, 255, false, false);
            } else if self.hover == Some(action) {
                c.round(
                    box_rect,
                    radius,
                    ink,
                    (25. * self.hover_alpha) as u8,
                    false,
                    false,
                );
            }
            if self.pressed == Some(action) {
                c.round(box_rect, radius, ink, 32, false, false);
            }
            self.reference_text(
                dc,
                &date.day.to_string(),
                rect,
                if weekly {
                    17.
                } else if agenda {
                    11.
                } else {
                    13.
                },
                if current { 650 } else { 450 },
                if current { 0xffffff } else { ink },
                if outside && !weekly { 0.2 } else { 1. },
                1,
            );
            if *date == self.today && !current {
                self.artwork
                    .border(dc, self.dpi, box_rect, radius, accent, 0.7, 1.);
            }
            if current && self.keyboard_day {
                self.artwork.border(
                    dc,
                    self.dpi,
                    [
                        box_rect[0] - 2.,
                        box_rect[1] - 2.,
                        box_rect[2] + 2.,
                        box_rect[3] + 2.,
                    ],
                    radius + 2.,
                    ink,
                    0.8,
                    1.,
                );
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
}
