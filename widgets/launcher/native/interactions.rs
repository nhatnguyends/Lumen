//! Pointer interactions are local to the dock HWND; no global input polling.
use super::*;
use crate::motion::{Drag, Landing, Point};
pub(super) const MOTION_TIMER: usize = 3;
pub(super) const LAUNCH_TIMER: usize = 4;
pub(super) const CARET_TIMER: usize = 5;
impl App {
    pub(super) unsafe fn adaptive_ink(
        &mut self,
        _dc: HDC,
        _rect: [f32; 4],
        _key: &str,
    ) -> (u32, u32) {
        (self.search_ink, self.search_muted)
    }
    pub(super) unsafe fn clear_hit(&self, hwnd: HWND, p: [f32; 2]) -> bool {
        self.search_visible() && !self.query.is_empty()
            && (p[0] - crate::geometry::dock_header(self.dimensions(hwnd).0, true).clear_x)
                .hypot(p[1] - 37.)
                < 12.
    }
    pub(super) unsafe fn clear_search(&mut self, hwnd: HWND) {
        SetWindowTextW(self.edit, wide("").as_ptr());
        self.query.clear();
        self.selected = 0;
        self.scroll.snap(0.);
        self.layout_edit(hwnd);
        SetFocus(self.edit);
        self.arm_motion(hwnd);
    }
    pub(super) unsafe fn widget_hover(&mut self, hwnd: HWND, inside: bool) {
        let target = if inside { 1. } else { 0. };
        if !self.motion.scrollbar.moving(self.now())
            && (self.motion.scrollbar.sample(self.now()) - target).abs() < 0.001
        {
            return;
        }
        self.motion.scrollbar.target(
            if inside { 1. } else { 0. },
            self.now(),
            200.,
            [0.25, 0.1, 0.25, 1.],
        );
        self.arm_motion(hwnd);
    }
    pub(super) unsafe fn check_widget_pointer(&mut self, hwnd: HWND) {
        let mut point: POINT = zeroed();
        GetCursorPos(&mut point);
        ScreenToClient(hwnd, &mut point);
        let (w, h) = self.dimensions(hwnd);
        let p = [point.x as f32 / self.dpi, point.y as f32 / self.dpi];
        self.widget_hover(
            hwnd,
            crate::compositor::distance(p[0], p[1], w as f32, h as f32, 34., 4.) <= 0.,
        );
    }
    pub(super) fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.
    }
    pub(super) unsafe fn ensure_motion_timer(&mut self, hwnd: HWND) {
        if !self.motion.timer_running {
            self.motion.clock = Some(crate::motion::FrameClock::start());
            // 16 ms rounds to two system ticks on some Windows configurations.
            // A 10 ms wake-up plus DWM presentation pacing avoids the 31 ms cadence.
            SetTimer(hwnd, MOTION_TIMER, 10, None);
            self.motion.timer_running = true;
        }
    }
    pub(super) unsafe fn arm_motion(&mut self, hwnd: HWND) {
        self.ensure_motion_timer(hwnd);
    }
    pub(super) unsafe fn hover_tile(&mut self, hwnd: HWND, p: Point) {
        let hover = self.hit(hwnd, p[0] as i32, p[1] as i32);
        if self.pointer == p && self.hover == hover {
            return;
        }
        self.pointer = p;
        self.hover = hover;
        self.motion
            .toolbar_hover(self.toolbar_hit(self.dimensions(hwnd).0, p), self.now());
        let visible = self.visible(hwnd);
        let id = self.hover.and_then(|pos| {
            visible
                .get(pos)
                .map(|i| self.dock.items[*i].id.clone())
                .or_else(|| (pos == visible.len()).then(|| "__add".into()))
        });
        self.motion.hover(id, p, self.now());
        self.arm_motion(hwnd);
    }
    pub(super) unsafe fn pointer_down(&mut self, hwnd: HWND, pos: usize, p: Point) {
        let visible = self.visible(hwnd);
        let Some(&index) = visible.get(pos) else {
            return;
        };
        let id = self.dock.items[index].id.clone();
        self.selected = pos;
        self.keyboard_selection = false;
        self.motion.hover(Some(id.clone()), p, self.now());
        self.motion.press(Some(id.clone()), self.now());
        let columns = self.columns(hwnd);
        let cell = (self.dimensions(hwnd).0 - 26) / columns as i32;
        let origin = [
            (16 + pos as i32 % columns as i32 * cell) as f32,
            (self.grid_top() + (pos / columns) as i32 * self.row_height()) as f32
                - self.scroll.value(self.now()),
        ];
        let order = visible
            .iter()
            .map(|i| self.dock.items[*i].id.clone())
            .collect();
        self.tile_drag = Some(Drag::new(id, p, origin, order));
        SetFocus(hwnd);
        SetCapture(hwnd);
        self.arm_motion(hwnd);
    }
    pub(super) unsafe fn group_at(&self, hwnd: HWND, p: Point, dragged: &str) -> Option<String> {
        let (w, _) = self.dimensions(hwnd);
        if p[0] < 23.
            || p[0] > w as f32 - 23.
            || p[1] < (self.grid_top() - 14) as f32
            || p[1] > (self.dimensions(hwnd).1 - 14) as f32
        {
            return None;
        }
        let visible = self.visible(hwnd);
        let columns = self.columns(hwnd);
        let cell = (w - 26) / columns as i32;
        let row = self.row_height();
        let mut bounds: HashMap<&str, [f32; 4]> = HashMap::new();
        for (pos, i) in visible.iter().enumerate() {
            let item = &self.dock.items[*i];
            if item.id == dragged {
                continue;
            }
            let Some(group) = item.group.as_deref() else {
                continue;
            };
            let x = (16 + (pos % columns) as i32 * cell) as f32;
            let y = (self.grid_top() + (pos / columns) as i32 * row) as f32
                - self.scroll.value(self.now());
            let r = [x, y, x + cell as f32 - 6., y + row as f32 - 4.];
            bounds
                .entry(group)
                .and_modify(|b| {
                    b[0] = b[0].min(r[0]);
                    b[1] = b[1].min(r[1]);
                    b[2] = b[2].max(r[2]);
                    b[3] = b[3].max(r[3]);
                })
                .or_insert(r);
        }
        bounds
            .into_iter()
            .find(|(_, r)| {
                p[0] >= r[0] - 3. && p[0] <= r[2] + 3. && p[1] >= r[1] - 3. && p[1] <= r[3] + 3.
            })
            .map(|(id, _)| id.into())
    }
    pub(super) unsafe fn pointer_move(&mut self, hwnd: HWND, p: Point) {
        let can_sort = self.query.trim().is_empty();
        if let Some(drag) = &mut self.tile_drag {
            if can_sort && drag.update(p) {
                let id = drag.id.clone();
                let own = self
                    .dock
                    .items
                    .iter()
                    .find(|i| i.id == id)
                    .and_then(|i| i.group.clone());
                self.motion.toolbar_hover(None, self.now());
                self.motion.hover(None, p, self.now());
                self.motion.press(None, self.now());
                let target = self.group_at(hwnd, p, &id);
                self.motion.group_target(
                    target.as_deref().filter(|g| Some(*g) != own.as_deref()),
                    self.now(),
                );
                let columns = self.columns(hwnd);
                let cell = (self.dimensions(hwnd).0 - 26) as f32 / columns as f32;
                let row = self.row_height() as f32;
                let top = self.grid_top() as f32;
                let scroll = self.scroll.value(self.now());
                if let Some(drag) = &mut self.tile_drag {
                    drag.target_group = target.clone();
                    if target.is_none() || target == own {
                        if let Some(index) = crate::motion::nearest_slot(
                            drag.preview.len(),
                            columns,
                            cell,
                            row,
                            top,
                            scroll,
                            p,
                        ) {
                            drag.reorder(index);
                        }
                    }
                }
                self.arm_motion(hwnd);
                return;
            }
        }
        self.hover_tile(hwnd, p);
    }
    pub(super) unsafe fn cancel_pointer(&mut self, hwnd: HWND) {
        self.clear_pressed = false;
        self.toolbar_pressed = None;
        self.add_pressed = false;
        self.bar_drag = None;
        if GetCapture() == hwnd {
            ReleaseCapture();
        }
        if self.tile_drag.take().is_some() {
            self.suppress_click_until = self.now() + 500.;
        }
        self.motion.press(None, self.now());
        self.motion.hover(None, [0.; 2], self.now());
        self.motion.group_target(None, self.now());
        self.hover = None;
        self.arm_motion(hwnd);
    }
    pub(super) unsafe fn pointer_up(&mut self, hwnd: HWND) {
        let Some(drag) = self.tile_drag.take() else {
            return;
        };
        ReleaseCapture();
        let now = self.now();
        self.motion.press(None, now);
        if drag.active {
            let target = drag.target_group.clone();
            if self
                .dock
                .commit_drag(&drag.preview, &drag.id, target.as_deref())
            {
                self.save();
                let visible = self.visible(hwnd);
                let columns = self.columns(hwnd);
                let cell = (self.dimensions(hwnd).0 - 26) / columns as i32;
                if let Some(pos) = visible
                    .iter()
                    .position(|i| self.dock.items[*i].id == drag.id)
                {
                    let to = [
                        (16 + (pos % columns) as i32 * cell) as f32,
                        (self.grid_top() + (pos / columns) as i32 * self.row_height()) as f32
                            - self.scroll.value(self.now()),
                    ];
                    self.motion.landing = Some(Landing {
                        id: drag.id.clone(),
                        from: drag.point(),
                        to,
                        start: now,
                    });
                }
            }
            self.suppress_click_until = now + 500.;
            self.motion.group_target(None, now);
            self.motion.hover(None, drag.pointer, now);
        } else if now >= self.suppress_click_until {
            if let Some(index) = self.dock.items.iter().position(|i| i.id == drag.id) {
                self.launch(hwnd, index);
            }
        }
        self.arm_motion(hwnd);
    }
    pub(super) unsafe fn launch(&mut self, hwnd: HWND, index: usize) {
        let now = self.now();
        let Some(item) = self.dock.items.get(index) else {
            return;
        };
        if now < self.suppress_click_until
            || now < self.open_busy_until
            || self.pending_launch.is_some()
            || self
                .last_open
                .as_ref()
                .is_some_and(|(id, at)| id == &item.id && now - *at < 700.)
        {
            return;
        }
        let id = item.id.clone();
        self.last_open = Some((id.clone(), now));
        self.open_busy_until = now + 280.;
        self.motion.launch(id.clone(), now);
        self.pending_launch = Some(id);
        SetTimer(hwnd, LAUNCH_TIMER, 32, None);
        self.arm_motion(hwnd);
    }
    pub(super) unsafe fn motion_tick(&mut self, hwnd: HWND) {
        let now = self.now();
        let resized = self.apply_resize();
        if let Some(drag) = &self.tile_drag {
            if drag.active && now - drag.scrolled > 70. {
                let p = drag.pointer;
                let (_, height) = self.dimensions(hwnd);
                let delta = if p[1] > (height - 35) as f32 {
                    24.
                } else if p[1] < (self.grid_top() + 20) as f32 {
                    -24.
                } else {
                    0.
                };
                self.scroll.by(delta, now);
                if let Some(d) = &mut self.tile_drag {
                    d.scrolled = now;
                }
                self.pointer_move(hwnd, p);
            }
        }
        if self.scroll.moving(now) && self.tile_drag.is_none() {
            self.hover_tile(hwnd, self.pointer);
        }
        let active = self.motion.tick(now)
            || self.scroll.moving(now)
            || self.tile_drag.as_ref().is_some_and(|d| d.active);
        let timer_active = active || self.resize_drag.is_some();
        if !timer_active {
            KillTimer(hwnd, MOTION_TIMER);
            self.motion.timer_running = false;
            self.motion.clock = None;
        }
        if active || resized || !timer_active {
            InvalidateRect(hwnd, null(), 0);
        }
    }
    pub(super) unsafe fn paint_item(
        &mut self,
        dc: HDC,
        item: &Item,
        r: [f32; 4],
        scale: f32,
        lift: f32,
        ghost: bool,
        tilt: f32,
    ) {
        self.cache_icon(item);
        let mut p = palette(self.dock.light);
        let y = r[1] + self.dock.icon_size as f32 + 14.;
        let (ink, muted) = self.adaptive_ink(
            dc,
            [r[0] + 3., y, r[2] - 3., y + 20.],
            &format!("label-{}", item.id),
        );
        p.text = ink;
        p.muted = muted;
        if let Some(Icon::Image(source)) = self.icons.get(&item.id) {
            let px = (self.dock.icon_size as f32 * self.dpi * 1.12).ceil() as u32;
            let key = (source.identity(), px);
            if self
                .scaled_icons
                .get(&item.id)
                .is_none_or(|(old, _)| *old != key)
            {
                if let Some(image) = source.resized(px) {
                    self.profile_counts[1] += 1;
                    if let Some((pad, shadow)) = image.icon_shadow() {
                        self.icon_shadows.insert(item.id.clone(), (pad, shadow));
                    }
                    self.scaled_icons.insert(item.id.clone(), (key, image));
                }
            }
        }
        let translating = self.scroll.moving(self.paint_time)
            || self.motion.nodes.get(&item.id).is_some_and(|node| {
                node.x.moving(self.paint_time) || node.y.moving(self.paint_time)
            });
        let text_has_shadow = crate::contrast::luminance(p.text) > 0.5;
        if !text_has_shadow {
            self.label_shadows.remove(&item.id);
        }
        let mut deferred_label = false;
        if self.labels_visible() {
            let width = ((r[2] - r[0] - 6.) * 64.).round() / 64.;
            let key = format!("{}:{}:{}:{}", item.name, width, self.dpi, p.text);
            let live_reuse = self.resize_drag.is_some()
                && self.labels.get(&item.id).is_some_and(|(old, _)| {
                    old.starts_with(&format!("{}:", item.name))
                        && old.ends_with(&format!(":{}:{}", self.dpi, p.text))
                });
            let needs_label =
                !live_reuse && self.labels.get(&item.id).is_none_or(|(k, _)| *k != key);
            deferred_label = needs_label
                && self.label_budget == 0
                && self.labels.get(&item.id).is_some_and(|(old, _)| {
                    old.starts_with(&format!("{}:", item.name))
                        && old.ends_with(&format!(":{}:{}", self.dpi, p.text))
                });
            self.cache_pending |= deferred_label;
            if needs_label && !deferred_label {
                self.label_budget = self.label_budget.saturating_sub(1);
                self.profile_counts[2] += 1;
                if let Some(image) = Canvas::label(&item.name, width, 20., self.dpi, p.text) {
                    let pad = (2. * self.dpi).ceil() as u32;
                    if text_has_shadow {
                        if let Some(shadow) = image.shadow(self.dpi * 0.65, 80, pad) {
                            self.label_shadows.insert(item.id.clone(), (pad, shadow));
                        }
                    }
                    self.labels.insert(item.id.clone(), (key, image));
                }
            }
        }
        let resizing = self.resize_drag.is_some() || deferred_label;
        if !ghost && (resizing || ((scale - 1.).abs() < 0.0001 && lift.abs() < 0.0001)) {
            if let Some((_, icon)) = self.scaled_icons.get(&item.id) {
                let style = format!(
                    "{}:{}:{}:{}:{}:{}:{}|",
                    item.name,
                    icon.identity(),
                    self.dpi,
                    p.text,
                    self.labels_visible(),
                    self.dock.icon_size,
                    self.labels
                        .get(&item.id)
                        .map_or("", |(key, _)| key.as_str())
                );
                let size = [
                    (64. * (r[2] - r[0])).round() / 64.,
                    (64. * (r[3] - r[1])).round() / 64.,
                ];
                let key = format!("{style}{}:{}", size[0], size[1]);
                let live_reuse = resizing
                    && self
                        .tile_cache
                        .get(&item.id)
                        .is_some_and(|(old, _, _)| old.starts_with(&style));
                if !live_reuse
                    && self
                        .tile_cache
                        .get(&item.id)
                        .is_none_or(|(old, _, _)| *old != key)
                {
                    let icon_shadow = self
                        .icon_shadows
                        .get(&item.id)
                        .map(|(p, image)| (*p, image));
                    let label = if self.labels_visible() {
                        self.labels.get(&item.id).map(|(_, image)| {
                            (
                                image,
                                self.label_shadows
                                    .get(&item.id)
                                    .map(|(p, image)| (*p, image)),
                            )
                        })
                    } else {
                        None
                    };
                    self.profile_counts[3] += 1;
                    if let Some((pad, image)) = Canvas::tile(
                        icon,
                        icon_shadow,
                        label,
                        size,
                        self.dock.icon_size as f32,
                        self.dpi,
                    ) {
                        self.tile_cache.insert(item.id.clone(), (key, pad, image));
                    }
                }
                if let Some((_, pad, image)) = self.tile_cache.get(&item.id) {
                    let size = image.pixel_size();
                    let width = size[0] as f32 / self.dpi;
                    let height = size[1] as f32 / self.dpi;
                    let center = (r[0] + r[2]) * 0.5;
                    let rect = [
                        center - width * 0.5,
                        r[1] - pad,
                        center + width * 0.5,
                        r[1] - pad + height,
                    ];
                    let canvas = Canvas::new(dc, self.dpi);
                    if resizing {
                        canvas.clipped_image(
                            image,
                            rect,
                            [r[0] - pad, r[1] - pad, r[2] + pad, r[3] + pad],
                        );
                    } else if translating {
                        canvas.image(image, rect);
                    } else {
                        canvas.image_snapped(image, rect);
                    }
                    return;
                }
            }
        }
        let canvas = Canvas::new(dc, self.dpi);
        if ghost {
            let cx = (r[0] + r[2]) / 2.;
            let cy = (r[1] + r[3]) / 2.;
            canvas.transform_about([cx, cy], scale, tilt);
            for i in (1..=7).rev() {
                let spread = i as f32 * 1.4;
                canvas.round(
                    [
                        r[0] - spread,
                        r[1] + 8. - spread,
                        r[2] + spread,
                        r[3] + 8. + spread,
                    ],
                    20. + spread,
                    0,
                    7,
                    false,
                    false,
                );
            }
            canvas.glass_surface(r, 20., 48, self.dock.light);
        }
        let size = self.dock.icon_size as f32 * if ghost { 1. } else { scale };
        let center = (r[0] + r[2]) / 2.;
        let y = r[1] + 8. + lift + (self.dock.icon_size as f32 - size) / 2.;
        let icon_rect = [center - size / 2., y, center + size / 2., y + size];
        if let Some((pad, shadow)) = self.icon_shadows.get(&item.id) {
            let px = (self.dock.icon_size as f32 * self.dpi * 1.12).ceil();
            let spread = *pad as f32 / px * size;
            let draw = if ghost || (scale - 1.).abs() > 0.0001 {
                Canvas::image_cpu
            } else {
                Canvas::image
            };
            draw(
                &canvas,
                shadow,
                [
                    icon_rect[0] - spread + 0.65,
                    icon_rect[1] - spread + 1.2,
                    icon_rect[2] + spread + 0.65,
                    icon_rect[3] + spread + 1.2,
                ],
            );
        }
        match self.icons.get(&item.id) {
            Some(Icon::Image(image)) => {
                let image = self
                    .scaled_icons
                    .get(&item.id)
                    .map_or(image, |(_, image)| image);
                if ghost || (scale - 1.).abs() > 0.0001 {
                    canvas.image_cpu(image, icon_rect);
                } else {
                    canvas.image(image, icon_rect);
                }
            }
            Some(Icon::Shell(handle)) if !handle.is_null() && !render::has_alpha(dc) => {
                DrawIconEx(
                    dc,
                    (icon_rect[0] * self.dpi) as i32,
                    (icon_rect[1] * self.dpi) as i32,
                    *handle,
                    (size * self.dpi) as i32,
                    (size * self.dpi) as i32,
                    0,
                    null_mut(),
                    DI_NORMAL,
                );
            }
            _ => {
                canvas.round(icon_rect, 12., p.text, 25, false, false);
                canvas.text(
                    &item.name.chars().next().unwrap_or('·').to_string(),
                    icon_rect,
                    size * 0.5,
                    true,
                    p.text,
                    1,
                );
            }
        }
        if self.labels_visible() {
            if let Some((_, image)) = self.labels.get(&item.id) {
                let y = r[1] + self.dock.icon_size as f32 + 14.;
                let rect = render::label_rect(
                    image,
                    (r[0] + r[2]) * 0.5,
                    y,
                    self.dpi,
                    !translating && !ghost,
                );
                let clip = render::label_clip(
                    r[2] - r[0] - 6.,
                    (r[0] + r[2]) * 0.5,
                    y,
                    self.dpi,
                    !translating && !ghost,
                );
                if let Some((pad, shadow)) = self.label_shadows.get(&item.id) {
                    let spread = *pad as f32 / self.dpi;
                    canvas.clipped_image(
                        shadow,
                        [
                            rect[0] - spread + 0.2,
                            y - spread + 0.65,
                            rect[2] + spread + 0.2,
                            rect[3] + spread + 0.65,
                        ],
                        [
                            clip[0] - spread,
                            y - spread,
                            clip[2] + spread,
                            clip[3] + spread + 1.,
                        ],
                    );
                }
                canvas.clipped_image(image, rect, clip);
            }
        }
    }
    pub(super) unsafe fn paint_tiles(
        &mut self,
        dc: HDC,
        hwnd: HWND,
        w: i32,
        h: i32,
    ) -> (usize, usize, usize, i32, i32) {
        let batch = render::Batch::new(dc, self.dpi);
        let drawing_started = std::time::Instant::now();
        let now = self.paint_time;
        let columns = self.columns(hwnd);
        let rows = self.page_rows(hwnd);
        let row = self.row_height();
        let top = self.grid_top();
        let visible = self.visible(hwnd);
        if self.tile_cache.len() > 128 {
            let keep: std::collections::HashSet<_> = self
                .dock
                .items
                .iter()
                .filter(|i| !i.catalog)
                .map(|i| i.id.clone())
                .chain(visible.iter().map(|i| self.dock.items[*i].id.clone()))
                .collect();
            self.tile_cache.retain(|id, _| keep.contains(id));
            self.labels.retain(|id, _| keep.contains(id));
            self.label_shadows.retain(|id, _| keep.contains(id));
            self.scaled_icons.retain(|id, _| keep.contains(id));
            self.icon_shadows.retain(|id, _| keep.contains(id));
        }
        let total = visible.len()
            + usize::from(self.query.trim().is_empty() && self.group_selection.is_none());
        let content_end = top as f32 + total.div_ceil(columns) as f32 * row as f32;
        let bottom = h as f32 - 14.;
        self.scroll.bounds(
            if content_end > bottom {
                content_end - bottom + 28.
            } else {
                0.
            },
            now,
        );
        let scroll = self.scroll.value(now);
        let cell = (w - 26) / columns as i32;
        let layout: Vec<_> = (0..total)
            .map(|pos| {
                let id = visible
                    .get(pos)
                    .map_or_else(|| "__add".into(), |i| self.dock.items[*i].id.clone());
                let point = [
                    (16 + (pos % columns) as i32 * cell) as f32,
                    (top + (pos / columns) as i32 * row) as f32,
                ];
                (id, point)
            })
            .collect();
        self.motion
            .nodes
            .retain(|id, _| id == "__add" || self.dock.items.iter().any(|item| item.id == *id));
        self.motion.sync(&layout, now, self.resize_drag.is_none());
        let accent = self.selection_color();
        for screen_row in 0..total.div_ceil(columns) {
            let mut col = 0;
            while col < columns {
                let pos = screen_row * columns + col;
                let group = visible
                    .get(pos)
                    .and_then(|i| self.dock.items[*i].group.clone());
                let mut count = 1;
                if let Some(group) = group {
                    while col + count < columns
                        && visible
                            .get(pos + count)
                            .and_then(|i| self.dock.items[*i].group.as_deref())
                            == Some(group.as_str())
                    {
                        count += 1;
                    }
                    let x = (16 + col as i32 * cell) as f32;
                    let y = (top + screen_row as i32 * row) as f32 - scroll;
                    if y + (row as f32) < (top - 14) as f32 || y > bottom {
                        col += count;
                        continue;
                    }
                    let r = [
                        x,
                        y,
                        x + cell as f32 * count as f32 - 6.,
                        y + row as f32 - 4.,
                    ];
                    if self.dock.opacity < 100. {
                        if let Some(background) = &self.background {
                            background.restore_glass_card(dc, r, 20.);
                        }
                    }
                    let canvas = Canvas::new(dc, self.dpi);
                    canvas.group_surface_live(r, 20., self.resize_drag.is_some());
                    let amount = self.motion.group_alpha(&group, now);
                    if amount > 0.001 {
                        canvas.round(r, 20., accent, (amount * 46.) as u8, false, false);
                        canvas.round(r, 20., accent, (amount * 205.) as u8, true, false);
                    }
                }
                col += count;
            }
        }
        let group_ms = drawing_started.elapsed().as_secs_f64() * 1000.;
        if visible.is_empty() && !self.query.is_empty() {
            let (ink, _) =
                self.adaptive_ink(dc, [20., 124., (w - 20) as f32, 164.], "empty-search");
            self.text(
                dc,
                if self.catalog_rx.is_some() {
                    "Searching installed apps..."
                } else {
                    "No matching apps"
                },
                [20, 124, w - 20, 164],
                15,
                false,
                ink,
                DT_CENTER,
            );
        }
        for pos in 0..total {
            let (id, target) = &layout[pos];
            let visual = self.motion.visual(id, *target, now);
            let [x, content_y] = visual.position;
            let y = content_y - scroll;
            if y + (row as f32) < (top - 14) as f32 || y > bottom {
                continue;
            }
            let r = [x, y, x + cell as f32 - 6., y + row as f32 - 4.];
            let mut p = palette(self.dock.light);
            let label_y = y + self.dock.icon_size as f32 + 14.;
            let (ink, muted) = self.adaptive_ink(
                dc,
                [r[0] + 3., label_y, r[2] - 3., label_y + 20.],
                &format!("tile-{id}"),
            );
            p.text = ink;
            p.muted = muted;
            let c = Canvas::new(dc, self.dpi);
            let selected = self.keyboard_selection && self.selected == pos;
            let hover = if self.resize_drag.is_some() {
                0.
            } else {
                visual.hover.max(if selected { 1. } else { 0. })
            };
            if hover > 0.001 {
                c.hover_surface(r, 20., hover, self.dock.light);
                c.glow(
                    r,
                    20.,
                    visual.pointer,
                    if self.dock.light { 0xffffff } else { p.text },
                    (visual.hover * if self.dock.light { 28. } else { 40. }) as u8,
                );
            }
            if visual.launch > 0.001 {
                c.round(r, 20., accent, (visual.launch * 30.) as u8, false, false);
                c.round(r, 20., accent, (visual.launch * 170.) as u8, true, false);
                c.glow(
                    r,
                    20.,
                    [(r[0] + r[2]) / 2., r[1] + 30.],
                    accent,
                    (visual.launch * 110.) as u8,
                );
            }
            if id == "__add" {
                let half = self.dock.icon_size as f32 * (1. + visual.hover * 0.04) / 2.;
                let center = (r[0] + r[2]) / 2.;
                let cy = y + 8. + self.dock.icon_size as f32 / 2.;
                let add = [center - half, cy - half, center + half, cy + half];
                c.round(add, 13., p.text, 12, false, false);
                c.round(
                    add,
                    13.,
                    p.text,
                    (90. + visual.hover * 60.) as u8,
                    true,
                    true,
                );
                drop(c);
                self.symbols
                    .draw(dc, self.dpi, 6, center - 10., cy - 10., 20., p.text);
                let c = Canvas::new(dc, self.dpi);
                if self.labels_visible() {
                    c.text(
                        "Add",
                        [
                            x,
                            y + self.dock.icon_size as f32 + 14.,
                            r[2],
                            y + self.dock.icon_size as f32 + 34.,
                        ],
                        11.,
                        true,
                        p.muted,
                        1,
                    );
                }
                continue;
            }
            let dragged = self
                .tile_drag
                .as_ref()
                .is_some_and(|d| d.active && d.id == *id);
            let landing = self.motion.landing.as_ref().is_some_and(|l| l.id == *id);
            if dragged || landing {
                if dragged {
                    c.round(r, 20., p.text, 13, false, false);
                    c.round(r, 20., p.text, 66, true, true);
                }
                continue;
            }
            let item = self.dock.items[visible[pos]].clone();
            drop(c);
            self.paint_item(
                dc,
                &item,
                r,
                if self.group_selection.is_some() {
                    1.
                } else {
                    visual.scale
                },
                if self.group_selection.is_some() {
                    0.
                } else {
                    visual.lift
                },
                false,
                0.,
            );
            self.paint_group_checkbox(dc, &item.id, r);
        }
        drop(batch);
        if self.motion.tick(now)
            || self.scroll.moving(now)
            || self.tile_drag.as_ref().is_some_and(|d| d.active)
        {
            self.ensure_motion_timer(hwnd);
        }
        if std::env::var_os("LUMEN_TRACE_FRAMES").is_some() {
            eprintln!(
                "native-grid group={:.2} all={:.2}",
                group_ms,
                drawing_started.elapsed().as_secs_f64() * 1000.
            );
        }
        (columns, rows, total, row, top)
    }
    pub(super) unsafe fn paint_floating(&mut self, dc: HDC, hwnd: HWND, w: i32) {
        let now = self.paint_time;
        let cell = (w - 26) / self.columns(hwnd) as i32;
        let row = self.row_height();
        let floating = if let Some(d) = self.tile_drag.as_ref().filter(|d| d.active) {
            Some((d.id.clone(), d.point(), 1.06, 1.5))
        } else {
            self.motion.landing.as_ref().map(|l| {
                let t =
                    crate::motion::bezier(((now - l.start) / 260.) as f32, [0.22, 0.9, 0.24, 1.]);
                (
                    l.id.clone(),
                    l.point(now),
                    1. + 0.06 * (1. - t),
                    1.5 * (1. - t),
                )
            })
        };
        if let Some((id, [x, y], scale, rotation)) = floating {
            if let Some(item) = self.dock.items.iter().find(|i| i.id == id).cloned() {
                self.paint_item(
                    dc,
                    &item,
                    [x, y, x + cell as f32 - 6., y + row as f32 - 4.],
                    scale,
                    0.,
                    true,
                    rotation,
                );
            }
        }
    }
}
