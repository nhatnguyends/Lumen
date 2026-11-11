//! Small outline icons for note actions; geometry uses DIP and native AA.
use super::*;
pub fn pin(c: &Canvas, x: f32, y: f32, ink: u32, pinned: bool) {
    c.line(
        &[
            (x + 5., y + 1.),
            (x + 11., y + 1.),
            (x + 10., y + 6.),
            (x + 13., y + 9.),
            (x + 3., y + 9.),
            (x + 6., y + 6.),
            (x + 5., y + 1.),
        ],
        ink,
        if pinned { 250 } else { 150 },
        1.25,
    );
    c.line(
        &[(x + 8., y + 9.), (x + 8., y + 15.)],
        ink,
        if pinned { 250 } else { 150 },
        1.25,
    );
}
pub fn trash(c: &Canvas, x: f32, y: f32, ink: u32) {
    c.line(&[(x + 2., y + 4.), (x + 14., y + 4.)], ink, 150, 1.25);
    c.line(
        &[
            (x + 5., y + 4.),
            (x + 5., y + 1.5),
            (x + 11., y + 1.5),
            (x + 11., y + 4.),
        ],
        ink,
        150,
        1.25,
    );
    c.line(
        &[
            (x + 4., y + 5.),
            (x + 4.8, y + 14.),
            (x + 11.2, y + 14.),
            (x + 12., y + 5.),
        ],
        ink,
        150,
        1.25,
    );
    c.line(&[(x + 7., y + 6.5), (x + 7., y + 12.)], ink, 150, 1.);
    c.line(&[(x + 9., y + 6.5), (x + 9., y + 12.)], ink, 150, 1.);
}
pub fn folder(c: &Canvas, x: f32, y: f32, ink: u32) {
    c.line(
        &[
            (x + 1., y + 13.),
            (x + 1., y + 3.),
            (x + 6., y + 3.),
            (x + 8., y + 5.),
            (x + 15., y + 5.),
            (x + 15., y + 13.),
            (x + 1., y + 13.),
        ],
        ink,
        180,
        1.25,
    );
}
pub fn search(c: &Canvas, x: f32, y: f32, ink: u32) {
    c.round(
        [x + 1., y + 1., x + 11., y + 11.],
        5.,
        ink,
        150,
        true,
        false,
    );
    c.line(&[(x + 10., y + 10.), (x + 15., y + 15.)], ink, 150, 1.4);
}
pub fn bead(c: &Canvas, x: f32, y: f32, color: u32) {
    let r = [x, y, x + 16., y + 16.];
    c.soft_shadow(r, 8., 50., 3., 1.5);
    c.round(r, 8., color, 245, false, false);
    c.glow(
        [x + 1., y + 1., x + 10., y + 10.],
        4.5,
        [x + 4., y + 3.],
        0xffffff,
        190,
    );
    c.round(r, 8., 0xffffff, 100, true, false);
}
pub fn note_day(stamp: u64) -> String {
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ticks = stamp
        .saturating_mul(10_000)
        .saturating_add(116_444_736_000_000_000);
    let time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    unsafe {
        let mut utc: SYSTEMTIME = zeroed();
        let mut local: SYSTEMTIME = zeroed();
        if FileTimeToSystemTime(&time, &mut utc) == 0
            || SystemTimeToTzSpecificLocalTime(null(), &utc, &mut local) == 0
        {
            return "Saved".into();
        }
        ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][local.wDayOfWeek as usize % 7].into()
    }
}
