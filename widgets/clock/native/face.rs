//! Nine content layouts, drawn with the dock's antialiased native canvas.
#[path = "layout.rs"]
mod layout;
#[path = "responsive.rs"]
mod responsive;
use super::model::*;
use crate::render::Canvas;
use windows_sys::Win32::Foundation::SYSTEMTIME;
fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}
pub fn color(theme: &str) -> u32 {
    match theme {
        "sunset" => rgb(164, 90, 78),
        "paper" => rgb(243, 242, 237),
        "neon" => rgb(16, 19, 16),
        "modular" => rgb(8, 9, 9),
        "ocean" => rgb(29, 42, 53),
        "rose" => rgb(16, 28, 19),
        "aurora" => rgb(23, 19, 38),
        "terminal" => rgb(14, 23, 17),
        _ => rgb(39, 46, 57),
    }
}
struct Face<'a> {
    c: &'a Canvas,
    s: f32,
    x: f32,
    y: f32,
    ink: u32,
    muted: u32,
    accent: u32,
    mono: bool,
}
impl Face<'_> {
    fn r(&self, r: [f32; 4]) -> [f32; 4] {
        [
            self.x + r[0] * self.s,
            self.y + r[1] * self.s,
            self.x + r[2] * self.s,
            self.y + r[3] * self.s,
        ]
    }
    fn text(&self, t: &str, r: [f32; 4], size: f32, bold: bool, color: u32, align: i32) {
        if size * self.s < 5.5 {
            return;
        }
        if self.mono {
            self.c
                .text_monospace(t, self.r(r), size * self.s, bold, color, align);
        } else {
            self.c.text(t, self.r(r), size * self.s, bold, color, align);
        }
    }
    fn tracked_text(
        &self,
        text: &str,
        r: [f32; 4],
        size: f32,
        bold: bool,
        color: u32,
        spacing: f32,
    ) {
        if size * self.s < 5.5 {
            return;
        }
        let mut x = r[0];
        for ch in text.chars() {
            if ch.is_whitespace() {
                x += size * 0.38 + spacing;
                continue;
            }
            let glyph = ch.to_string();
            self.c.text_alpha(
                &glyph,
                self.r([x, r[1], x + size * 3., r[3]]),
                size * self.s,
                bold,
                color,
                0,
                210,
            );
            x += (self.c.measure_clock_label(&glyph, size * self.s, bold) / self.s - size / 3.)
                .max(size * 0.22)
                + spacing;
        }
    }
    fn line(&self, p: &[(f32, f32)], color: u32, a: u8, width: f32) {
        let pts: Vec<_> = p
            .iter()
            .map(|p| (self.x + p.0 * self.s, self.y + p.1 * self.s))
            .collect();
        self.c.line(&pts, color, a, width * self.s);
    }
    fn round(&self, r: [f32; 4], radius: f32, color: u32, a: u8) {
        self.c
            .round(self.r(r), radius * self.s, color, a, false, false);
    }
    fn fit_digital(&self, t: &SYSTEMTIME, cfg: &Config, r: [f32; 4], max_size: f32) -> f32 {
        let bw = (r[2] - r[0]).max(10.);
        let bh = (r[3] - r[1]).max(10.);
        let mut size = max_size.min(bh * 0.92);
        let digits = digital(t, cfg.hour12);
        let is_display = ["lumen", "sunset", "neon"].contains(&cfg.theme.as_str());
        let probe = 40.;
        let mut w = if is_display {
            self.c.measure_clock_display(&digits, probe) / probe
        } else if self.mono {
            self.c.measure_clock_monospace(&digits, probe) / probe
        } else {
            self.c.measure_clock_label(&digits, probe, false) / probe
        };
        if cfg.hour12 {
            w += 0.72;
        }
        if w > 0.1 {
            size = size.min(bw / w);
        }
        size.max(10.)
    }
    fn digital_in(&self, t: &SYSTEMTIME, cfg: &Config, r: [f32; 4], size: f32, align: i32) {
        let digits = digital(t, cfg.hour12);
        let is_display = ["lumen", "sunset", "neon"].contains(&cfg.theme.as_str());
        let ampm = if t.wHour < 12 { "AM" } else { "PM" };
        let ampm_sz = (size * 0.24).max(7.5);
        let measure = |s: f32| {
            if is_display {
                self.c.measure_clock_display(&digits, s)
            } else if self.mono {
                self.c.measure_clock_monospace(&digits, s)
            } else {
                self.c.measure_clock_label(&digits, s, false)
            }
        };
        if cfg.hour12 {
            let dw = measure(size);
            let gap = size * 0.02;
            let aw = ampm_sz * 2.4;
            let bw = (r[2] - r[0]).max(10.);
            let start = match align {
                1 => r[0] + ((bw - (dw + gap + aw)).max(0.)) / 2.,
                2 => (r[2] - dw - gap - aw).max(r[0]),
                _ => r[0],
            };
            let d_rect = [start, r[1], (start + dw + 6.).min(r[2] - aw).max(start + 10.), r[3]];
            if is_display {
                self.c.text_clock_display(&digits, self.r(d_rect), size * self.s, self.ink, 0, false);
            } else {
                self.text(&digits, d_rect, size, false, self.ink, 0);
            }
            let ax = (start + dw + gap).min((r[2] - aw).max(r[0]));
            let cy = (r[1] + r[3]) / 2.;
            let ampm_cy = cy + (size - ampm_sz) * 0.34;
            self.text(
                ampm,
                [ax, ampm_cy - ampm_sz * 0.85, r[2], ampm_cy + ampm_sz * 0.85],
                ampm_sz,
                true,
                self.muted,
                0,
            );
        } else if is_display {
            self.c
                .text_clock_display(&digits, self.r(r), size * self.s, self.ink, align, false);
        } else {
            self.text(&digits, r, size, false, self.ink, align);
        }
    }
    fn digital(&self, t: &SYSTEMTIME, cfg: &Config, r: [f32; 4], size: f32, align: i32) {
        let r = if cfg.hour12 && align == 1 {
            let center = (r[0] + r[2]) / 2.;
            [center - size * 2., r[1], center + size * 2., r[3]]
        } else {
            r
        };
        let mut rect = r;
        if cfg.hour12 {
            rect[2] -= size * 0.7;
        }
        if ["lumen", "sunset", "neon"].contains(&cfg.theme.as_str()) {
            self.c.text_clock_display(
                &digital(t, cfg.hour12),
                self.r(rect),
                size * self.s,
                self.ink,
                align,
                false,
            );
        } else {
            self.text(&digital(t, cfg.hour12), rect, size, false, self.ink, align);
        }
        if cfg.hour12 {
            self.text(
                if t.wHour < 12 { "AM" } else { "PM" },
                [r[2] - size * 0.7, r[1] + (r[3] - r[1]) * 0.4, r[2], r[3]],
                size * 0.22,
                true,
                self.muted,
                0,
            );
        }
    }
    fn track(&self, y: f32, f: f32) {
        self.track_in([26., y, 314., y + 4.], f);
    }
    fn track_in(&self, r: [f32; 4], f: f32) {
        let h = (r[3] - r[1]).max(2.);
        let rad = h / 2.;
        let w = (r[2] - r[0]).max(1.);
        self.round(r, rad, self.ink, 35);
        self.round(
            [r[0], r[1], r[0] + w * f.clamp(0., 1.), r[3]],
            rad,
            self.accent,
            230,
        );
    }
    fn meter(&self, label: &str, value: &str, y: f32, f: f32, count: usize) {
        self.meter_in(label, value, [26., y, 314., y + 31.], f, count);
    }
    fn meter_in(&self, label: &str, value: &str, r: [f32; 4], f: f32, count: usize) {
        let accent = match label {
            "This hour" => rgb(41, 197, 143),
            "This minute" => rgb(86, 198, 231),
            _ => self.accent,
        };
        let w = (r[2] - r[0]).max(20.);
        let h = (r[3] - r[1]).max(18.);
        let label_sz = (h * 0.34).clamp(8., 11.);
        let label_h = (label_sz + 5.).min(h * 0.56);
        self.text(
            label,
            [r[0], r[1], r[0] + w * 0.62, r[1] + label_h],
            label_sz,
            false,
            self.muted,
            0,
        );
        self.text(
            value,
            [r[0] + w * 0.45, r[1], r[2], r[1] + label_h],
            label_sz,
            true,
            self.ink,
            2,
        );
        let segs = if w < 140. {
            count.min(16)
        } else if w < 200. {
            count.min(22)
        } else {
            count
        };
        let bar_top = r[1] + label_h + 3.;
        let bar_bot = r[3].max(bar_top + 4.);
        let width = w / segs as f32;
        let gap = (width * 0.25).clamp(1.5, 3.);
        for i in 0..segs {
            self.round(
                [
                    r[0] + i as f32 * width,
                    bar_top,
                    (r[0] + (i + 1) as f32 * width - gap).max(r[0] + i as f32 * width + 1.),
                    bar_bot,
                ],
                2.,
                if (i as f32) < (f * segs as f32).floor() {
                    accent
                } else {
                    self.ink
                },
                if (i as f32) < (f * segs as f32).floor() {
                    240
                } else {
                    32
                },
            );
        }
    }
    fn dial(&self, t: &SYSTEMTIME, cfg: &Config, cx: f32, cy: f32, r: f32) {
        let point = |a: f32, l: f32| (cx + a.sin() * l, cy - a.cos() * l);
        for i in 0..60 {
            let a = i as f32 * std::f32::consts::TAU / 60.;
            self.line(
                &[
                    point(a, r * if i % 5 == 0 { 0.81 } else { 0.9 }),
                    point(a, r),
                ],
                self.ink,
                if i % 5 == 0 { 200 } else { 75 },
                if i % 5 == 0 {
                    (r * 0.028).clamp(1.2, 2.0)
                } else {
                    (r * 0.014).clamp(0.7, 1.0)
                },
            );
        }
        let num_sz = r * 0.14;
        if num_sz * self.s >= 5.5 {
            for (n, x, y) in [
                ("12", cx, cy - r * 0.67),
                ("3", cx + r * 0.67, cy),
                ("6", cx, cy + r * 0.67),
                ("9", cx - r * 0.67, cy),
            ] {
                let pad = num_sz * 1.1;
                self.text(
                    n,
                    [x - pad, y - pad * 0.8, x + pad, y + pad * 0.8],
                    num_sz,
                    true,
                    self.muted,
                    1,
                );
            }
        }
        for (angle, len, width, color) in [
            (
                (t.wHour % 12) as f32 * 30. + t.wMinute as f32 * 0.5,
                r * 0.48,
                (r * 0.048).clamp(1.8, 3.2),
                self.ink,
            ),
            (
                t.wMinute as f32 * 6. + t.wSecond as f32 * 0.1,
                r * 0.75,
                (r * 0.032).clamp(1.3, 2.2),
                self.ink,
            ),
        ] {
            let a = angle.to_radians();
            self.line(&[point(a, -r * 0.1), point(a, len)], color, 255, width);
        }
        if cfg.show_seconds {
            let a = (t.wSecond as f32 * 6.).to_radians();
            self.line(
                &[point(a, -r * 0.17), point(a, r * 0.8)],
                rgb(231, 91, 75),
                245,
                (r * 0.018).clamp(0.9, 1.3),
            );
        }
        let pin = (r * 0.055).clamp(2.2, 3.4);
        self.round([cx - pin, cy - pin, cx + pin, cy + pin], pin, self.ink, 255);
    }
}
pub fn draw(canvas: &Canvas, cfg: &Config, t: &SYSTEMTIME, w: f32, h: f32, ink: u32, muted: u32) {
    draw_content(canvas, cfg, t, w, h, ink, muted, true);
}
pub(super) fn draw_content(
    canvas: &Canvas,
    cfg: &Config,
    t: &SYSTEMTIME,
    w: f32,
    h: f32,
    ink: u32,
    muted: u32,
    emblem: bool,
) {
    let mode = layout::mode(w, h);
    let (x, y, s) = if mode == layout::Mode::Classic {
        layout::frame(w, h)
    } else {
        (0., 0., 1.)
    };
    let accent = if crate::contrast::luminance(ink) < 0.5 {
        rgb(62, 102, 69)
    } else {
        match cfg.theme.as_str() {
            "sunset" => rgb(255, 195, 137),
            "ocean" => rgb(230, 205, 156),
            "aurora" => rgb(200, 168, 255),
            "neon" => rgb(185, 237, 77),
            "rose" => rgb(165, 221, 37),
            "terminal" => rgb(184, 237, 141),
            _ => ink,
        }
    };
    let f = Face {
        c: canvas,
        s,
        x,
        y,
        ink,
        muted,
        accent,
        mono: cfg.theme == "terminal",
    };
    let zone = if cfg.time_zone == "local" {
        "Local time".into()
    } else {
        cfg.time_zone
            .split('/')
            .next_back()
            .unwrap_or("UTC")
            .replace('_', " ")
    };
    let date = date(t);
    let seconds = format!("{:02} seconds", t.wSecond);
    let progress = progress(t);
    if mode != layout::Mode::Classic {
        responsive::draw(&f, cfg, t, mode, w, h, &date, &zone, progress, emblem);
        return;
    }
    match cfg.theme.as_str() {
        "lumen" => {
            f.digital(t, cfg, [22., 84., 317., 184.], 78., 0);
            f.text(&date, [26., 211., 233., 233.], 12., true, ink, 0);
            f.text(
                &if cfg.show_seconds {
                    format!("{seconds} · {zone}")
                } else {
                    zone
                },
                [26., 237., 231., 257.],
                10.,
                false,
                muted,
                0,
            );
            f.dial(t, cfg, 276., 230., 35.);
            f.track(288., progress);
        }
        "terminal" => {
            f.digital(t, cfg, [22., 92., 318., 187.], 74., 0);
            if cfg.show_seconds {
                f.text(
                    &format!(":{:02}", t.wSecond),
                    [26., 207., 104., 250.],
                    32.,
                    false,
                    accent,
                    0,
                );
                f.text(
                    "SECONDS",
                    [112., 219., 310., 243.],
                    10.,
                    true,
                    muted,
                    0,
                );
            }
            f.line(&[(26., 277.), (314., 277.)], accent, 100, 1.);
            f.text(
                &date.to_uppercase(),
                [26., 285., 314., 305.],
                10.,
                false,
                accent,
                0,
            );
        }
        "rose" => {
            f.digital(t, cfg, [23., 78., 318., 164.], 70., 0);
            f.meter(
                "Day elapsed",
                &format!("{}%", (progress * 100.) as u32),
                187.,
                progress,
                24,
            );
            f.meter(
                "This hour",
                &format!("{} / 60 min", t.wMinute),
                232.,
                t.wMinute as f32 / 60.,
                30,
            );
            if cfg.show_seconds {
                f.meter(
                    "This minute",
                    &format!("{} / 60 sec", t.wSecond),
                    277.,
                    t.wSecond as f32 / 60.,
                    30,
                );
            } else {
                f.text(&date, [26., 283., 314., 308.], 11., false, muted, 0);
            }
        }
        "ocean" => {
            f.text(&date, [26., 66., 263., 88.], 10., true, muted, 0);
            f.text(
                if cfg.hour12 {
                    if t.wHour < 12 {
                        "AM"
                    } else {
                        "PM"
                    }
                } else {
                    "24H"
                },
                [265., 66., 314., 88.],
                10.,
                true,
                muted,
                2,
            );
            for (i, n) in [hour(t.wHour, cfg.hour12), t.wMinute].iter().enumerate() {
                let x = 26. + i as f32 * 149.;
                f.round([x, 103., x + 139., 245.], 13., rgb(27, 36, 44), 255);
                f.round([x, 103., x + 139., 180.], 13., rgb(40, 50, 58), 255);
                f.round([x, 160., x + 139., 180.], 0., rgb(40, 50, 58), 255);
                canvas.text_clock_display(
                    &format!("{n:02}"),
                    f.r([x, 109., x + 139., 219.]),
                    88. * f.s,
                    rgb(255, 243, 217),
                    1,
                    true,
                );
                f.line(&[(x, 178.), (x + 139., 178.)], color("ocean"), 190, 2.);
                f.text(
                    if i == 0 { "HOURS" } else { "MINUTES" },
                    [x, 220., x + 139., 239.],
                    8.,
                    true,
                    rgb(196, 185, 165),
                    1,
                );
            }
            if cfg.show_seconds {
                f.meter(
                    "Seconds",
                    &format!("{:02}", t.wSecond),
                    272.,
                    t.wSecond as f32 / 60.,
                    30,
                );
            }
        }
        "aurora" => {
            f.line(&[(37., 96.), (37., 270.)], ink, 60, 1.);
            for (i, offset) in [-2, -1, 0, 1, 2].iter().enumerate() {
                let y = [100., 131., 170., 231., 262.][i];
                f.line(&[(33., y + 12.), (46., y + 12.)], accent, 200, 1.);
                if *offset == 0 {
                    f.digital(t, cfg, [58., y - 8., 314., y + 45.], 45., 0);
                    f.text(
                        &if cfg.show_seconds {
                            format!("NOW · {:02}s", t.wSecond)
                        } else {
                            "NOW".into()
                        },
                        [234., y + 40., 314., y + 56.],
                        8.,
                        true,
                        muted,
                        2,
                    );
                } else {
                    let h = ((t.wHour as i32 + offset + 24) % 24) as u16;
                    f.text(
                        &format!(
                            "{:02}:00{}",
                            hour(h, cfg.hour12),
                            if cfg.hour12 {
                                if h < 12 {
                                    " AM"
                                } else {
                                    " PM"
                                }
                            } else {
                                ""
                            }
                        ),
                        [59., y, 215., y + 25.],
                        17.,
                        false,
                        muted,
                        0,
                    );
                    f.text(
                        &format!(
                            "{}h {}",
                            offset.abs(),
                            if *offset < 0 { "ago" } else { "ahead" }
                        ),
                        [225., y, 314., y + 25.],
                        10.,
                        false,
                        muted,
                        2,
                    );
                }
            }
            f.text(&date, [26., 301., 314., 321.], 10., false, muted, 0);
        }
        "neon" => {
            let center = (170., 187.);
            let p = |a: f32, r: f32| (center.0 + a.sin() * r, center.1 - a.cos() * r);
            let ring: Vec<_> = (0..=180)
                .map(|i| p(i as f32 * std::f32::consts::TAU / 180., 116.))
                .collect();
            f.line(&ring, ink, 26, 5.);
            let arc: Vec<_> = (0..=180)
                .map(|i| p(i as f32 * std::f32::consts::TAU / 180. * progress, 116.))
                .collect();
            f.line(&arc, accent, 245, 5.);
            for i in 0..24 {
                let a = i as f32 * std::f32::consts::TAU / 24.;
                f.line(
                    &[p(a, 101.), p(a, 106.)],
                    ink,
                    if i <= t.wHour { 180 } else { 50 },
                    1.,
                );
            }
            f.text(&date, [67., 115., 273., 136.], 10., true, muted, 1);
            f.digital(t, cfg, [61., 148., 279., 215.], 51., 1);
            if cfg.show_seconds {
                f.text(
                    &seconds.to_uppercase(),
                    [66., 228., 274., 246.],
                    9.,
                    false,
                    muted,
                    1,
                );
            }
            f.text(
                &format!("{}% OF THE DAY", (progress * 100.) as u32),
                [60., 264., 280., 283.],
                9.,
                true,
                accent,
                1,
            );
        }
        "sunset" => {
            let digits = digital(t, cfg.hour12);
            let size = 84.;
            let left = 12.;
            canvas.text_clock_display(
                &digits,
                f.r([left, 86., 314., 187.]),
                size * f.s,
                ink,
                0,
                false,
            );
            if cfg.hour12 {
                let end =
                    left + canvas.measure_clock_display(&digits, size * f.s) / f.s - size * 0.12;
                f.c.text_light(
                    if t.wHour < 12 { "AM" } else { "PM" },
                    f.r([end, 132., 314., 187.]),
                    22. * f.s,
                    ink,
                    0,
                );
            }
            let weekday = date.split(',').next().unwrap_or("").to_uppercase();
            let full = date
                .split_once(", ")
                .map_or(date.as_str(), |(_, rest)| rest);
            f.tracked_text(&weekday, [26., 226., 225., 242.], 9., false, ink, 1.7);
            if f.s >= 0.45 || 16. * f.s >= 6. {
                canvas.text_medium(full, f.r([26., 244., 226., 269.]), 16. * f.s, ink, 0);
            }
            if emblem {
                draw_afterglow_motion(canvas, cfg, t, w, h, ink);
            }
            f.round([26., 282., 314., 284.5], 1.25, ink, 48);
            f.round([26., 282., 26. + 288. * progress, 284.5], 1.25, ink, 235);
            f.text(
                &format!("{}%", (progress * 100.) as u32),
                [268., 292., 314., 309.],
                8.,
                false,
                ink,
                2,
            );
        }
        "modular" => {
            let f = Face {
                ink: rgb(8, 13, 25),
                muted: rgb(24, 16, 12),
                ..f
            };
            let ink = f.ink;
            let muted = f.muted;
            f.round([26., 62., 314., 179.], 23., rgb(56, 110, 245), 255);
            f.digital(t, cfg, [35., 68., 306., 153.], 64., 1);
            if cfg.show_seconds {
                f.text(
                    &seconds.to_uppercase(),
                    [35., 152., 306., 173.],
                    9.,
                    true,
                    muted,
                    1,
                );
            }
            f.round([26., 189., 165., 326.], 23., rgb(255, 146, 38), 255);
            f.text(
                &date.split(',').next().unwrap_or(""),
                [36., 200., 155., 235.],
                26.,
                false,
                ink,
                1,
            );
            f.text(
                &t.wDay.to_string(),
                [36., 233., 155., 316.],
                62.,
                false,
                ink,
                1,
            );
            f.round([177., 189., 314., 326.], 68.5, rgb(255, 67, 44), 255);
            f.dial(t, cfg, 245.5, 257.5, 58.);
        }
        _ => {
            f.dial(t, cfg, 170., 170., 92.);
            f.digital(t, cfg, [26., 272., 314., 311.], 28., 1);
            f.text(&date, [26., 317., 314., 333.], 9., false, muted, 1);
        }
    }
}

pub(super) fn draw_afterglow_motion(
    c: &Canvas,
    cfg: &Config,
    t: &SYSTEMTIME,
    w: f32,
    h: f32,
    ink: u32,
) {
    super::afterglow::draw(c, responsive::afterglow_emblem_rect(w, h), cfg, t, ink, ink);
}

pub(super) fn draw_location(c: &Canvas, w: f32, h: f32, label: &str, ink: u32) {
    responsive::draw_location(c, w, h, label, ink);
}
