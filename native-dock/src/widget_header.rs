//! App-local header reveal/hover animation. No background repaint loop at rest.
use crate::{
    motion::{FrameClock, Tween},
    settings_ui::widget_header_hits,
};
use std::{cell::RefCell, collections::HashMap, mem::zeroed, ptr::null, time::Instant};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{
        Controls::WM_MOUSELEAVE,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::*,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
const SUBCLASS: usize = 0x4c484452;
const TIMER: usize = 0x4c4844;
const EASE: [f32; 4] = [0.25, 0.1, 0.25, 1.];
pub struct Visual {
    pub alpha: f32,
    pub hover: [f32; 2],
    pub rotation: [f32; 2],
}
struct Reveal {
    alpha: Tween,
    hover: [Tween; 2],
    rotation: [Tween; 2],
    inside: bool,
    hot: Option<usize>,
    child_poll: bool,
    clock: Option<FrameClock>,
    period: u32,
}
impl Default for Reveal {
    fn default() -> Self {
        Self {
            alpha: Tween::fixed(0.),
            hover: [Tween::fixed(0.); 2],
            rotation: [Tween::fixed(0.); 2],
            inside: false,
            hot: None,
            child_poll: false,
            clock: None,
            period: 0,
        }
    }
}
impl Reveal {
    fn target(&mut self, inside: bool, hover: Option<usize>, now: f64) {
        self.inside = inside;
        self.hot = hover;
        self.alpha.target(
            if inside { 1. } else { 0. },
            now,
            if inside { 180. } else { 240. },
            EASE,
        );
        for i in 0..2 {
            let hot = inside && hover == Some(i);
            self.hover[i].target(if hot { 1. } else { 0. }, now, 180., EASE);
            self.rotation[i].target(
                if hot {
                    if i == 0 {
                        60.
                    } else {
                        90.
                    }
                } else {
                    0.
                },
                now,
                600.,
                [0.2, 0.9, 0.2, 1.],
            );
        }
    }
    fn moving(&self, now: f64) -> bool {
        self.alpha.moving(now)
            || self
                .hover
                .iter()
                .chain(self.rotation.iter())
                .any(|t| t.moving(now))
    }
    fn visual(&self, now: f64) -> Visual {
        Visual {
            alpha: self.alpha.sample(now),
            hover: self.hover.map(|t| t.sample(now)),
            rotation: self.rotation.map(|t| t.sample(now)),
        }
    }
}
thread_local! {
    static START:Instant=Instant::now();
    static HEADERS:RefCell<HashMap<usize,Reveal>>=RefCell::new(HashMap::new());
}
fn now() -> f64 {
    START.with(|s| s.elapsed().as_secs_f64() * 1000.)
}
unsafe fn track_leave(hwnd: HWND, nonclient: bool) {
    let mut track = TRACKMOUSEEVENT {
        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_LEAVE | if nonclient { TME_NONCLIENT } else { 0 },
        hwndTrack: hwnd,
        dwHoverTime: 0,
    };
    TrackMouseEvent(&mut track);
}
fn leave_tracking_flags(msg: u32) -> Option<u32> {
    match msg {
        WM_MOUSEMOVE => Some(TME_LEAVE),
        WM_NCMOUSEMOVE => Some(TME_LEAVE | TME_NONCLIENT),
        _ => None,
    }
}
unsafe fn pointer(hwnd: HWND) -> (bool, Option<usize>, bool) {
    let mut screen = zeroed();
    GetCursorPos(&mut screen);
    let under = WindowFromPoint(screen);
    // A fused cluster has a transparent, disabled decoration surface. Windows
    // can report that sibling instead of the member under its optical fill.
    // Bound the pointer to this member below, then accept our own process's
    // surfaces/children while still excluding unrelated occluding apps.
    let mut under_pid = 0;
    let mut own_pid = 0;
    GetWindowThreadProcessId(under, &mut under_pid);
    GetWindowThreadProcessId(hwnd, &mut own_pid);
    let ours = under == hwnd || IsChild(hwnd, under) != 0 || own_pid != 0 && under_pid == own_pid;
    let mut p = screen;
    ScreenToClient(hwnd, &mut p);
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let inside = ours && p.x >= 0 && p.y >= 0 && p.x < r.right && p.y < r.bottom;
    let dpi = (GetDpiForWindow(hwnd) as f32 / 96.).max(1.);
    let hit = inside
        .then(|| {
            widget_header_hits(r.right as f32 / dpi)
                .iter()
                .position(|(_, r)| {
                    p.x as f32 / dpi >= r[0]
                        && p.x as f32 / dpi < r[2]
                        && p.y as f32 / dpi >= r[1]
                        && p.y as f32 / dpi < r[3]
                })
        })
        .flatten();
    (inside, hit, inside && under != hwnd)
}
unsafe fn message_pointer(hwnd: HWND, msg: u32, lp: LPARAM) -> (bool, Option<usize>, bool) {
    let mut p = POINT {
        x: lp as i16 as i32,
        y: (lp >> 16) as i16 as i32,
    };
    if matches!(msg, WM_NCMOUSEMOVE | WM_NCLBUTTONDOWN) {
        ScreenToClient(hwnd, &mut p);
    }
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    let inside = p.x >= 0 && p.y >= 0 && p.x < r.right && p.y < r.bottom;
    let dpi = (GetDpiForWindow(hwnd) as f32 / 96.).max(1.);
    let hit = if inside {
        widget_header_hits(r.right as f32 / dpi)
            .iter()
            .position(|(_, r)| {
                p.x as f32 / dpi >= r[0]
                    && p.x as f32 / dpi < r[2]
                    && p.y as f32 / dpi >= r[1]
                    && p.y as f32 / dpi < r[3]
            })
    } else {
        None
    };
    (inside, hit, false)
}
unsafe fn schedule(hwnd: HWND, s: &mut Reveal, time: f64) {
    if s.moving(time) {
        if s.clock.is_none() {
            s.clock = Some(FrameClock::start());
        }
        if s.period != 10 {
            SetTimer(hwnd, TIMER, 10, None);
            s.period = 10;
        }
    } else {
        s.clock = None;
        if s.child_poll {
            if s.period != 50 {
                SetTimer(hwnd, TIMER, 50, None);
                s.period = 50;
            }
        } else if s.period != 0 {
            KillTimer(hwnd, TIMER);
            s.period = 0;
        }
    }
}
pub unsafe fn visual(hwnd: HWND) -> Visual {
    if hwnd.is_null() || IsWindow(hwnd) == 0 {
        return Visual {
            alpha: 1.,
            hover: [0.; 2],
            rotation: [0.; 2],
        };
    }
    HEADERS.with(|headers| {
        let mut headers = headers.borrow_mut();
        let s = headers.entry(hwnd as usize).or_insert_with(|| {
            SetWindowSubclass(hwnd, Some(proc), SUBCLASS, 0);
            let (inside, hit, child) = pointer(hwnd);
            let mut s = Reveal::default();
            s.child_poll = child;
            s.target(inside, hit, now());
            if inside {
                track_leave(hwnd, false);
            }
            s
        });
        schedule(hwnd, s, now());
        s.visual(now())
    })
}
unsafe extern "system" fn proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        KillTimer(hwnd, TIMER);
        HEADERS.with(|h| h.borrow_mut().remove(&(hwnd as usize)));
        RemoveWindowSubclass(hwnd, Some(proc), SUBCLASS);
        return DefSubclassProc(hwnd, msg, wp, lp);
    }
    if msg == WM_SHOWWINDOW && wp == 0 {
        HEADERS.with(|h| {
            if let Some(s) = h.borrow_mut().get_mut(&(hwnd as usize)) {
                *s = Reveal::default();
            }
        });
        KillTimer(hwnd, TIMER);
    }
    if matches!(
        msg,
        WM_MOUSEMOVE
            | WM_NCMOUSEMOVE
            | WM_MOUSELEAVE
            | WM_NCMOUSELEAVE
            | WM_LBUTTONDOWN
            | WM_NCLBUTTONDOWN
            | WM_CAPTURECHANGED
            | WM_EXITSIZEMOVE
    ) || msg == WM_TIMER && wp == TIMER
    {
        let time = now();
        let (inside, hit, child) = if matches!(
            msg,
            WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_NCMOUSEMOVE | WM_NCLBUTTONDOWN
        ) {
            message_pointer(hwnd, msg, lp)
        } else if msg == WM_TIMER {
            let saved = HEADERS.with(|h| {
                h.borrow()
                    .get(&(hwnd as usize))
                    .map(|s| (s.inside, s.hot, s.child_poll))
            });
            match saved {
                Some((inside, hit, false)) => (inside, hit, false),
                _ => pointer(hwnd),
            }
        } else {
            pointer(hwnd)
        };
        HEADERS.with(|h| {
            if let Some(s) = h.borrow_mut().get_mut(&(hwnd as usize)) {
                let old = s.visual(time);
                let was_moving = s.moving(time);
                let final_tick = msg == WM_TIMER && s.period == 10;
                s.child_poll = child;
                s.target(inside, hit, time);
                // Invalidate only during a transition. Native children need a cheap
                // pointer check after parent leave, but no idle rasterization.
                if final_tick
                    || was_moving
                    || s.moving(time)
                    || (old.alpha - s.alpha.sample(time)).abs() > 0.001
                {
                    InvalidateRect(hwnd, null(), 0);
                }
                schedule(hwnd, s, time);
            }
        });
        if msg == WM_TIMER {
            return 0;
        }
        // Track the region that actually delivered the movement. Caption/edge
        // motion is non-client motion. Re-arming CLIENT leave from a leave or
        // timer event while over the caption immediately posts another leave,
        // creating a hot message loop that starves paint and button input.
        if inside {
            if let Some(flags) = leave_tracking_flags(msg) {
                track_leave(hwnd, flags & TME_NONCLIENT != 0);
            }
        }
    }
    // Keep the existing widget press/release actions. An invisible header must
    // not turn empty glass into an unexpected Settings/Hide button.
    if matches!(msg, WM_LBUTTONDOWN | WM_LBUTTONDBLCLK) {
        let (_, hit, _) = message_pointer(hwnd, msg, lp);
        if hit.is_some()
            && HEADERS.with(|h| {
                h.borrow()
                    .get(&(hwnd as usize))
                    .is_some_and(|s| s.alpha.sample(now()) < 0.05)
            })
        {
            return 0;
        }
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caption_tracking_is_nonclient_and_leave_never_rearms_itself() {
        assert_eq!(leave_tracking_flags(WM_MOUSEMOVE), Some(TME_LEAVE));
        assert_eq!(
            leave_tracking_flags(WM_NCMOUSEMOVE),
            Some(TME_LEAVE | TME_NONCLIENT)
        );
        for msg in [
            WM_MOUSELEAVE,
            WM_NCMOUSELEAVE,
            WM_TIMER,
            WM_CAPTURECHANGED,
            WM_EXITSIZEMOVE,
            WM_NCLBUTTONDOWN,
        ] {
            assert_eq!(
                leave_tracking_flags(msg),
                None,
                "must not restart leave from {msg}"
            );
        }
    }
    #[test]
    fn reveal_retargets_continuously_and_settles_without_idle_animation() {
        let mut s = Reveal::default();
        assert_eq!(s.visual(0.).alpha, 0.);
        s.target(true, Some(0), 0.);
        let mid = s.visual(80.);
        assert!(mid.alpha > 0. && mid.alpha < 1.);
        s.target(false, None, 80.);
        assert!((s.visual(80.).alpha - mid.alpha).abs() < 0.0001);
        assert_eq!(s.visual(800.).alpha, 0.);
        assert!(!s.moving(800.));
        s.target(true, Some(1), 900.);
        assert_eq!(s.visual(1600.).alpha, 1.);
        assert_eq!(s.visual(1600.).rotation[1], 90.);
        assert_eq!(s.visual(1600.).hover[1], 1.);
    }
    #[test]
    fn app_local_hook_and_animation_timer_are_removed_with_the_widget() {
        let _lock = crate::native_ui_test_lock();
        unsafe {
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                crate::model::wide("STATIC").as_ptr(),
                crate::model::wide("Lumen header test").as_ptr(),
                WS_POPUP,
                -20000,
                -20000,
                240,
                180,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            assert_eq!(visual(hwnd).alpha, 0.);
            SendMessageW(hwnd, WM_MOUSEMOVE, 0, (70isize << 16) | 20);
            HEADERS.with(|h| assert!(h.borrow().get(&(hwnd as usize)).unwrap().inside));
            visual(hwnd);
            HEADERS.with(|h| {
                assert!(
                    h.borrow().get(&(hwnd as usize)).unwrap().inside,
                    "painting must not override a delivered pointer event with a global hit test"
                )
            });
            HEADERS.with(|h| {
                let mut h = h.borrow_mut();
                let s = h.get_mut(&(hwnd as usize)).unwrap();
                s.target(true, Some(0), now());
                schedule(hwnd, s, now());
                assert_eq!(s.period, 10);
                assert!(s.clock.is_some());
            });
            SendMessageW(hwnd, WM_SHOWWINDOW, 0, 0);
            HEADERS.with(|h| {
                let h = h.borrow();
                let s = h.get(&(hwnd as usize)).unwrap();
                assert_eq!(s.period, 0);
                assert!(s.clock.is_none());
                assert_eq!(s.visual(now()).alpha, 0.);
            });
            DestroyWindow(hwnd);
            assert!(!HEADERS.with(|h| h.borrow().contains_key(&(hwnd as usize))));
        }
    }
}
