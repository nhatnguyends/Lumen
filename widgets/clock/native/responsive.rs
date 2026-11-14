//! Responsive theme layouts for Wide, Tall, and CompactSquare clock sizes.
use super::{hour, layout, Face};
use crate::render::Canvas;
use windows_sys::Win32::Foundation::SYSTEMTIME;

fn sunset_tall_metrics(w: f32, h: f32) -> (f32, f32, f32, f32) {
    let b = layout::content_box(w, h);
    let top = if w >= 140. && h >= 210. {
        b[1] + 30.
    } else {
        b[1]
    };
    let bar_y = b[3] - 20.;
    let date_top = bar_y - 42.;
    let avail = (date_top - top).max(60.);
    let time_h = (avail * 0.44).clamp(32., 84.);
    (top, time_h, date_top, bar_y)
}

pub(super) fn afterglow_emblem_rect(w: f32, h: f32) -> [f32; 4] {
    match layout::mode(w, h) {
        layout::Mode::Classic => {
            let (x, y, scale) = layout::frame(w, h);
            [
                x + 218. * scale,
                y + 194. * scale,
                x + 314. * scale,
                y + 263. * scale,
            ]
        }
        layout::Mode::Wide => {
            let b = layout::content_box(w, h);
            let bw = b[2] - b[0];
            let bh = b[3] - b[1];
            let emb_h = (bh * 0.78).clamp(48., 125.);
            let emb_w = (emb_h * 1.38).min(bw * 0.42);
            let emb_h = emb_w / 1.38;
            let cy = (b[1] + b[3]) / 2.;
            [b[2] - emb_w, cy - emb_h / 2., b[2], cy + emb_h / 2.]
        }
        layout::Mode::Tall => {
            let b = layout::content_box(w, h);
            let bw = b[2] - b[0];
            let (top, time_h, date_top, _) = sunset_tall_metrics(w, h);
            let emb_top = top + time_h + 6.;
            let rem_h = (date_top - 6. - emb_top).max(32.);
            let emb_w = (bw * 0.78).min(150.);
            let emb_h = (emb_w / 1.38).min(rem_h * 0.88);
            let emb_w = emb_h * 1.38;
            let cx = (b[0] + b[2]) / 2.;
            let cy = emb_top + rem_h / 2.;
            [cx - emb_w / 2., cy - emb_h / 2., cx + emb_w / 2., cy + emb_h / 2.]
        }
        layout::Mode::CompactSquare => {
            let b = layout::content_box(w, h);
            let bw = b[2] - b[0];
            let bh = b[3] - b[1];
            let emb_w = (bw * 0.44).clamp(44., 86.);
            let emb_h = (emb_w / 1.38).min(bh * 0.38);
            let emb_w = emb_h * 1.38;
            [b[2] - emb_w, b[3] - 14. - emb_h, b[2], b[3] - 14.]
        }
    }
}

pub(super) fn draw_location(c: &Canvas, w: f32, h: f32, label: &str, ink: u32) {
    match layout::mode(w, h) {
        layout::Mode::Classic => {
            let (x, y, s) = layout::frame(w, h);
            if 12. * s >= 7. && w >= 220. {
                let f = Face {
                    c,
                    s,
                    x,
                    y,
                    ink,
                    muted: ink,
                    accent: ink,
                    mono: false,
                };
                f.round([24.5, 32.5, 27.5, 35.5], 1.5, ink, 170);
                c.text_medium(label, f.r([34., 21., 246., 47.]), 12. * s, ink, 0);
            }
        }
        layout::Mode::Wide if w >= 220. && h >= 135. => {
            c.round([22., 26.5, 25., 29.5], 1.5, ink, 170, false, false);
            c.text_medium(label, [30., 19., w * 0.56, 37.], 10.5, ink, 0);
        }
        layout::Mode::Tall if w >= 140. && h >= 210. => {
            c.round([22., 29.5, 25., 32.5], 1.5, ink, 170, false, false);
            c.text_medium(label, [30., 22., w - 22., 40.], 10.5, ink, 0);
        }
        layout::Mode::CompactSquare if w >= 165. && h >= 165. => {
            c.round([20., 25.5, 23., 28.5], 1.5, ink, 170, false, false);
            c.text_medium(label, [28., 18., w - 76., 36.], 10., ink, 0);
        }
        _ => {}
    }
}

fn short_date(s: &str) -> String {
    let mut parts = s.splitn(2, ", ");
    if let (Some(wk), Some(rest)) = (parts.next(), parts.next()) {
        let md = rest.split(',').next().unwrap_or(rest);
        let mut w = md.split_whitespace();
        if let (Some(mon), Some(day)) = (w.next(), w.next()) {
            let short_m: String = mon.chars().take(3).collect();
            return format!("{wk}, {short_m} {day}");
        }
    }
    s.to_string()
}

fn short_month_date(s: &str) -> String {
    let mut w = s.splitn(2, ' ');
    if let (Some(mon), Some(rest)) = (w.next(), w.next()) {
        let short_m: String = mon.chars().take(3).collect();
        return format!("{short_m} {rest}");
    }
    s.to_string()
}

pub(super) fn draw(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    w: f32,
    h: f32,
    date_str: &str,
    zone: &str,
    progress: f32,
    emblem: bool,
) {
    let b = layout::content_box(w, h);
    let seconds = format!("{:02} seconds", t.wSecond);
    match cfg.theme.as_str() {
        "lumen" => draw_lumen(f, cfg, t, mode, b, date_str, &seconds, zone, progress),
        "terminal" => draw_terminal(f, cfg, t, mode, b, date_str),
        "rose" => draw_rose(f, cfg, t, mode, b, date_str, progress),
        "ocean" => draw_ocean(f, cfg, t, mode, b, date_str),
        "aurora" => draw_aurora(f, cfg, t, mode, b, date_str),
        "neon" => draw_neon(f, cfg, t, mode, b, date_str, &seconds, progress),
        "sunset" => draw_sunset(f, cfg, t, mode, b, w, h, date_str, progress, emblem),
        "modular" => draw_modular(f, cfg, t, mode, b, date_str, &seconds),
        _ => draw_paper(f, cfg, t, mode, b, date_str),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_lumen(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    b: [f32; 4],
    date_str: &str,
    seconds: &str,
    zone: &str,
    progress: f32,
) {
    let bw = b[2] - b[0];
    let bh = b[3] - b[1];
    let sub = if cfg.show_seconds {
        format!("{seconds} · {zone}")
    } else {
        zone.to_string()
    };
    match mode {
        layout::Mode::Wide => {
            let dial_r = (bh * 0.44).min(bw * 0.19).max(24.);
            let dial_cx = b[2] - dial_r - 4.;
            let dial_cy = (b[1] + b[3]) / 2.;
            f.dial(t, cfg, dial_cx, dial_cy, dial_r);
            let lx1 = dial_cx - dial_r - 14.;
            let top = b[1];
            let track_y = b[3] - 4.;
            let footer_h = if bh >= 100. { 32. } else { 18. };
            let time_r = [b[0], top, lx1, (track_y - footer_h - 4.).max(top + 28.)];
            let sz = f.fit_digital(t, cfg, time_r, 84.);
            f.digital_in(t, cfg, time_r, sz, 0);
            if bh >= 100. {
                f.text(date_str, [b[0], track_y - 33., lx1, track_y - 18.], 11., true, f.ink, 0);
                f.text(&sub, [b[0], track_y - 18., lx1, track_y - 4.], 9.5, false, f.muted, 0);
            } else {
                f.text(date_str, [b[0], track_y - 18., lx1, track_y - 4.], 10., true, f.ink, 0);
            }
            f.track_in([b[0], track_y, lx1, track_y + 4.], progress);
        }
        layout::Mode::Tall => {
            let dial_r = (bw * 0.40).min(bh * 0.21).max(24.);
            let dial_cy = b[1] + dial_r + 6.;
            f.dial(t, cfg, (b[0] + b[2]) / 2., dial_cy, dial_r);
            let time_top = dial_cy + dial_r + 12.;
            let track_y = b[3] - 4.;
            let time_r = [b[0], time_top, b[2], (track_y - 40.).max(time_top + 28.)];
            let sz = f.fit_digital(t, cfg, time_r, 72.);
            f.digital_in(t, cfg, time_r, sz, 1);
            f.text(date_str, [b[0], track_y - 36., b[2], track_y - 20.], 11., true, f.ink, 1);
            f.text(&sub, [b[0], track_y - 20., b[2], track_y - 5.], 9.5, false, f.muted, 1);
            f.track_in([b[0], track_y, b[2], track_y + 4.], progress);
        }
        _ => {
            let track_y = b[3] - 4.;
            let has_dial = bh >= 118. && bw >= 118.;
            let time_h = (bh * if has_dial { 0.44 } else { 0.56 }).max(28.);
            let time_r = [b[0], b[1], b[2], b[1] + time_h];
            let sz = f.fit_digital(t, cfg, time_r, 64.);
            f.digital_in(t, cfg, time_r, sz, 1);
            if has_dial {
                let dial_r = ((track_y - (b[1] + time_h) - 8.) * 0.44)
                    .min(bw * 0.21)
                    .max(18.);
                let dial_cx = b[2] - dial_r - 2.;
                let dial_cy = (b[1] + time_h + track_y - 4.) / 2.;
                f.dial(t, cfg, dial_cx, dial_cy, dial_r);
                let lx1 = dial_cx - dial_r - 8.;
                f.text(date_str, [b[0], dial_cy - 14., lx1, dial_cy + 2.], 10., true, f.ink, 0);
                f.text(
                    if cfg.show_seconds { seconds } else { zone },
                    [b[0], dial_cy + 2., lx1, dial_cy + 16.],
                    8.5,
                    false,
                    f.muted,
                    0,
                );
            } else {
                f.text(date_str, [b[0], b[1] + time_h + 2., b[2], track_y - 4.], 10., true, f.ink, 1);
            }
            f.track_in([b[0], track_y, b[2], track_y + 4.], progress);
        }
    }
}

fn draw_terminal(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    b: [f32; 4],
    date_str: &str,
) {
    let bw = b[2] - b[0];
    let bh = b[3] - b[1];
    let sec_str = format!(":{:02}", t.wSecond);
    match mode {
        layout::Mode::Wide => {
            let bot_y = b[3] - 18.;
            if cfg.show_seconds {
                let rx0 = b[0] + bw * 0.68;
                let time_r = [b[0], b[1], rx0 - 10., (bot_y - 8.).max(b[1] + 28.)];
                let sz = f.fit_digital(t, cfg, time_r, 86.);
                f.digital_in(t, cfg, time_r, sz, 0);
                let sec_sz = (sz * 0.48).clamp(16., 34.);
                let mid_y = (b[1] + bot_y - 8.) / 2.;
                f.text(&sec_str, [rx0, mid_y - sec_sz * 0.85, b[2], mid_y + 4.], sec_sz, false, f.accent, 2);
                f.text("SECONDS", [rx0, mid_y + 4., b[2], mid_y + 18.], 8.5, true, f.muted, 2);
            } else {
                let time_r = [b[0], b[1], b[2], (bot_y - 8.).max(b[1] + 28.)];
                let sz = f.fit_digital(t, cfg, time_r, 92.);
                f.digital_in(t, cfg, time_r, sz, 0);
            }
            f.line(&[(b[0], bot_y - 4.), (b[2], bot_y - 4.)], f.accent, 100, 1.);
            f.text(&date_str.to_uppercase(), [b[0], bot_y, b[2], b[3]], 9.5, false, f.accent, 0);
        }
        layout::Mode::Tall => {
            let bot_y = b[3] - 20.;
            let time_r = [b[0], b[1] + 8., b[2], b[1] + 8. + bh * 0.40];
            let sz = f.fit_digital(t, cfg, time_r, 72.);
            f.digital_in(t, cfg, time_r, sz, 0);
            if cfg.show_seconds {
                let sec_y = time_r[3] + 10.;
                f.text(&sec_str, [b[0], sec_y, b[2], sec_y + 38.], 30., false, f.accent, 0);
                f.text("SECONDS", [b[0], sec_y + 38., b[2], sec_y + 54.], 9.5, true, f.muted, 0);
            }
            f.line(&[(b[0], bot_y - 6.), (b[2], bot_y - 6.)], f.accent, 100, 1.);
            f.text(&date_str.to_uppercase(), [b[0], bot_y, b[2], b[3]], 9.5, false, f.accent, 0);
        }
        _ => {
            let bot_y = b[3] - 18.;
            let sec_space = if cfg.show_seconds { 26. } else { 8. };
            let time_r = [b[0], b[1], b[2], (bot_y - sec_space).max(b[1] + 28.)];
            let sz = f.fit_digital(t, cfg, time_r, 68.);
            f.digital_in(t, cfg, time_r, sz, 0);
            if cfg.show_seconds {
                f.text(&sec_str, [b[0], bot_y - 26., b[2], bot_y - 6.], 16., false, f.accent, 0);
            }
            f.line(&[(b[0], bot_y - 4.), (b[2], bot_y - 4.)], f.accent, 100, 1.);
            let dt = if bw < 160. { short_date(date_str) } else { date_str.to_string() };
            f.text(&dt.to_uppercase(), [b[0], bot_y, b[2], b[3]], 8.5, false, f.accent, 0);
        }
    }
}

fn draw_rose(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    b: [f32; 4],
    date_str: &str,
    progress: f32,
) {
    let bw = b[2] - b[0];
    let bh = b[3] - b[1];
    let pct = format!("{}%", (progress * 100.) as u32);
    let min_str = format!("{} / 60 min", t.wMinute);
    let sec_str = format!("{} / 60 sec", t.wSecond);
    match mode {
        layout::Mode::Wide => {
            let split = b[0] + bw * 0.48;
            let time_r = [b[0], b[1], split - 10., (b[3] - 18.).max(b[1] + 28.)];
            let sz = f.fit_digital(t, cfg, time_r, 82.);
            f.digital_in(t, cfg, time_r, sz, 0);
            f.text(date_str, [b[0], b[3] - 16., split - 8., b[3]], 9.5, false, f.muted, 0);

            let rx0 = split + 10.;
            let count = if bh >= 105. && cfg.show_seconds { 3 } else { 2 };
            let mh = ((bh - (count - 1) as f32 * 8.) / count as f32).clamp(20., 34.);
            let total_h = count as f32 * mh + (count - 1) as f32 * 8.;
            let mut my = (b[1] + b[3] - total_h) / 2.;
            f.meter_in("Day elapsed", &pct, [rx0, my, b[2], my + mh], progress, 24);
            my += mh + 8.;
            f.meter_in("This hour", &min_str, [rx0, my, b[2], my + mh], t.wMinute as f32 / 60., 30);
            if count == 3 {
                my += mh + 8.;
                f.meter_in("This minute", &sec_str, [rx0, my, b[2], my + mh], t.wSecond as f32 / 60., 30);
            }
        }
        layout::Mode::Tall => {
            let time_r = [b[0], b[1] + 4., b[2], b[1] + 4. + bh * 0.30];
            let sz = f.fit_digital(t, cfg, time_r, 68.);
            f.digital_in(t, cfg, time_r, sz, 0);
            let mut my = time_r[3] + 14.;
            let mh = ((b[3] - my - 24.) / 3.).clamp(24., 36.);
            f.meter_in("Day elapsed", &pct, [b[0], my, b[2], my + mh], progress, 24);
            my += mh + 10.;
            f.meter_in("This hour", &min_str, [b[0], my, b[2], my + mh], t.wMinute as f32 / 60., 30);
            my += mh + 10.;
            if cfg.show_seconds {
                f.meter_in("This minute", &sec_str, [b[0], my, b[2], my + mh], t.wSecond as f32 / 60., 30);
            } else {
                f.text(date_str, [b[0], my + 6., b[2], my + 24.], 10.5, false, f.muted, 0);
            }
        }
        _ => {
            let count = if bh >= 145. && cfg.show_seconds { 3 } else { 2 };
            let mh = if bh >= 145. { 26. } else { 20. };
            let meters_h = count as f32 * mh + (count - 1) as f32 * 5.;
            let time_r = [b[0], b[1], b[2], (b[3] - meters_h - 6.).max(b[1] + 28.)];
            let sz = f.fit_digital(t, cfg, time_r, 64.);
            f.digital_in(t, cfg, time_r, sz, 0);
            let mut my = b[3] - meters_h;
            f.meter_in("Day elapsed", &pct, [b[0], my, b[2], my + mh], progress, 20);
            my += mh + 5.;
            f.meter_in("This hour", &min_str, [b[0], my, b[2], my + mh], t.wMinute as f32 / 60., 20);
            if count == 3 {
                my += mh + 5.;
                f.meter_in("This minute", &sec_str, [b[0], my, b[2], my + mh], t.wSecond as f32 / 60., 20);
            }
        }
    }
}

fn draw_ocean(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    b: [f32; 4],
    date_str: &str,
) {
    let bw = b[2] - b[0];
    let bh = b[3] - b[1];
    let badge = if cfg.hour12 {
        if t.wHour < 12 { "AM" } else { "PM" }
    } else {
        "24H"
    };
    let draw_card = |x0: f32, y0: f32, x1: f32, y1: f32, val: u16, label: &str| {
        let cw = x1 - x0;
        let ch = y1 - y0;
        let rad = (cw.min(ch) * 0.11).clamp(6., 13.);
        let mid = y0 + ch * 0.52;
        f.round([x0, y0, x1, y1], rad, super::rgb(27, 36, 44), 255);
        f.round([x0, y0, x1, mid], rad, super::rgb(40, 50, 58), 255);
        f.round([x0, (mid - rad).max(y0), x1, mid], 0., super::rgb(40, 50, 58), 255);
        let dig_sz = (ch * 0.64).min(cw * 0.60).max(16.);
        let lbl_sz = (ch * 0.08).clamp(6.5, 9.);
        f.c.text_clock_display(
            &format!("{val:02}"),
            f.r([x0, y0 + ch * 0.04, x1, y1 - lbl_sz - 6.]),
            dig_sz * f.s,
            super::rgb(255, 243, 217),
            1,
            true,
        );
        f.line(&[(x0, mid), (x1, mid)], super::color("ocean"), 190, 1.8);
        if ch >= 48. {
            f.text(
                label,
                [x0, y1 - lbl_sz - 6., x1, y1 - 2.],
                lbl_sz,
                true,
                super::rgb(196, 185, 165),
                1,
            );
        }
    };
    match mode {
        layout::Mode::Wide => {
            let info_w = (bw * 0.25).clamp(72., 124.);
            let cards_r = b[2] - info_w - 12.;
            let cw = (cards_r - b[0] - 10.) / 2.;
            draw_card(b[0], b[1], b[0] + cw, b[3], hour(t.wHour, cfg.hour12), "HOURS");
            draw_card(b[0] + cw + 10., b[1], cards_r, b[3], t.wMinute, "MINUTES");

            let ix0 = cards_r + 12.;
            let dt = if info_w < 120. { short_date(date_str) } else { date_str.to_string() };
            f.text(&dt, [ix0, b[1], b[2], b[1] + 18.], 9.5, true, f.muted, 0);
            f.text(badge, [ix0, b[1] + 18., b[2], b[1] + 34.], 10., true, f.accent, 0);
            if cfg.show_seconds {
                if bh >= 105. {
                    f.meter_in(
                        "Seconds",
                        &format!("{:02}", t.wSecond),
                        [ix0, b[3] - 30., b[2], b[3]],
                        t.wSecond as f32 / 60.,
                        16,
                    );
                } else {
                    f.text(
                        &format!(":{:02}", t.wSecond),
                        [ix0, b[3] - 28., b[2], b[3]],
                        22.,
                        true,
                        f.ink,
                        0,
                    );
                }
            }
        }
        layout::Mode::Tall => {
            let dt = if bw < 160. { short_date(date_str) } else { date_str.to_string() };
            f.text(&dt, [b[0], b[1], b[2] - 32., b[1] + 16.], 9.5, true, f.muted, 0);
            f.text(badge, [b[2] - 34., b[1], b[2], b[1] + 16.], 9.5, true, f.muted, 2);
            let bot_h = if cfg.show_seconds { 36. } else { 4. };
            let avail_h = (b[3] - bot_h - (b[1] + 22.)).max(60.);
            let ch = (avail_h - 10.) / 2.;
            draw_card(b[0], b[1] + 20., b[2], b[1] + 20. + ch, hour(t.wHour, cfg.hour12), "HOURS");
            draw_card(b[0], b[1] + 30. + ch, b[2], b[1] + 30. + 2. * ch, t.wMinute, "MINUTES");
            if cfg.show_seconds {
                f.meter_in(
                    "Seconds",
                    &format!("{:02}", t.wSecond),
                    [b[0], b[3] - 28., b[2], b[3]],
                    t.wSecond as f32 / 60.,
                    20,
                );
            }
        }
        _ => {
            let dt = if bw < 160. { short_date(date_str) } else { date_str.to_string() };
            f.text(&dt, [b[0], b[1], b[2] - 32., b[1] + 15.], 9., true, f.muted, 0);
            f.text(badge, [b[2] - 34., b[1], b[2], b[1] + 15.], 9., true, f.muted, 2);
            let bot_h = if !cfg.show_seconds {
                4.
            } else if bh >= 120. {
                32.
            } else {
                18.
            };
            let card_top = b[1] + 18.;
            let card_bot = (b[3] - bot_h - 4.).max(card_top + 40.);
            let cw = (bw - 8.) / 2.;
            draw_card(b[0], card_top, b[0] + cw, card_bot, hour(t.wHour, cfg.hour12), "HOURS");
            draw_card(b[0] + cw + 8., card_top, b[2], card_bot, t.wMinute, "MINUTES");
            if cfg.show_seconds && bh >= 120. {
                f.meter_in(
                    "Seconds",
                    &format!("{:02}", t.wSecond),
                    [b[0], b[3] - 26., b[2], b[3]],
                    t.wSecond as f32 / 60.,
                    20,
                );
            } else if cfg.show_seconds {
                f.text(
                    &format!("{:02} SEC", t.wSecond),
                    [b[0], b[3] - 14., b[2], b[3]],
                    8.5,
                    true,
                    f.muted,
                    1,
                );
            }
        }
    }
}

fn draw_aurora(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    b: [f32; 4],
    date_str: &str,
) {
    let bw = b[2] - b[0];
    let bh = b[3] - b[1];
    let now_lbl = if cfg.show_seconds {
        format!("NOW · {:02}s", t.wSecond)
    } else {
        "NOW".into()
    };
    let draw_timeline = |rx0: f32, ry0: f32, rx1: f32, ry1: f32, five: bool| {
        let offsets: &[i32] = if five { &[-2, -1, 0, 1, 2] } else { &[-1, 0, 1] };
        let line_x = rx0 + 8.;
        f.line(&[(line_x, ry0 + 6.), (line_x, ry1 - 6.)], f.ink, 60, 1.);
        let step = (ry1 - ry0) / offsets.len().max(1) as f32;
        for (i, offset) in offsets.iter().enumerate() {
            let cy = ry0 + step * (i as f32 + 0.5);
            f.line(&[(line_x - 4., cy), (line_x + 8., cy)], f.accent, 200, 1.);
            if *offset == 0 {
                f.text(&now_lbl, [line_x + 14., cy - 9., rx1, cy + 9.], 11., true, f.ink, 0);
            } else {
                let h = ((t.wHour as i32 + offset + 24) % 24) as u16;
                let htxt = format!(
                    "{:02}:00{}",
                    hour(h, cfg.hour12),
                    if cfg.hour12 {
                        if h < 12 { " AM" } else { " PM" }
                    } else {
                        ""
                    }
                );
                f.text(&htxt, [line_x + 14., cy - 10., rx1 - 48., cy + 10.], 12., false, f.muted, 0);
                f.text(
                    &format!("{}h {}", offset.abs(), if *offset < 0 { "ago" } else { "ahead" }),
                    [rx1 - 52., cy - 8., rx1, cy + 8.],
                    8.5,
                    false,
                    f.muted,
                    2,
                );
            }
        }
    };
    match mode {
        layout::Mode::Wide => {
            let split = b[0] + bw * 0.50;
            let time_r = [b[0], b[1], split - 10., (b[3] - 20.).max(b[1] + 28.)];
            let sz = f.fit_digital(t, cfg, time_r, 76.);
            f.digital_in(t, cfg, time_r, sz, 0);
            f.text(
                date_str,
                [b[0], b[3] - 16., split - 8., b[3]],
                9.5,
                false,
                f.muted,
                0,
            );
            draw_timeline(split + 10., b[1], b[2], b[3], bh >= 115.);
        }
        layout::Mode::Tall => {
            let line_x = b[0] + 10.;
            let ry0 = b[1] + 8.;
            let ry1 = b[3] - 24.;
            f.line(&[(line_x, ry0 + 8.), (line_x, ry1 - 8.)], f.ink, 60, 1.);
            let ys = [
                ry0,
                ry0 + (ry1 - ry0) * 0.18,
                ry0 + (ry1 - ry0) * 0.36,
                ry0 + (ry1 - ry0) * 0.72,
                ry0 + (ry1 - ry0) * 0.90,
            ];
            for (i, offset) in [-2, -1, 0, 1, 2].iter().enumerate() {
                let y = ys[i];
                f.line(&[(line_x - 4., y + 10.), (line_x + 8., y + 10.)], f.accent, 200, 1.);
                if *offset == 0 {
                    let time_r = [line_x + 14., y - 6., b[2], y + 48.];
                    let sz = f.fit_digital(t, cfg, time_r, 46.);
                    f.digital_in(t, cfg, time_r, sz, 0);
                    f.text(&now_lbl, [line_x + 14., y + 44., b[2], y + 58.], 8.5, true, f.muted, 0);
                } else {
                    let h = ((t.wHour as i32 + offset + 24) % 24) as u16;
                    f.text(
                        &format!("{:02}:00", hour(h, cfg.hour12)),
                        [line_x + 14., y, b[2] - 46., y + 20.],
                        14.,
                        false,
                        f.muted,
                        0,
                    );
                    f.text(
                        &format!("{}h {}", offset.abs(), if *offset < 0 { "ago" } else { "ahead" }),
                        [b[2] - 50., y + 2., b[2], y + 20.],
                        9.,
                        false,
                        f.muted,
                        2,
                    );
                }
            }
            f.text(date_str, [b[0], b[3] - 16., b[2], b[3]], 9.5, false, f.muted, 0);
        }
        _ => {
            let time_r = [b[0], b[1] + 18., b[2], (b[3] - 36.).max(b[1] + 46.)];
            let prev_h = ((t.wHour as i32 + 23) % 24) as u16;
            let next_h = ((t.wHour as i32 + 1) % 24) as u16;
            f.text(&format!("{:02}:00 · 1h ago", hour(prev_h, cfg.hour12)), [b[0], b[1], b[2], b[1] + 16.], 9.5, false, f.muted, 0);
            let sz = f.fit_digital(t, cfg, time_r, 56.);
            f.digital_in(t, cfg, time_r, sz, 0);
            f.text(&format!("{:02}:00 · 1h ahead", hour(next_h, cfg.hour12)), [b[0], b[3] - 34., b[2], b[3] - 18.], 9.5, false, f.muted, 0);
            f.text(date_str, [b[0], b[3] - 16., b[2], b[3]], 9., false, f.muted, 0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_neon(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    b: [f32; 4],
    date_str: &str,
    seconds: &str,
    progress: f32,
) {
    let bw = b[2] - b[0];
    let bh = b[3] - b[1];
    let pct_str = format!("{}%", (progress * 100.) as u32);
    let draw_ring = |cx: f32, cy: f32, r: f32| {
        let p = |a: f32, rad: f32| (cx + a.sin() * rad, cy - a.cos() * rad);
        let ring: Vec<_> = (0..=180)
            .map(|i| p(i as f32 * std::f32::consts::TAU / 180., r))
            .collect();
        let stroke = (r * 0.048).clamp(2.5, 5.);
        f.line(&ring, f.ink, 26, stroke);
        let arc: Vec<_> = (0..=180)
            .map(|i| p(i as f32 * std::f32::consts::TAU / 180. * progress, r))
            .collect();
        f.line(&arc, f.accent, 245, stroke);
        for i in 0..24 {
            let a = i as f32 * std::f32::consts::TAU / 24.;
            f.line(
                &[p(a, r * 0.86), p(a, r * 0.91)],
                f.ink,
                if i <= t.wHour { 180 } else { 50 },
                1.,
            );
        }
    };
    match mode {
        layout::Mode::Wide => {
            let ring_r = (bh * 0.44).min(bw * 0.22).max(24.);
            let cx = b[2] - ring_r - 4.;
            let cy = (b[1] + b[3]) / 2.;
            draw_ring(cx, cy, ring_r);
            let pct_sz = (ring_r * 0.44).clamp(11., 24.);
            f.text(&pct_str, [cx - ring_r * 0.75, cy - pct_sz * 0.75, cx + ring_r * 0.75, cy + pct_sz * 0.35], pct_sz, true, f.ink, 1);
            f.text("OF DAY", [cx - ring_r * 0.75, cy + pct_sz * 0.3, cx + ring_r * 0.75, cy + pct_sz * 0.85], (ring_r * 0.16).clamp(6.5, 9.), true, f.accent, 1);

            let lx1 = cx - ring_r - 16.;
            f.text(date_str, [b[0], b[1], lx1, b[1] + 16.], 10., true, f.muted, 0);
            let time_r = [b[0], b[1] + 18., lx1, (b[3] - 20.).max(b[1] + 46.)];
            let sz = f.fit_digital(t, cfg, time_r, 76.);
            f.digital_in(t, cfg, time_r, sz, 0);
            let bot = if cfg.show_seconds {
                format!("{} · {}% OF THE DAY", seconds.to_uppercase(), (progress * 100.) as u32)
            } else {
                format!("{}% OF THE DAY", (progress * 100.) as u32)
            };
            f.text(&bot, [b[0], b[3] - 16., lx1, b[3]], 8.5, true, f.accent, 0);
        }
        layout::Mode::Tall => {
            let ring_r = (bw * 0.42).min(bh * 0.24).max(24.);
            let cx = (b[0] + b[2]) / 2.;
            let cy = b[1] + ring_r + 6.;
            draw_ring(cx, cy, ring_r);
            let pct_sz = (ring_r * 0.44).clamp(11., 24.);
            f.text(&pct_str, [cx - ring_r * 0.75, cy - pct_sz * 0.75, cx + ring_r * 0.75, cy + pct_sz * 0.35], pct_sz, true, f.ink, 1);
            f.text("OF DAY", [cx - ring_r * 0.75, cy + pct_sz * 0.3, cx + ring_r * 0.75, cy + pct_sz * 0.85], 8., true, f.accent, 1);

            let time_top = cy + ring_r + 14.;
            let time_r = [b[0], time_top, b[2], (b[3] - 40.).max(time_top + 28.)];
            let sz = f.fit_digital(t, cfg, time_r, 64.);
            f.digital_in(t, cfg, time_r, sz, 1);
            if cfg.show_seconds {
                f.text(&seconds.to_uppercase(), [b[0], b[3] - 34., b[2], b[3] - 18.], 9., false, f.muted, 1);
            }
            f.text(date_str, [b[0], b[3] - 17., b[2], b[3]], 9.5, true, f.muted, 1);
        }
        _ => {
            let ring_r = (bw.min(bh) * 0.45).max(36.);
            let cx = (b[0] + b[2]) / 2.;
            let cy = (b[1] + b[3]) / 2.;
            draw_ring(cx, cy, ring_r);
            let inner_w = ring_r * 1.48;
            let time_r = [cx - inner_w / 2., cy - ring_r * 0.36, cx + inner_w / 2., cy + ring_r * 0.36];
            let sz = f.fit_digital(t, cfg, time_r, 46.);
            f.digital_in(t, cfg, time_r, sz, 1);
            if ring_r >= 58. {
                f.text(date_str, [cx - ring_r * 0.65, cy - ring_r * 0.62, cx + ring_r * 0.65, cy - ring_r * 0.38], 8.5, true, f.muted, 1);
                f.text(&format!("{}% OF DAY", (progress * 100.) as u32), [cx - ring_r * 0.65, cy + ring_r * 0.38, cx + ring_r * 0.65, cy + ring_r * 0.62], 8., true, f.accent, 1);
            } else {
                f.text(&pct_str, [cx - ring_r * 0.5, cy + ring_r * 0.34, cx + ring_r * 0.5, cy + ring_r * 0.64], 8.5, true, f.accent, 1);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_sunset(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    b: [f32; 4],
    w: f32,
    h: f32,
    date_str: &str,
    progress: f32,
    emblem: bool,
) {
    let weekday = date_str.split(',').next().unwrap_or("").to_uppercase();
    let full = date_str
        .split_once(", ")
        .map_or(date_str, |(_, rest)| rest);
    let emb = afterglow_emblem_rect(w, h);
    if emblem {
        super::super::afterglow::draw(f.c, emb, cfg, t, f.ink, f.ink);
    }
    match mode {
        layout::Mode::Wide => {
            let lx1 = emb[0] - 12.;
            let top = if w >= 220. && h >= 135. { b[1] + 24. } else { b[1] };
            let bh_left = b[3] - top;
            let bar_y = b[3] - 18.;
            let time_bot = (bar_y - if bh_left >= 95. { 24. } else { 18. }).max(top + 28.);
            let time_r = [b[0], top, lx1, time_bot];
            let sz = f.fit_digital(t, cfg, time_r, 82.);
            f.digital_in(t, cfg, time_r, sz, 0);
            f.text(&format!("{weekday} · {full}"), [b[0], time_bot + 2., lx1, bar_y - 3.], 11., false, f.ink, 0);
            f.round([b[0], bar_y, lx1, bar_y + 2.5], 1.25, f.ink, 48);
            f.round([b[0], bar_y, b[0] + (lx1 - b[0]) * progress, bar_y + 2.5], 1.25, f.ink, 235);
            f.text(&format!("{}%", (progress * 100.) as u32), [lx1 - 38., bar_y + 5., lx1, b[3]], 8., false, f.ink, 2);
        }
        layout::Mode::Tall => {
            let (top, time_h, _, bar_y) = sunset_tall_metrics(w, h);
            let time_r = [b[0], top, b[2], top + time_h];
            let sz = f.fit_digital(t, cfg, time_r, 64.);
            f.digital_in(t, cfg, time_r, sz, 1);
            f.text(&weekday, [b[0], bar_y - 38., b[2], bar_y - 22.], 8.5, false, f.ink, 1);
            let mfull = if b[2] - b[0] < 160. { short_month_date(full) } else { full.to_string() };
            f.c.text_medium(&mfull, [b[0], bar_y - 22., b[2], bar_y - 3.], 13., f.ink, 1);
            f.round([b[0], bar_y, b[2], bar_y + 2.5], 1.25, f.ink, 48);
            f.round([b[0], bar_y, b[0] + (b[2] - b[0]) * progress, bar_y + 2.5], 1.25, f.ink, 235);
            f.text(&format!("{}%", (progress * 100.) as u32), [b[2] - 38., bar_y + 5., b[2], b[3]], 8., false, f.ink, 2);
        }
        _ => {
            let top = if w >= 165. && h >= 165. { b[1] + 22. } else { b[1] };
            let bar_y = b[3] - 12.;
            let time_r = [b[0], top, b[2], (emb[1] - 4.).max(top + 28.)];
            let sz = f.fit_digital(t, cfg, time_r, 56.);
            f.digital_in(t, cfg, time_r, sz, 0);
            let lx1 = emb[0] - 6.;
            f.text(&weekday, [b[0], emb[1] + 2., lx1, emb[1] + 16.], 8., false, f.ink, 0);
            let mfull = if lx1 - b[0] < 130. { short_month_date(full) } else { full.to_string() };
            f.c.text_medium(&mfull, [b[0], emb[1] + 16., lx1, bar_y - 3.], 11.5, f.ink, 0);
            f.round([b[0], bar_y, b[2], bar_y + 2.5], 1.25, f.ink, 48);
            f.round([b[0], bar_y, b[0] + (b[2] - b[0]) * progress, bar_y + 2.5], 1.25, f.ink, 235);
            f.text(&format!("{}%", (progress * 100.) as u32), [b[2] - 36., bar_y + 4., b[2], b[3]], 7.5, false, f.ink, 2);
        }
    }
}

fn draw_modular(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    b: [f32; 4],
    date_str: &str,
    seconds: &str,
) {
    let f = Face {
        ink: super::rgb(8, 13, 25),
        muted: super::rgb(24, 16, 12),
        ..*f
    };
    let bw = b[2] - b[0];
    let bh = b[3] - b[1];
    let gap = 8.;
    let weekday = date_str.split(',').next().unwrap_or("");
    let day_num = t.wDay.to_string();
    let draw_blue = |r: [f32; 4]| {
        let rw = r[2] - r[0];
        let rh = r[3] - r[1];
        let rad = (rw.min(rh) * 0.18).min(23.);
        f.round(r, rad, super::rgb(56, 110, 245), 255);
        let pad = (rw * 0.06).clamp(6., 12.);
        let sec_h = if cfg.show_seconds && rh >= 56. { 16. } else { 0. };
        let tr = [r[0] + pad, r[1] + 4., r[2] - pad, r[3] - sec_h - 4.];
        let sz = f.fit_digital(t, cfg, tr, 68.);
        f.digital_in(t, cfg, tr, sz, 1);
        if sec_h > 0. {
            f.text(&seconds.to_uppercase(), [r[0] + pad, r[3] - 18., r[2] - pad, r[3] - 4.], 8.5, true, f.muted, 1);
        }
    };
    let draw_orange = |r: [f32; 4]| {
        let rw = r[2] - r[0];
        let rh = r[3] - r[1];
        let rad = (rw.min(rh) * 0.18).min(23.);
        f.round(r, rad, super::rgb(255, 146, 38), 255);
        let wk_sz = (rh * 0.22).clamp(11., 24.);
        let day_sz = (rh * 0.50).min(rw * 0.65).clamp(20., 58.);
        f.text(weekday, [r[0] + 4., r[1] + rh * 0.10, r[2] - 4., r[1] + rh * 0.36], wk_sz, false, f.ink, 1);
        f.text(&day_num, [r[0] + 4., r[1] + rh * 0.34, r[2] - 4., r[3] - 4.], day_sz, false, f.ink, 1);
    };
    let draw_red = |cx: f32, cy: f32, d: f32| {
        f.round([cx - d / 2., cy - d / 2., cx + d / 2., cy + d / 2.], d / 2., super::rgb(255, 67, 44), 255);
        f.dial(t, cfg, cx, cy, d * 0.425);
    };
    match mode {
        layout::Mode::Wide => {
            let d = bh.min(bw * 0.28);
            let ow = (bw * 0.22).min(bh * 0.88).clamp(54., 110.);
            let bx1 = (b[2] - d - gap - ow - gap).max(b[0] + 60.);
            draw_blue([b[0], b[1], bx1, b[3]]);
            draw_orange([bx1 + gap, b[1], b[2] - d - gap, b[3]]);
            draw_red(b[2] - d / 2., (b[1] + b[3]) / 2., d);
        }
        layout::Mode::Tall => {
            let blue_h = (bh * 0.34).max(48.);
            let mid_h = (bh * 0.28).max(44.);
            draw_blue([b[0], b[1], b[2], b[1] + blue_h]);
            draw_orange([b[0], b[1] + blue_h + gap, b[2], b[1] + blue_h + gap + mid_h]);
            let rem_top = b[1] + blue_h + gap + mid_h + gap;
            let d = bw.min(b[3] - rem_top).max(36.);
            draw_red((b[0] + b[2]) / 2., (rem_top + b[3]) / 2., d);
        }
        _ => {
            let top_h = (bh * 0.45).max(38.);
            draw_blue([b[0], b[1], b[2], b[1] + top_h]);
            let bot_top = b[1] + top_h + gap;
            let bot_h = (b[3] - bot_top).max(36.);
            let half_w = (bw - gap) / 2.;
            let d = half_w.min(bot_h);
            draw_orange([b[0], bot_top, b[0] + half_w, b[3]]);
            draw_red(b[0] + half_w + gap + half_w / 2., (bot_top + b[3]) / 2., d);
        }
    }
}

fn draw_paper(
    f: &Face,
    cfg: &super::Config,
    t: &SYSTEMTIME,
    mode: layout::Mode,
    b: [f32; 4],
    date_str: &str,
) {
    let bw = b[2] - b[0];
    let bh = b[3] - b[1];
    match mode {
        layout::Mode::Wide => {
            let dial_r = (bh * 0.45).min(bw * 0.25).max(24.);
            let cx = b[0] + dial_r + 6.;
            let cy = (b[1] + b[3]) / 2.;
            f.dial(t, cfg, cx, cy, dial_r);
            let rx0 = cx + dial_r + 18.;
            let time_r = [rx0, b[1] + bh * 0.10, b[2], b[3] - 24.];
            let sz = f.fit_digital(t, cfg, time_r, 72.);
            f.digital_in(t, cfg, time_r, sz, 0);
            f.text(date_str, [rx0, b[3] - 22., b[2], b[3]], 10.5, false, f.muted, 0);
        }
        layout::Mode::Tall => {
            let dial_r = (bw * 0.44).min(bh * 0.28).max(26.);
            let cx = (b[0] + b[2]) / 2.;
            let cy = b[1] + dial_r + 8.;
            f.dial(t, cfg, cx, cy, dial_r);
            let time_top = cy + dial_r + 16.;
            let time_r = [b[0], time_top, b[2], (b[3] - 24.).max(time_top + 28.)];
            let sz = f.fit_digital(t, cfg, time_r, 56.);
            f.digital_in(t, cfg, time_r, sz, 1);
            f.text(date_str, [b[0], b[3] - 20., b[2], b[3]], 10., false, f.muted, 1);
        }
        _ => {
            let dial_r = (bh * 0.30).min(bw * 0.36).max(22.);
            let cx = (b[0] + b[2]) / 2.;
            let cy = b[1] + dial_r + 2.;
            f.dial(t, cfg, cx, cy, dial_r);
            let time_top = cy + dial_r + 6.;
            let time_r = [b[0], time_top, b[2], (b[3] - 18.).max(time_top + 22.)];
            let sz = f.fit_digital(t, cfg, time_r, 40.);
            f.digital_in(t, cfg, time_r, sz, 1);
            f.text(date_str, [b[0], b[3] - 16., b[2], b[3]], 9., false, f.muted, 1);
        }
    }
}
