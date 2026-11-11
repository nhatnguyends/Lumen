//! Time-based motion copied from the original dock's CSS/FLIP timings.
use std::collections::HashMap;
pub type Point = [f32; 2];
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
impl Drop for FrameClock {
    fn drop(&mut self) {
        if self.0 {
            unsafe {
                timeEndPeriod(1);
            }
        }
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
        self.velocity = velocity;
        self.target = target;
        self.start = now;
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
            pointer: point,
        }
    }
}
#[derive(Clone, Copy)]
pub struct Visual {
    pub position: Point,
    pub hover: f32,
    pub scale: f32,
    pub lift: f32,
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
    pub launching: Option<(String, f64, f64)>,
    pub group: HashMap<String, Tween>,
    pub timer_running: bool,
    pub clock: Option<FrameClock>,
    pub toolbar: HashMap<usize, Tween>,
    pub toolbar_rotation: HashMap<usize, Tween>,
    pub search_focus: Tween,
    pub scrollbar: Tween,
}
impl Motion {
    pub fn toolbar_hover(&mut self, index: Option<usize>, now: f64) {
        for i in 0..5 {
            self.toolbar_rotation
                .entry(i)
                .or_insert_with(|| Tween::fixed(0.))
                .target(if index == Some(i) { 60. } else { 0. }, now, 600., LIFT);
            self.toolbar
                .entry(i)
                .or_insert_with(|| Tween::fixed(0.))
                .target(if index == Some(i) { 1. } else { 0. }, now, 180., EASE);
        }
    }
    pub fn sync(&mut self, layout: &[(String, Point)], now: f64, animate: bool) {
        for (id, p) in layout {
            let node = self
                .nodes
                .entry(id.clone())
                .or_insert_with(|| Node::new(*p));
            if animate {
                node.x.target(p[0], now, 260., MOVE);
                node.y.target(p[1], now, 260., MOVE);
            } else {
                node.x = Tween::fixed(p[0]);
                node.y = Tween::fixed(p[1]);
            }
        }
    }
    fn retarget(&mut self, now: f64) {
        for (id, node) in &mut self.nodes {
            let hovered = self.hovered.as_deref() == Some(id);
            let pressed = self.pressed.as_deref() == Some(id);
            node.hover
                .target(if hovered { 1. } else { 0. }, now, 200., EASE);
            node.scale.target(
                if pressed {
                    0.97
                } else if hovered {
                    1.055
                } else {
                    1.
                },
                now,
                300.,
                LIFT,
            );
            node.lift
                .target(if hovered && !pressed { -1.5 } else { 0. }, now, 300., LIFT);
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
                200.,
                EASE,
            );
        }
    }
    pub fn group_alpha(&self, id: &str, now: f64) -> f32 {
        self.group.get(id).map_or(0., |t| t.sample(now))
    }
    pub fn launch(&mut self, id: String, now: f64) {
        self.launching = Some((id, now, now + 280.));
    }
    pub fn launch_finished(&mut self, now: f64) {
        if let Some((_, _, end)) = &mut self.launching {
            *end = now + 280.;
        }
    }
    pub fn visual(&self, id: &str, fallback: Point, now: f64) -> Visual {
        let launch = self
            .launching
            .as_ref()
            .filter(|(key, _, _)| key == id)
            .map_or(0., |(_, start, end)| {
                if now < *end {
                    ((now - start) / 60.).clamp(0., 1.) as f32
                } else {
                    (1. - (now - end) / 200.).clamp(0., 1.) as f32
                }
            });
        if let Some(n) = self.nodes.get(id) {
            Visual {
                position: [n.x.sample(now), n.y.sample(now)],
                hover: n.hover.sample(now),
                scale: n.scale.sample(now),
                lift: n.lift.sample(now),
                pointer: n.pointer,
                launch,
            }
        } else {
            Visual {
                position: fallback,
                hover: 0.,
                scale: 1.,
                lift: 0.,
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
            .is_some_and(|(_, _, end)| now >= end + 200.)
        {
            self.launching = None;
        }
        self.landing.is_some()
            || self.launching.is_some()
            || self.nodes.values().any(|n| {
                [n.x, n.y, n.hover, n.scale, n.lift]
                    .iter()
                    .any(|t| t.moving(now))
            })
            || self.group.values().any(|t| t.moving(now))
            || self.toolbar.values().any(|t| t.moving(now))
            || self.toolbar_rotation.values().any(|t| t.moving(now))
            || self.search_focus.moving(now)
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
}
