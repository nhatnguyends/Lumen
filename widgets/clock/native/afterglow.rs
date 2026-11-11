//! A continuous sun/crescent outline; no background-colored eraser/matte.
use super::model::Config;
use crate::render::Canvas;
use windows_sys::Win32::Foundation::SYSTEMTIME;
fn smooth(x: f32) -> f32 {
    let x = x.clamp(0., 1.);
    x * x * (3. - 2. * x)
}
pub(super) fn seconds(t: &SYSTEMTIME) -> f32 {
    t.wHour as f32 * 3600.
        + t.wMinute as f32 * 60.
        + t.wSecond as f32
        + t.wMilliseconds as f32 / 1000.
}
pub(super) fn night(t: &SYSTEMTIME, mode: &str) -> f32 {
    let s = seconds(t);
    if mode == "loop" {
        return 0.5 - 0.5 * (s * std::f32::consts::TAU / 24.).cos();
    }
    let h = s / 3600.;
    if h < 7. {
        1. - smooth((h - 5.5) / 1.5)
    } else {
        smooth((h - 17.5) / 1.5)
    }
}
pub(super) fn content_key(
    cfg: &Config,
    t: &SYSTEMTIME,
    w: f32,
    h: f32,
    dpi: f32,
    ink: u32,
    muted: u32,
) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        w,
        h,
        dpi,
        t.wYear,
        t.wMonth,
        t.wDay,
        t.wHour,
        t.wMinute,
        if cfg.show_seconds { t.wSecond } else { 0 },
        cfg.hour12,
        cfg.show_seconds,
        ink,
        muted
    )
}
pub(super) fn interval(cfg: &Config, visible: bool, moving: bool) -> Option<u32> {
    (visible && !moving && cfg.theme == "sunset" && cfg.dynamic_animation).then_some(40)
}
fn outline(n: f32) -> Vec<(f32, f32)> {
    let mut points = Vec::with_capacity(83);
    let a = 0.225f32.acos();
    let bottom = std::f32::consts::TAU - a;
    for i in 0..=48 {
        let t = i as f32 / 48.;
        let theta = a + (bottom - a) * t;
        points.push((theta.cos(), theta.sin()));
    }
    for i in 0..=32 {
        let t = i as f32 / 32.;
        let sun = bottom + 2. * a * t;
        let moon = std::f32::consts::PI + a - 2. * a * t;
        let x = sun.cos() * (1. - n) + (0.45 + moon.cos()) * n;
        let y = sun.sin() * (1. - n) + moon.sin() * n;
        points.push((x, y));
    }
    points
}
pub(super) fn draw(
    c: &Canvas,
    rect: [f32; 4],
    cfg: &Config,
    t: &SYSTEMTIME,
    ink: u32,
    accent: u32,
) {
    let n = night(
        t,
        if cfg.dynamic_animation {
            &cfg.animation_mode
        } else {
            "clock"
        },
    );
    let time = if cfg.dynamic_animation {
        seconds(t)
    } else {
        0.
    };
    let w = rect[2] - rect[0];
    let h = rect[3] - rect[1];
    let r = (w * 0.22).min(h * 0.36);
    let cx = (rect[0] + rect[2]) / 2.;
    let horizon = rect[3] - 2.;
    let cy = horizon - r * 1.1 * n + (time * std::f32::consts::TAU / 9.).sin() * r * 0.035;
    let tilt = (-0.28 * n + (time * std::f32::consts::TAU / 13.).sin() * 0.025).clamp(-0.35, 0.1);
    let point = |x: f32, y: f32| {
        (
            cx + (x * tilt.cos() - y * tilt.sin()) * r,
            cy + (x * tilt.sin() + y * tilt.cos()) * r,
        )
    };
    let points: Vec<_> = outline(n)
        .into_iter()
        .map(|(x, y)| {
            let (x, y) = point(x, y);
            (x, y.min(horizon))
        })
        .collect();
    c.line(&points, ink, 245, (w / 65.).clamp(1.35, 2.2));
    for i in 0..17 {
        let theta = i as f32 * std::f32::consts::TAU / 17. + time * std::f32::consts::TAU / 70.;
        let p = point(theta.cos() * 1.30, theta.sin() * 1.30);
        let q = point(theta.cos() * 1.56, theta.sin() * 1.56);
        if p.1 > horizon && q.1 > horizon {
            continue;
        }
        let p = (p.0, p.1.min(horizon));
        let q = (q.0, q.1.min(horizon));
        c.line(
            &[p, q],
            accent,
            ((1. - n) * 225.) as u8,
            (w / 80.).clamp(1., 1.6),
        );
    }
    for (i, (x, y)) in [(-1.65, -0.6), (1.48, -0.95), (1.7, 0.35)]
        .into_iter()
        .enumerate()
    {
        let (x, y) = point(x, y);
        let a = (n * (130. + 55. * (time * 1.2 + i as f32 * 1.9).sin())) as u8;
        let d = (r * 0.11).max(0.8);
        c.line(&[(x - d, y), (x + d, y)], ink, a, 0.8);
        c.line(&[(x, y - d), (x, y + d)], ink, a, 0.8);
    }
    c.line(
        &[(rect[0] + 2., horizon), (rect[2] - 2., horizon)],
        ink,
        215,
        1.5,
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    fn time(h: u16, m: u16) -> SYSTEMTIME {
        SYSTEMTIME {
            wHour: h,
            wMinute: m,
            ..unsafe { std::mem::zeroed() }
        }
    }
    #[test]
    fn old_configs_gain_motion_defaults_without_losing_legacy_options() {
        let mut c: Config =
            serde_json::from_str(r#"{"theme":"sunset","size":[160,160],"hour12":true}"#).unwrap();
        assert!(c.dynamic_animation);
        assert_eq!(c.animation_mode, "clock");
        assert_eq!(c.size, [160, 160]);
        assert!(c.hour12);
        c.animation_mode = "loop".into();
        c.dynamic_animation = false;
        let saved = serde_json::to_string(&c).unwrap();
        let next: Config = serde_json::from_str(&saved).unwrap();
        assert_eq!(next.animation_mode, "loop");
        assert!(!next.dynamic_animation);
        assert_eq!(next.size, c.size);
    }
    #[test]
    fn content_cache_ignores_animation_milliseconds_but_tracks_visible_options() {
        let mut cfg = Config {
            theme: "sunset".into(),
            ..Config::default()
        };
        let mut t = time(12, 34);
        t.wSecond = 10;
        let key = content_key(&cfg, &t, 380., 380., 1., 0xffffff, 0xdddddd);
        t.wMilliseconds = 800;
        assert_eq!(
            content_key(&cfg, &t, 380., 380., 1., 0xffffff, 0xdddddd),
            key
        );
        cfg.show_seconds = false;
        assert_ne!(
            content_key(&cfg, &t, 380., 380., 1., 0xffffff, 0xdddddd),
            key
        );
        let hidden = content_key(&cfg, &t, 380., 380., 1., 0xffffff, 0xdddddd);
        t.wSecond = 40;
        assert_eq!(
            content_key(&cfg, &t, 380., 380., 1., 0xffffff, 0xdddddd),
            hidden
        );
        t.wMinute += 1;
        assert_ne!(
            content_key(&cfg, &t, 380., 380., 1., 0xffffff, 0xdddddd),
            hidden
        );
    }
    #[test]
    fn daylight_is_continuous_across_dawn_dusk_and_midnight() {
        assert_eq!(night(&time(12, 0), "clock"), 0.);
        assert_eq!(night(&time(0, 0), "clock"), 1.);
        assert_eq!(night(&time(23, 59), "clock"), 1.);
        assert!((night(&time(18, 15), "clock") - 0.5).abs() < 0.001);
        for minute in 0..1439 {
            let a = night(&time(minute / 60, minute % 60), "clock");
            let b = night(&time((minute + 1) / 60, (minute + 1) % 60), "clock");
            assert!((a - b).abs() < 0.02);
        }
    }
    #[test]
    fn morph_paths_are_closed_finite_and_bounded() {
        for i in 0..=100 {
            let p = outline(i as f32 / 100.);
            assert!(p.iter().all(|(x, y)| x.is_finite()
                && y.is_finite()
                && x.abs() <= 1.001
                && y.abs() <= 1.001));
            let first = p.first().unwrap();
            let last = p.last().unwrap();
            assert!((first.0 - last.0).abs() < 0.001 && (first.1 - last.1).abs() < 0.001);
        }
    }
    #[test]
    fn animation_stops_for_hidden_moving_disabled_and_other_faces() {
        let mut c = Config {
            theme: "sunset".into(),
            ..Config::default()
        };
        assert_eq!(interval(&c, true, false), Some(40));
        assert_eq!(interval(&c, false, false), None);
        assert_eq!(interval(&c, true, true), None);
        c.dynamic_animation = false;
        assert_eq!(interval(&c, true, false), None);
        c.dynamic_animation = true;
        c.theme = "paper".into();
        assert_eq!(interval(&c, true, false), None);
    }
}
