//! Time-based motion copied from the original dock's CSS/FLIP timings.
use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, Ordering},
};
pub type Point = [f32; 2];
static LOW_POWER: AtomicBool = AtomicBool::new(false);
pub fn set_low_power(enabled: bool) {
    LOW_POWER.store(enabled, Ordering::Relaxed);
}
pub fn low_power() -> bool {
    LOW_POWER.load(Ordering::Relaxed)
}
#[link(name = "winmm")]
extern "system" {
    fn timeBeginPeriod(period: u32) -> u32;
    fn timeEndPeriod(period: u32) -> u32;
}
// Request precise pacing only while an animation is active, never while idle.
pub struct FrameClock(bool);
impl FrameClock {
    pub fn start() -> Self {
        Self(unsafe { timeBeginPeriod(1) == 0 })
    }
}
pub fn frame_interval_ms() -> u32 {
    if low_power() { 16 } else { 10 }
}
impl Drop for FrameClock {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                timeEndPeriod(1);
            }
        }
    }
}
pub const VSYNC_FRAME: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 19;
struct FramePacing {
    period: std::time::Duration,
    checked: Option<std::time::Instant>,
    avoid_flush_until: Option<std::time::Instant>,
    timer: windows_sys::Win32::Foundation::HANDLE,
}
impl FramePacing {
    fn new() -> Self {
        Self {
            period: std::time::Duration::from_nanos(8_333_333),
            checked: None,
            avoid_flush_until: None,
            timer: std::ptr::null_mut(),
        }
    }
    fn budget(&self, low: bool) -> std::time::Duration {
        if low { self.period.max(std::time::Duration::from_nanos(16_666_667)) } else { self.period }
    }
    fn can_flush(&self, cost_ms: f64, now: std::time::Instant, low: bool) -> bool {
        cost_ms < (if low { 4.0 } else { 5.5 })
            && self.avoid_flush_until.is_none_or(|until| now >= until)
    }
    fn record_flush(&mut self, elapsed: std::time::Duration, now: std::time::Instant, low: bool) -> bool {
        if elapsed > self.budget(low).mul_f64(1.35) {
            self.avoid_flush_until = Some(now + std::time::Duration::from_secs(1));
            true
        } else {
            false
        }
    }
    unsafe fn refresh(&mut self, now: std::time::Instant) {
        if self.checked.is_some_and(|last| now.duration_since(last).as_secs_f32() < 2.) { return; }
        self.checked = Some(now);
        use windows_sys::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};
        let mut info: DWM_TIMING_INFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<DWM_TIMING_INFO>() as u32;
        if DwmGetCompositionTimingInfo(std::ptr::null_mut(), &mut info) >= 0 {
            let ratio = info.rateRefresh;
            let hz = ratio.uiNumerator as f64 / (ratio.uiDenominator as f64).max(1.);
            if (30.0..=500.0).contains(&hz) {
                self.period = std::time::Duration::from_secs_f64(1. / hz);
            }
        }
    }
    unsafe fn arm_timer(
        &mut self,
        remaining: std::time::Duration,
    ) -> Option<windows_sys::Win32::Foundation::HANDLE> {
        use windows_sys::Win32::System::Threading::*;
        if self.timer.is_null() {
            self.timer = CreateWaitableTimerExW(
                std::ptr::null(),
                std::ptr::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
            );
        }
        if !self.timer.is_null() {
            let due = -((remaining.as_nanos() / 100).max(1).min(i64::MAX as u128) as i64);
            if SetWaitableTimer(self.timer, &due, 0, None, std::ptr::null(), 0) != 0 {
                return Some(self.timer);
            }
        }
        None
    }
    unsafe fn wait_until(&mut self, started: std::time::Instant, low: bool) {
        let remaining = self.budget(low).saturating_sub(started.elapsed());
        if remaining < std::time::Duration::from_micros(100) {
            return;
        }
        if let Some(timer) = self.arm_timer(remaining) {
            windows_sys::Win32::System::Threading::WaitForSingleObject(
                timer,
                windows_sys::Win32::System::Threading::INFINITE,
            );
            return;
        }
        std::thread::sleep(remaining);
    }
}
impl Drop for FramePacing {
    fn drop(&mut self) {
        if !self.timer.is_null() { unsafe { windows_sys::Win32::Foundation::CloseHandle(self.timer); } }
    }
}
struct ScheduledVsync {
    deadline: std::time::Instant,
    queued: Vec<(usize, u32)>,
}
thread_local! {
    static PENDING_VSYNC: std::cell::RefCell<Vec<(usize, u32)>> = const { std::cell::RefCell::new(Vec::new()) };
    static PENDING_VSYNC_START: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    static SCHEDULED_VSYNC: std::cell::RefCell<Option<ScheduledVsync>> = const { std::cell::RefCell::new(None) };
    static FRAME_PACING: std::cell::RefCell<FramePacing> = std::cell::RefCell::new(FramePacing::new());
}
unsafe fn post_vsync_batch(queued: Vec<(usize, u32)>) {
    for (hwnd, message) in queued {
        windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(
            hwnd as windows_sys::Win32::Foundation::HWND,
            message,
            0,
            0,
        );
    }
}
pub unsafe fn flush_pending_vsync(frame_started: std::time::Instant) {
    let mut queued = PENDING_VSYNC.with(|q| {
        let mut q = q.borrow_mut();
        if q.is_empty() {
            Vec::new()
        } else {
            std::mem::take(&mut *q)
        }
    });
    if let Some(sched) = SCHEDULED_VSYNC.with(|s| s.borrow_mut().take()) {
        for entry in sched.queued {
            if !queued.contains(&entry) {
                queued.push(entry);
            }
        }
    }
    if queued.is_empty() {
        return;
    }
    let started = PENDING_VSYNC_START
        .with(|c| c.take())
        .unwrap_or(frame_started);
    let now = std::time::Instant::now();
    let present_ms = now.saturating_duration_since(started).as_secs_f64() * 1000.;
    let low = low_power();
    let use_dwm = !low
        && FRAME_PACING.with(|p| {
            let mut p = p.borrow_mut();
            p.refresh(now);
            p.can_flush(present_ms, now, false)
        });
    if use_dwm {
        let hr = windows_sys::Win32::Graphics::Dwm::DwmFlush();
        let done = std::time::Instant::now();
        FRAME_PACING.with(|p| {
            let mut p = p.borrow_mut();
            // A successful DwmFlush can still wait TWO refreshes for a layered
            // window. Detect that measured miss instead of halving 120Hz scroll.
            if hr < 0 || p.record_flush(done.saturating_duration_since(started), done, false) {
                p.avoid_flush_until = Some(done + std::time::Duration::from_secs(1));
            }
        });
    } else {
        FRAME_PACING.with(|p| p.borrow_mut().wait_until(started, low));
    }
    post_vsync_batch(queued);
}
pub unsafe fn step_pending_vsync(
    frame_started: std::time::Instant,
) -> Option<windows_sys::Win32::Foundation::HANDLE> {
    let queued = PENDING_VSYNC.with(|q| {
        let mut q = q.borrow_mut();
        if q.is_empty() {
            Vec::new()
        } else {
            std::mem::take(&mut *q)
        }
    });
    let started = PENDING_VSYNC_START
        .with(|c| c.take())
        .unwrap_or(frame_started);
    if let Some(mut sched) = SCHEDULED_VSYNC.with(|s| s.borrow_mut().take()) {
        for entry in queued {
            if !sched.queued.contains(&entry) {
                sched.queued.push(entry);
            }
        }
        let now = std::time::Instant::now();
        if now + std::time::Duration::from_micros(250) >= sched.deadline {
            post_vsync_batch(sched.queued);
            return None;
        }
        let remaining = sched.deadline.saturating_duration_since(now);
        if let Some(handle) = FRAME_PACING.with(|p| p.borrow_mut().arm_timer(remaining)) {
            SCHEDULED_VSYNC.with(|s| *s.borrow_mut() = Some(sched));
            return Some(handle);
        }
        std::thread::sleep(remaining);
        post_vsync_batch(sched.queued);
        return None;
    }
    if queued.is_empty() {
        return None;
    }
    let low = low_power();
    let now = std::time::Instant::now();
    let present_ms = now.saturating_duration_since(started).as_secs_f64() * 1000.;
    let (use_dwm, budget) = FRAME_PACING.with(|p| {
        let mut p = p.borrow_mut();
        p.refresh(now);
        (!low && p.can_flush(present_ms, now, false), p.budget(low))
    });
    if use_dwm {
        let hr = windows_sys::Win32::Graphics::Dwm::DwmFlush();
        let done = std::time::Instant::now();
        FRAME_PACING.with(|p| {
            let mut p = p.borrow_mut();
            if hr < 0 || p.record_flush(done.saturating_duration_since(started), done, false) {
                p.avoid_flush_until = Some(done + std::time::Duration::from_secs(1));
            }
        });
        post_vsync_batch(queued);
        return None;
    }
    let deadline = started + budget;
    if now + std::time::Duration::from_micros(250) >= deadline {
        post_vsync_batch(queued);
        return None;
    }
    if let Some(handle) = FRAME_PACING
        .with(|p| p.borrow_mut().arm_timer(deadline.saturating_duration_since(now)))
    {
        SCHEDULED_VSYNC.with(|s| {
            *s.borrow_mut() = Some(ScheduledVsync { deadline, queued });
        });
        return Some(handle);
    }
    FRAME_PACING.with(|p| p.borrow_mut().wait_until(started, low));
    post_vsync_batch(queued);
    None
}
pub fn clear_pending_vsync() {
    PENDING_VSYNC.with(|q| q.borrow_mut().clear());
    PENDING_VSYNC_START.with(|c| c.set(None));
    SCHEDULED_VSYNC.with(|s| *s.borrow_mut() = None);
}
pub unsafe fn pace_vsync(
    hwnd: windows_sys::Win32::Foundation::HWND,
    frame_started: std::time::Instant,
    frame_queued: &mut bool,
    message: u32,
) {
    if *frame_queued {
        return;
    }
    *frame_queued = true;
    if PENDING_VSYNC_START.with(|c| c.get().is_none()) {
        PENDING_VSYNC_START.with(|c| c.set(Some(frame_started)));
    }
    PENDING_VSYNC.with(|q| {
        let mut q = q.borrow_mut();
        let entry = (hwnd as usize, message);
        if !q.contains(&entry) {
            q.push(entry);
        }
    });
    // The host batches every widget's tick and paint before one shared wait.
    // Waiting here normally makes independently animated HWNDs wait separate
    // refreshes. Native move/menu loops do not run the host pump, so retain a
    // local flush there to keep the other widgets alive during a modal loop.
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let mut gui: GUITHREADINFO = std::mem::zeroed();
    gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if GetGUIThreadInfo(windows_sys::Win32::System::Threading::GetCurrentThreadId(), &mut gui) != 0
        && gui.flags & (GUI_INMOVESIZE | GUI_INMENUMODE) != 0
    {
        flush_pending_vsync(frame_started);
    }
}
pub fn bezier(t: f32, curve: [f32; 4]) -> f32 {
    let t = t.clamp(0., 1.);
    let [x1, y1, x2, y2] = curve;
    let component = |s: f32, a: f32, b: f32| {
        3. * (1. - s).powi(2) * s * a + 3. * (1. - s) * s * s * b + s * s * s
    };
    let (mut low, mut high) = (0., 1.);
    for _ in 0..18 {
        let middle = (low + high) * 0.5;
        if component(middle, x1, x2) < t {
            low = middle;
        } else {
            high = middle;
        }
    }
    if t == 0. {
        0.
    } else if t == 1. {
        1.
    } else {
        component((low + high) * 0.5, y1, y2)
    }
}
const EASE: [f32; 4] = [0.25, 0.1, 0.25, 1.];
const HOVER_FADE: [f32; 4] = [0.37, 0., 0.63, 1.];
const LIFT: [f32; 4] = [0.2, 0.9, 0.2, 1.];
const MOVE: [f32; 4] = [0.22, 0.9, 0.24, 1.];
#[derive(Clone, Copy)]
pub struct Tween {
    from: f32,
    to: f32,
    start: f64,
    ms: f64,
    curve: [f32; 4],
}
impl Default for Tween {
    fn default() -> Self {
        Self::fixed(0.)
    }
}
impl Tween {
    pub fn fixed(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            start: 0.,
            ms: 0.,
            curve: EASE,
        }
    }
    pub fn sample(&self, now: f64) -> f32 {
        if self.ms == 0. {
            return self.to;
        }
        self.from
            + (self.to - self.from) * bezier(((now - self.start) / self.ms) as f32, self.curve)
    }
    pub fn target(&mut self, to: f32, now: f64, ms: f64, curve: [f32; 4]) {
        if (self.to - to).abs() < 0.0001 {
            return;
        }
        *self = Self {
            from: self.sample(now),
            to,
            start: now,
            ms,
            curve,
        };
    }
    pub fn moving(&self, now: f64) -> bool {
        (self.to - self.from).abs() > 0.0001 && now < self.start + self.ms
    }
}
pub struct SmoothScroll {
    from: f32,
    velocity: f32,
    target: f32,
    start: f64,
    pub max: f32,
}
impl Default for SmoothScroll {
    fn default() -> Self {
        Self {
            from: 0.,
            velocity: 0.,
            target: 0.,
            start: 0.,
            max: 0.,
        }
    }
}
impl SmoothScroll {
    fn state(&self, now: f64) -> (f32, f32) {
        let t = ((now - self.start) / 1000.).max(0.) as f32;
        let omega = 22.;
        let displacement = self.from - self.target;
        let c = self.velocity + omega * displacement;
        let decay = (-omega * t).exp();
        let value = self.target + (displacement + c * t) * decay;
        let velocity = (self.velocity - omega * c * t) * decay;
        if (value - self.target).abs() < 0.025 && velocity.abs() < 0.5 {
            (self.target, 0.)
        } else {
            (value, velocity)
        }
    }
    pub fn value(&self, now: f64) -> f32 {
        self.state(now).0.clamp(0., self.max)
    }
    pub fn target(&self) -> f32 {
        self.target
    }
    pub fn snap(&mut self, value: f32) {
        self.from = value.clamp(0., self.max);
        self.target = self.from;
        self.velocity = 0.;
    }
    pub fn bounds(&mut self, max: f32, now: f64) {
        self.max = max.max(0.);
        if self.target > self.max {
            self.snap(self.value(now));
        }
    }
    pub fn to(&mut self, value: f32, now: f64) {
        let target = value.clamp(0., self.max);
        if (target - self.target).abs() < 0.001 {
            return;
        }
        let (position, velocity) = self.state(now);
        self.from = position;
        let delta = target - position;
        if velocity.abs() < 80. || velocity * delta <= 0. {
            self.velocity = (delta * 10.).clamp(-3600., 3600.)
                + if velocity * delta > 0. {
                    velocity * 0.5
                } else {
                    0.
                };
            self.start = now - 8.;
        } else {
            self.velocity = velocity;
            self.start = now;
        }
        self.target = target;
    }
    pub fn by(&mut self, delta: f32, now: f64) {
        self.to(self.target + delta, now);
    }
    pub fn moving(&self, now: f64) -> bool {
        let (p, v) = self.state(now);
        (p - self.target).abs() >= 0.025 || v.abs() >= 0.5
    }
}
pub struct Node {
    pub x: Tween,
    pub y: Tween,
    pub hover: Tween,
    pub scale: Tween,
    pub lift: Tween,
    pub alpha: Tween,
    pub pointer: Point,
}
impl Node {
    fn new(point: Point) -> Self {
        Self {
            x: Tween::fixed(point[0]),
            y: Tween::fixed(point[1]),
            hover: Tween::fixed(0.),
            scale: Tween::fixed(1.),
            lift: Tween::fixed(0.),
            alpha: Tween::fixed(1.),
            pointer: point,
        }
    }
}
pub const LAUNCH_DURATION_MS: f64 = 420.;
pub const LAUNCH_MAX_SCALE: f32 = 2.0; // 200%
pub const LAUNCH_SCALE_CURVE: [f32; 4] = [0.16, 1.0, 0.3, 1.0];
pub const LAUNCH_FADE_CURVE: [f32; 4] = [0.4, 0.0, 0.2, 1.0];

#[derive(Clone, Copy, Debug)]
pub struct LaunchFrame {
    pub progress: f32,
    pub scale: f32,
    pub alpha: f32,
}

#[derive(Clone, Copy)]
pub struct Visual {
    pub position: Point,
    pub hover: f32,
    pub scale: f32,
    pub lift: f32,
    pub alpha: f32,
    pub pointer: Point,
    pub launch: f32,
}
pub struct Landing {
    pub id: String,
    pub from: Point,
    pub to: Point,
    pub start: f64,
}
impl Landing {
    pub fn point(&self, now: f64) -> Point {
        let t = bezier(((now - self.start) / 260.) as f32, MOVE);
        [
            self.from[0] + (self.to[0] - self.from[0]) * t,
            self.from[1] + (self.to[1] - self.from[1]) * t,
        ]
    }
}
#[derive(Default)]
pub struct Motion {
    pub nodes: HashMap<String, Node>,
    pub hovered: Option<String>,
    pub pressed: Option<String>,
    pub landing: Option<Landing>,
    pub launching: Option<(String, f64)>,
    pub group: HashMap<String, Tween>,
    pub timer_running: bool,
    pub clock: Option<FrameClock>,
    pub toolbar: HashMap<usize, Tween>,
    pub toolbar_rotation: HashMap<usize, Tween>,
    pub toolbar_press: HashMap<usize, Tween>,
    pub search_focus: Tween,
    pub search_clear: Tween,
    pub status_alpha: Tween,
    pub scrollbar: Tween,
}
impl Motion {
    pub fn search_clear_hover(&mut self, hovered: bool, now: f64) {
        self.search_clear
            .target(if hovered { 1. } else { 0. }, now, 180., EASE);
    }
    pub fn status_visible(&mut self, visible: bool, now: f64) {
        self.status_alpha
            .target(if visible { 1. } else { 0. }, now, 180., EASE);
    }
    pub fn toolbar_hover(&mut self, index: Option<usize>, now: f64) {
        self.toolbar_hover_count(index, 5, now);
    }
    pub fn toolbar_hover_count(&mut self, index: Option<usize>, count: usize, now: f64) {
        for i in 0..count {
            self.toolbar_rotation
                .entry(i)
                .or_insert_with(|| Tween::fixed(0.))
                .target(if index == Some(i) { 60. } else { 0. }, now, 600., LIFT);
            self.toolbar
                .entry(i)
                .or_insert_with(|| Tween::fixed(0.))
                .target(
                    if index == Some(i) { 1. } else { 0. },
                    now,
                    if index == Some(i) { 240. } else { 300. },
                    HOVER_FADE,
                );
        }
    }
    pub fn toolbar_press(&mut self, index: Option<usize>, count: usize, now: f64) {
        for i in 0..count {
            self.toolbar_press
                .entry(i)
                .or_insert_with(|| Tween::fixed(1.))
                .target(if index == Some(i) { 0.88 } else { 1. }, now, 140., LIFT);
        }
    }
    pub fn sync(&mut self, layout: &[(String, Point)], now: f64, animate: bool) {
        let had_any = !self.nodes.is_empty();
        for (id, p) in layout {
            let is_new = !self.nodes.contains_key(id);
            let node = self
                .nodes
                .entry(id.clone())
                .or_insert_with(|| {
                    let mut n = Node::new(*p);
                    if animate && had_any {
                        n.alpha = Tween::fixed(0.);
                    }
                    n
                });
            if animate {
                node.x.target(p[0], now, 260., MOVE);
                node.y.target(p[1], now, 260., MOVE);
                if is_new && had_any {
                    node.alpha.target(1., now, 180., EASE);
                } else {
                    node.alpha.target(1., now, 180., EASE);
                }
            } else {
                node.x = Tween::fixed(p[0]);
                node.y = Tween::fixed(p[1]);
                node.alpha = Tween::fixed(1.);
            }
        }
    }
    fn retarget(&mut self, now: f64) {
        for (id, node) in &mut self.nodes {
            let hovered = self.hovered.as_deref() == Some(id);
            let pressed = self.pressed.as_deref() == Some(id);
            node.hover.target(
                if hovered { 1. } else { 0. },
                now,
                if hovered { 90. } else { 180. },
                if hovered { LIFT } else { HOVER_FADE },
            );
            node.scale.target(
                if pressed {
                    0.97
                } else if hovered {
                    1.055
                } else {
                    1.
                },
                now,
                if pressed {
                    100.
                } else if hovered {
                    160.
                } else {
                    200.
                },
                LIFT,
            );
            node.lift.target(
                if hovered && !pressed { -1.5 } else { 0. },
                now,
                if hovered && !pressed { 160. } else { 200. },
                LIFT,
            );
        }
    }
    pub fn hover(&mut self, id: Option<String>, pointer: Point, now: f64) {
        self.hovered = id;
        if let Some(node) = self.hovered.as_ref().and_then(|id| self.nodes.get_mut(id)) {
            node.pointer = pointer;
        }
        self.retarget(now);
    }
    pub fn press(&mut self, id: Option<String>, now: f64) {
        self.pressed = id;
        self.retarget(now);
    }
    pub fn group_target(&mut self, id: Option<&str>, now: f64) {
        if let Some(id) = id {
            self.group
                .entry(id.into())
                .or_insert_with(|| Tween::fixed(0.));
        }
        for (key, tween) in &mut self.group {
            tween.target(
                if id == Some(key.as_str()) { 1. } else { 0. },
                now,
                if id == Some(key.as_str()) { 240. } else { 300. },
                HOVER_FADE,
            );
        }
    }
    pub fn group_alpha(&self, id: &str, now: f64) -> f32 {
        self.group.get(id).map_or(0., |t| t.sample(now))
    }
    pub fn launch(&mut self, id: String, now: f64) {
        self.launching = Some((id, now));
    }
    pub fn launch_finished(&mut self, _now: f64) {}
    pub fn launch_frame(&self, now: f64) -> Option<(&str, LaunchFrame)> {
        let (id, start) = self.launching.as_ref()?;
        let elapsed = now - *start;
        if elapsed < 0. || elapsed >= LAUNCH_DURATION_MS {
            return None;
        }
        let progress = (elapsed / LAUNCH_DURATION_MS).clamp(0., 1.) as f32;
        let scale_t = bezier(progress, LAUNCH_SCALE_CURVE);
        let fade_t = bezier(progress, LAUNCH_FADE_CURVE);
        Some((
            id.as_str(),
            LaunchFrame {
                progress,
                scale: 1.0 + (LAUNCH_MAX_SCALE - 1.0) * scale_t,
                alpha: (1.0 - fade_t).clamp(0.0, 1.0),
            },
        ))
    }
    pub fn visual(&self, id: &str, fallback: Point, now: f64) -> Visual {
        let launch = self
            .launch_frame(now)
            .filter(|(key, _)| *key == id)
            .map_or(0., |(_, f)| f.alpha);
        if let Some(n) = self.nodes.get(id) {
            Visual {
                position: [n.x.sample(now), n.y.sample(now)],
                hover: n.hover.sample(now),
                scale: n.scale.sample(now),
                lift: n.lift.sample(now),
                alpha: n.alpha.sample(now),
                pointer: n.pointer,
                launch,
            }
        } else {
            Visual {
                position: fallback,
                hover: 0.,
                scale: 1.,
                lift: 0.,
                alpha: 1.,
                pointer: fallback,
                launch,
            }
        }
    }
    pub fn tick(&mut self, now: f64) -> bool {
        if self.landing.as_ref().is_some_and(|l| now >= l.start + 260.) {
            self.landing = None;
        }
        if self
            .launching
            .as_ref()
            .is_some_and(|(_, start)| now >= *start + LAUNCH_DURATION_MS)
        {
            self.launching = None;
        }
        self.landing.is_some()
            || self.launching.is_some()
            || self.nodes.values().any(|n| {
                [n.x, n.y, n.hover, n.scale, n.lift, n.alpha]
                    .iter()
                    .any(|t| t.moving(now))
            })
            || self.group.values().any(|t| t.moving(now))
            || self.toolbar.values().any(|t| t.moving(now))
            || self.toolbar_rotation.values().any(|t| t.moving(now))
            || self.toolbar_press.values().any(|t| t.moving(now))
            || self.search_focus.moving(now)
            || self.search_clear.moving(now)
            || self.status_alpha.moving(now)
            || self.scrollbar.moving(now)
    }
}
pub struct Drag {
    pub id: String,
    pub start: Point,
    pub pointer: Point,
    pub origin: Point,
    pub active: bool,
    pub preview: Vec<String>,
    pub target_group: Option<String>,
    pub scrolled: f64,
}
impl Drag {
    pub fn new(id: String, start: Point, origin: Point, preview: Vec<String>) -> Self {
        Self {
            id,
            start,
            pointer: start,
            origin,
            active: false,
            preview,
            target_group: None,
            scrolled: 0.,
        }
    }
    pub fn update(&mut self, p: Point) -> bool {
        self.pointer = p;
        if !self.active && (p[0] - self.start[0]).hypot(p[1] - self.start[1]) >= 4. {
            self.active = true;
        }
        self.active
    }
    pub fn point(&self) -> Point {
        [
            self.origin[0] + self.pointer[0] - self.start[0],
            self.origin[1] + self.pointer[1] - self.start[1],
        ]
    }
    pub fn reorder(&mut self, index: usize) -> bool {
        let Some(old) = self.preview.iter().position(|id| id == &self.id) else {
            return false;
        };
        let index = index.min(self.preview.len().saturating_sub(1));
        if old == index {
            return false;
        }
        let id = self.preview.remove(old);
        self.preview.insert(index, id);
        true
    }
}
pub fn nearest_slot(
    count: usize,
    columns: usize,
    cell: f32,
    row: f32,
    top: f32,
    scroll: f32,
    p: Point,
) -> Option<usize> {
    (0..count).min_by(|a, b| {
        let d = |i: usize| {
            let x = 16. + (i % columns) as f32 * cell + (cell - 6.) / 2.;
            let y = top + (i / columns) as f32 * row - scroll + (row - 4.) / 2.;
            (p[0] - x).powi(2) + (p[1] - y).powi(2)
        };
        d(*a).total_cmp(&d(*b))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pacing_bypasses_measured_double_refresh_waits_then_retries() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let mut pacing = FramePacing::new();
        assert!(pacing.can_flush(5., now, false));
        assert!(!pacing.can_flush(5.5, now, false));
        assert!(!pacing.record_flush(Duration::from_micros(8333), now, false));
        assert!(pacing.can_flush(5., now, false));
        assert!(pacing.record_flush(Duration::from_micros(16667), now, false));
        assert!(!pacing.can_flush(5., now + Duration::from_millis(999), false));
        assert!(pacing.can_flush(5., now + Duration::from_secs(1), false));
        assert!(!pacing.can_flush(4., now + Duration::from_secs(1), true));
        assert_eq!(pacing.budget(true), Duration::from_nanos(16_666_667));
        assert_eq!(pacing.budget(false).saturating_sub(Duration::from_millis(12)), Duration::ZERO);
    }
    #[test]
    fn toolbar_uses_original_rotation_and_returns_to_rest() {
        let mut motion = Motion::default();
        motion.toolbar_hover(Some(2), 0.);
        assert!(motion.tick(300.));
        assert_eq!(motion.toolbar_rotation[&2].sample(600.), 60.);
        assert_eq!(motion.toolbar_rotation[&1].sample(600.), 0.);
        assert!(!motion.tick(601.));
        motion.toolbar_hover(None, 700.);
        assert_eq!(motion.toolbar_rotation[&2].sample(1300.), 0.);
        assert!(!motion.tick(1301.));
    }
    #[test]
    fn scroll_retargets_without_jumping_and_clamps() {
        let mut s = SmoothScroll::default();
        s.bounds(250., 0.);
        s.by(72., 0.);
        let first = s.value(60.);
        assert!(first > 0. && first < 72.);
        let speed = s.state(60.).1;
        s.by(72., 60.);
        assert!((speed - s.state(60.).1).abs() < 0.001);
        assert!((first - s.value(60.)).abs() < 0.0001);
        assert_eq!(144., s.value(1000.));
        s.by(500., 1000.);
        assert_eq!(250., s.value(2000.));
        s.bounds(25., 2000.);
        assert_eq!(25., s.value(2000.));
    }
    #[test]
    fn drag_targets_follow_fractional_pixel_scrolling() {
        assert_eq!(
            nearest_slot(12, 4, 90., 90., 80., 45.5, [58., 165.]),
            Some(4)
        );
        assert_eq!(nearest_slot(12, 4, 90., 90., 80., 0., [58., 165.]), Some(0));
    }
    #[test]
    fn hover_and_press_use_old_dock_values_and_then_stop() {
        let mut m = Motion::default();
        m.sync(&[("a".into(), [16., 80.])], 0., false);
        m.hover(Some("a".into()), [30., 100.], 0.);
        assert!(m.tick(150.));
        let v = m.visual("a", [0.; 2], 300.);
        assert!((v.scale - 1.055).abs() < 0.0001);
        assert!((v.lift + 1.5).abs() < 0.0001);
        assert!(!m.tick(301.));
        m.press(Some("a".into()), 400.);
        assert!((m.visual("a", [0.; 2], 700.).scale - 0.97).abs() < 0.0001);
    }
    #[test]
    fn retargeting_never_jumps() {
        let mut t = Tween::fixed(0.);
        t.target(1., 0., 200., EASE);
        let before = t.sample(70.);
        t.target(0., 70., 200., EASE);
        assert!((before - t.sample(70.)).abs() < 0.0001);
        assert_eq!(t.sample(300.), 0.);
    }
    #[test]
    fn placeholder_prevents_stationary_pointer_reordering_back() {
        let mut d = Drag::new(
            "a".into(),
            [0.; 2],
            [0.; 2],
            vec!["a".into(), "b".into(), "c".into()],
        );
        assert!(!d.update([3.9, 0.]));
        assert!(d.update([4., 0.]));
        assert!(d.reorder(2));
        assert!(!d.reorder(2));
        assert_eq!(d.preview, vec!["b", "c", "a"]);
    }
    #[test]
    fn flipped_positions_settle_after_260_ms() {
        let mut m = Motion::default();
        m.sync(&[("a".into(), [16., 80.])], 0., false);
        m.sync(&[("a".into(), [106., 80.])], 10., true);
        assert_eq!(m.visual("a", [0.; 2], 10.).position, [16., 80.]);
        assert_eq!(m.visual("a", [0.; 2], 270.).position, [106., 80.]);
        assert!(!m.tick(271.));
    }
    #[test]
    fn bezier_has_exact_endpoints() {
        assert_eq!(bezier(0., LIFT), 0.);
        assert_eq!(bezier(1., LIFT), 1.);
        assert!(bezier(0.5, LIFT) > 0.8);
    }
    #[test]
    fn low_power_scroll_and_animation_stay_at_least_60hz() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                set_low_power(self.0);
            }
        }
        let _reset = Reset(low_power());
        set_low_power(true);
        assert!(frame_interval_ms() <= 16);
        set_low_power(false);
        assert_eq!(frame_interval_ms(), 10);
    }
    #[test]
    fn low_power_step_pending_vsync_targets_60hz_without_blocking() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                clear_pending_vsync();
                set_low_power(self.0);
                unsafe {
                    use windows_sys::Win32::UI::WindowsAndMessaging::*;
                    let mut msg: MSG = std::mem::zeroed();
                    while PeekMessageW(&mut msg, std::ptr::null_mut(), VSYNC_FRAME, VSYNC_FRAME, PM_REMOVE) != 0 {}
                }
            }
        }
        let _reset = Reset(low_power());
        clear_pending_vsync();
        set_low_power(true);

        let start = std::time::Instant::now();
        let mut queued = false;
        unsafe {
            pace_vsync(std::ptr::null_mut(), start, &mut queued, VSYNC_FRAME);
            assert!(queued);
            let handle = step_pending_vsync(start);
            assert!(handle.is_some());
            assert!(start.elapsed() < std::time::Duration::from_millis(12));

            let mut handle = step_pending_vsync(start);
            assert!(handle.is_some());

            for _ in 0..5 {
                let Some(h) = handle else {
                    break;
                };
                windows_sys::Win32::System::Threading::WaitForSingleObject(h, 100);
                handle = step_pending_vsync(start);
            }
            assert!(handle.is_none());
            assert!(start.elapsed() >= std::time::Duration::from_millis(15));
        }
    }
    #[test]
    fn launch_scales_to_two_x_and_fades_along_curves() {
        let mut m = Motion::default();
        m.launch("app".into(), 100.);

        let (_, start) = m.launch_frame(100.).expect("frame at start");
        assert!((start.scale - 1.0).abs() < 0.0001);
        assert!((start.alpha - 1.0).abs() < 0.0001);

        let mid_time = 100. + LAUNCH_DURATION_MS * 0.5;
        let (_, mid) = m.launch_frame(mid_time).expect("frame at midpoint");
        assert!(mid.scale > 1.5 && mid.scale < LAUNCH_MAX_SCALE);
        assert!(mid.alpha > 0.0 && mid.alpha < 1.0);
        assert!(m.tick(mid_time));

        let (_, near_end) = m
            .launch_frame(100. + LAUNCH_DURATION_MS - 0.1)
            .expect("frame just before completion");
        assert!(near_end.scale > 1.98 && near_end.scale <= LAUNCH_MAX_SCALE);
        assert!(near_end.alpha < 0.02 && near_end.alpha >= 0.0);

        let end_time = 100. + LAUNCH_DURATION_MS;
        assert!(m.launch_frame(end_time).is_none());
        assert!(!m.tick(end_time));
        assert!(m.launching.is_none());
    }
}
