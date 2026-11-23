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
            && (p[0] - self.header_layout(self.dimensions(hwnd).0).clear_x)
                .hypot(p[1] - 37.)
                < 12.
    }
    pub(super) unsafe fn clear_search(&mut self, hwnd: HWND) {
        SetWindowTextW(self.edit, wide("").as_ptr());
        self.query.clear();
        self.bump_dock_revision();
        self.selected = 0;
        self.scroll.snap(0.);
        self.layout_edit(hwnd);
        SetFocus(self.edit);
        self.arm_motion(hwnd);
        InvalidateRect(hwnd, null(), 0);
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
            SetTimer(hwnd, MOTION_TIMER, crate::motion::frame_interval_ms(), None);
            self.motion.timer_running = true;
            InvalidateRect(hwnd, null(), 0);
        }
    }
    pub(super) unsafe fn arm_motion(&mut self, hwnd: HWND) {
        self.ensure_motion_timer(hwnd);
    }
    pub(super) unsafe fn hover_tile(&mut self, hwnd: HWND, p: Point) {
        let now = self.now();
        let hover = self.hit(hwnd, p[0] as i32, p[1] as i32);
        let width = self.dimensions(hwnd).0;
        let prev_toolbar = self.toolbar_hit(width, self.pointer);
        let next_toolbar = self.toolbar_hit(width, p);
        let prev_clear = !self.query.is_empty() && self.clear_hit(hwnd, self.pointer);
        let next_clear = !self.query.is_empty() && self.clear_hit(hwnd, p);
        if self.pointer == p
            && self.hover == hover
            && prev_toolbar == next_toolbar
            && prev_clear == next_clear
        {
            return;
        }
        let hover_changed = self.hover != hover;
        let toolbar_changed = prev_toolbar != next_toolbar;
        let clear_changed = prev_clear != next_clear;
        self.pointer = p;
        self.hover = hover;
        if toolbar_changed {
            let toolbar_count = self.toolbar_count();
            self.motion
                .toolbar_hover_count(next_toolbar, toolbar_count, now);
        }
        if clear_changed {
            self.motion.search_clear_hover(next_clear, now);
        }
        self.group_selection_move(hwnd, p);
        let id = if let Some(pos) = self.hover {
            let (visible, _) = self.cached_visible_and_layout(hwnd);
            visible
                .get(pos)
                .map(|i| self.dock.items[*i].id.clone())
                .or_else(|| (pos == visible.len()).then(|| "__add".into()))
        } else {
            None
        };
        self.motion.hover(id, p, now);
        self.arm_motion(hwnd);
        if (hover_changed
            || self.hover.is_some()
            || toolbar_changed
            || next_toolbar.is_some()
            || clear_changed)
            && !self.frame_queued
        {
            InvalidateRect(hwnd, null(), 0);
        }
    }
    pub(super) unsafe fn pointer_down(&mut self, hwnd: HWND, pos: usize, p: Point) {
        let (visible, grid_layout) = self.cached_visible_and_layout(hwnd);
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
        let (col, sr) = grid_layout
            .cells
            .get(pos)
            .copied()
            .unwrap_or((pos % columns, pos / columns));
        let origin = [
            (16 + col as i32 * cell) as f32,
            (self.grid_top() + sr as i32 * self.row_height()) as f32
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
        let (visible, layout) = self.cached_visible_and_layout(hwnd);
        let columns = self.columns(hwnd);
        let cell = (w - 26) / columns as i32;
        let row = self.row_height();
        for (pos, i) in visible.iter().enumerate() {
            let item = &self.dock.items[*i];
            if item.id == dragged {
                continue;
            }
            let Some(group) = item.group.as_deref() else {
                continue;
            };
            let (col, sr) = layout
                .cells
                .get(pos)
                .copied()
                .unwrap_or((pos % columns, pos / columns));
            let x = (16 + col as i32 * cell) as f32;
            let y = (self.grid_top() + sr as i32 * row) as f32 - self.scroll.value(self.now());
            if p[0] >= x - 3.
                && p[0] <= x + cell as f32 - 3.
                && p[1] >= y - 3.
                && p[1] <= y + row as f32 - 1.
            {
                return Some(group.into());
            }
        }
        None
    }
    pub(super) unsafe fn pointer_move(&mut self, hwnd: HWND, p: Point) {
        let can_sort = self.query.trim().is_empty();
        if let Some(drag) = &mut self.tile_drag {
            if can_sort && drag.update(p) {
                let id = drag.id.clone();
                let drag_len = drag.preview.len();
                let own = self
                    .dock
                    .items
                    .iter()
                    .find(|i| i.id == id)
                    .and_then(|i| i.group.clone());
                let toolbar_count = self.toolbar_count();
                self.motion.toolbar_hover_count(None, toolbar_count, self.now());
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
                let (_, layout) = self.cached_visible_and_layout(hwnd);
                let best = layout
                    .cells
                    .iter()
                    .take(drag_len)
                    .enumerate()
                    .min_by(|(_, &(ca, ra)), (_, &(cb, rb))| {
                        let d2 = |c: usize, r: usize| {
                            let cx = 16. + c as f32 * cell + (cell - 6.) * 0.5;
                            let cy = top + r as f32 * row - scroll + (row - 4.) * 0.5;
                            (p[0] - cx).powi(2) + (p[1] - cy).powi(2)
                        };
                        d2(ca, ra).total_cmp(&d2(cb, rb))
                    })
                    .map(|(idx, _)| idx);
                if let Some(drag) = &mut self.tile_drag {
                    drag.target_group = target.clone();
                    if target.is_none() || target == own {
                        if let Some(index) = best {
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
        self.motion.search_clear_hover(false, self.now());
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
                let (visible, grid_layout) = self.cached_visible_and_layout(hwnd);
                let columns = self.columns(hwnd);
                let cell = (self.dimensions(hwnd).0 - 26) / columns as i32;
                if let Some(pos) = visible
                    .iter()
                    .position(|i| self.dock.items[*i].id == drag.id)
                {
                    let (col, sr) = grid_layout
                        .cells
                        .get(pos)
                        .copied()
                        .unwrap_or((pos % columns, pos / columns));
                    let to = [
                        (16 + col as i32 * cell) as f32,
                        (self.grid_top() + sr as i32 * self.row_height()) as f32
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
        InvalidateRect(hwnd, null(), 0);
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
        if self.tile_drag.is_none()
            && self.bar_drag.is_none()
            && self.popup.is_none()
            && self.pointer[0] >= 0.
            && (self.scroll.moving(now) || self.hover.is_none())
        {
            self.hover_tile(hwnd, self.pointer);
        }
        let active = self.motion.tick(now)
            || self.scroll.moving(now)
            || self.tile_drag.as_ref().is_some_and(|d| d.active)
            || self.popup.as_ref().is_some_and(|p| p.moving(now))
            || self.group_selection_moving(now)
            || self.cache_pending;
        let timer_active = active;
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
        batch: Option<&render::Batch>,
        item_idx: usize,
        r: [f32; 4],
        scale: f32,
        lift: f32,
        ghost: bool,
        tilt: f32,
    ) {
        if !self.icons.contains_key(&self.dock.items[item_idx].id) {
            if self.icon_budget > 0 {
                self.icon_budget = self.icon_budget.saturating_sub(1);
                let item = self.dock.items[item_idx].clone();
                self.cache_icon(&item);
            } else {
                self.cache_pending = true;
            }
        }
        let mut p = palette(self.dock.light);
        p.text = self.search_ink;
        p.muted = self.search_muted;
        let labels_visible = self.labels_visible();
        let dpi = self.dpi;
        let icon_size = self.dock.icon_size;
        let resizing_drag = self.resize_drag.is_some();
        let scrolling = self.scroll.moving(self.paint_time);
        let item = &self.dock.items[item_idx];
        if let Some(Icon::Image(source)) = self.icons.get(&item.id) {
            let px = (icon_size as f32 * dpi * 1.12).ceil() as u32;
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
        let x_moving = self
            .motion
            .nodes
            .get(&item.id)
            .is_some_and(|node| node.x.moving(self.paint_time));
        let translating = scrolling
            || x_moving
            || self
                .motion
                .nodes
                .get(&item.id)
                .is_some_and(|node| node.y.moving(self.paint_time));
        let text_has_shadow = crate::contrast::luminance(p.text) > 0.5;
        if !text_has_shadow {
            self.label_shadows.remove(&item.id);
        }
        let mut deferred_label = false;
        if labels_visible {
            let width = ((r[2] - r[0] - 6.) * 64.).round() / 64.;
            let width_q = ((r[2] - r[0] - 6.) * 64.).round() as i32;
            let dpi_bits = dpi.to_bits();
            let same_style = self.labels.get(&item.id).is_some_and(|(old, _)| {
                old.name == item.name && old.dpi_bits == dpi_bits && old.text_color == p.text
            });
            let live_reuse = resizing_drag && same_style;
            let exact_label = same_style
                && self
                    .labels
                    .get(&item.id)
                    .is_some_and(|(old, _)| old.width_q == width_q);
            let needs_label = !live_reuse && !exact_label;
            deferred_label = needs_label && self.label_budget == 0 && same_style;
            self.cache_pending |= deferred_label;
            if needs_label && !deferred_label {
                self.label_budget = self.label_budget.saturating_sub(1);
                self.profile_counts[2] += 1;
                if let Some(image) = Canvas::label(&item.name, width, 20., dpi, p.text) {
                    let pad = (2. * dpi).ceil() as u32;
                    if text_has_shadow {
                        if let Some(shadow) = image.shadow(dpi * 0.65, 80, pad) {
                            self.label_shadows.insert(item.id.clone(), (pad, shadow));
                        }
                    }
                    let key = LabelKey {
                        name: item.name.clone(),
                        width_q,
                        dpi_bits,
                        text_color: p.text,
                    };
                    self.labels.insert(item.id.clone(), (key, image));
                }
            }
        }
        let resizing = resizing_drag || deferred_label;
        let scaled_or_lifted = (scale - 1.).abs() >= 0.0001 || lift.abs() >= 0.0001;
        if !ghost && (resizing || tilt.abs() < 0.001) {
            if let Some((_, icon)) = self.scaled_icons.get(&item.id) {
                let icon_id = icon.identity();
                let dpi_bits = dpi.to_bits();
                let label_key = if labels_visible {
                    self.labels.get(&item.id).map(|(k, _)| k)
                } else {
                    None
                };
                let size = [
                    (64. * (r[2] - r[0])).round() / 64.,
                    (64. * (r[3] - r[1])).round() / 64.,
                ];
                let size_q = [
                    (64. * (r[2] - r[0])).round() as i32,
                    (64. * (r[3] - r[1])).round() as i32,
                ];
                let same_style = |old: &TileKey| {
                    old.name == item.name
                        && old.icon_id == icon_id
                        && old.dpi_bits == dpi_bits
                        && old.text_color == p.text
                        && old.labels_visible == labels_visible
                        && old.icon_size == icon_size
                        && old.label_key.as_ref() == label_key
                };
                let live_reuse = resizing
                    && self
                        .tile_cache
                        .get(&item.id)
                        .is_some_and(|(old, _, _)| same_style(old));
                let exact_hit = self
                    .tile_cache
                    .get(&item.id)
                    .is_some_and(|(old, _, _)| same_style(old) && old.size_q == size_q);
                if !live_reuse && !exact_hit {
                    let icon_shadow = self
                        .icon_shadows
                        .get(&item.id)
                        .map(|(p, image)| (*p, image));
                    let label = if labels_visible {
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
                        icon_size as f32,
                        dpi,
                    ) {
                        let key = TileKey {
                            name: item.name.clone(),
                            icon_id,
                            dpi_bits,
                            text_color: p.text,
                            labels_visible,
                            icon_size,
                            label_key: label_key.cloned(),
                            size_q,
                        };
                        self.tile_cache.insert(item.id.clone(), (key, pad, image));
                    }
                }
                if let Some((cached_key, pad, image)) = self.tile_cache.get(&item.id) {
                    let size = image.pixel_size();
                    let width = size[0] as f32 / dpi;
                    let height = size[1] as f32 / dpi;
                    let center = (r[0] + r[2]) * 0.5;
                    let left = if x_moving {
                        center - width * 0.5
                    } else {
                        (((r[0] + r[2]) * dpi - size[0] as f32) * 0.5).round() / dpi
                    };
                    let key_matches = same_style(cached_key) && cached_key.size_q == size_q;
                    if !resizing && tilt.abs() < 0.001 && key_matches && scaled_or_lifted {
                        if let Some(b) = batch {
                            let center_y = r[1] + lift + 8. + icon_size as f32 * 0.5;
                            if b.blit_scaled(
                                image,
                                [
                                    left,
                                    r[1] + lift - pad,
                                    left + width,
                                    r[1] + lift - pad + height,
                                ],
                                [center, center_y],
                                scale,
                            ) {
                                return;
                            }
                        }
                    } else if resizing || !scaled_or_lifted {
                        let rect = [
                            left,
                            r[1] - pad,
                            left + width,
                            r[1] - pad + height,
                        ];
                        if !resizing {
                            if let Some(b) = batch {
                                if b.blit(image, rect, !translating) {
                                    return;
                                }
                            }
                        }
                        let canvas = Canvas::new(dc, dpi);
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
        }
        let canvas = Canvas::new(dc, dpi);
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
        let size = icon_size as f32 * if ghost { 1. } else { scale };
        let center = (r[0] + r[2]) / 2.;
        let y = r[1] + 8. + lift + (icon_size as f32 - size) / 2.;
        let icon_rect = [center - size / 2., y, center + size / 2., y + size];
        if let Some((pad, shadow)) = self.icon_shadows.get(&item.id) {
            let px = (icon_size as f32 * dpi * 1.12).ceil();
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
                    (icon_rect[0] * dpi) as i32,
                    (icon_rect[1] * dpi) as i32,
                    *handle,
                    (size * dpi) as i32,
                    (size * dpi) as i32,
                    0,
                    null_mut(),
                    DI_NORMAL,
                );
            }
            _ => {
                canvas.round(icon_rect, 12., p.text, 25, false, false);
                let mut buf = [0u8; 4];
                let ch = item.name.chars().next().unwrap_or('·').encode_utf8(&mut buf);
                canvas.text(ch, icon_rect, size * 0.5, true, p.text, 1);
            }
        }
        if labels_visible {
            if let Some((_, image)) = self.labels.get(&item.id) {
                let y = r[1] + icon_size as f32 + 14.;
                let rect = render::label_rect(
                    image,
                    (r[0] + r[2]) * 0.5,
                    y,
                    dpi,
                    !translating && !ghost,
                );
                let clip = render::label_clip(
                    r[2] - r[0] - 6.,
                    (r[0] + r[2]) * 0.5,
                    y,
                    dpi,
                    !translating && !ghost,
                );
                if let Some((pad, shadow)) = self.label_shadows.get(&item.id) {
                    let spread = *pad as f32 / dpi;
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
        px_w: i32,
        px_h: i32,
        skip_groups: bool,
        can_cache_groups: bool,
        groups_key: String,
    ) -> (usize, usize, usize, i32, i32) {
        let batch = render::Batch::new(dc, self.dpi);
        let drawing_started = std::time::Instant::now();
        let now = self.paint_time;
        let columns = self.columns(hwnd);
        let rows = self.page_rows(hwnd);
        let row = self.row_height();
        let top = self.grid_top();
        let (visible, grid_layout) = self.cached_visible_and_layout(hwnd);
        if self.tile_cache.len() > 128 {
            let keep: std::collections::HashSet<&str> = self
                .dock
                .items
                .iter()
                .filter(|i| !i.catalog)
                .map(|i| i.id.as_str())
                .chain(visible.iter().map(|i| self.dock.items[*i].id.as_str()))
                .collect();
            self.tile_cache.retain(|id, _| keep.contains(id.as_str()));
            self.labels.retain(|id, _| keep.contains(id.as_str()));
            self.label_shadows.retain(|id, _| keep.contains(id.as_str()));
            self.scaled_icons.retain(|id, _| keep.contains(id.as_str()));
            self.icon_shadows.retain(|id, _| keep.contains(id.as_str()));
        }
        let has_add = self.query.trim().is_empty() && self.group_selection.is_none();
        let total = visible.len() + usize::from(has_add);
        let content_end = top as f32 + grid_layout.rows as f32 * row as f32;
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
        let scrolling = self.scroll.moving(now);
        let cell = (w - 26) / columns as i32;
        let dragging = self.tile_drag.as_ref().is_some_and(|d| d.active);
        let layout_sync_key = (
            self.dock_revision,
            self.layout_generation.get(),
            w,
            h,
            columns,
            self.labels_visible(),
            self.search_visible(),
        );
        if dragging || layout_sync_key != self.synced_layout_key {
            let layout: Vec<_> = (0..total)
                .map(|pos| {
                    let id = visible
                        .get(pos)
                        .map_or_else(|| "__add".into(), |i| self.dock.items[*i].id.clone());
                    let (col, sr) = grid_layout
                        .cells
                        .get(pos)
                        .copied()
                        .unwrap_or((pos % columns, pos / columns));
                    let point = [(16 + col as i32 * cell) as f32, (top + sr as i32 * row) as f32];
                    (id, point)
                })
                .collect();
            let item_ids: std::collections::HashSet<&str> =
                self.dock.items.iter().map(|item| item.id.as_str()).collect();
            self.motion
                .nodes
                .retain(|id, _| id == "__add" || item_ids.contains(id.as_str()));
            self.motion.sync(&layout, now, self.resize_drag.is_none());
            self.synced_layout_key = if dragging {
                (0, 0, 0, 0, 0, false, false)
            } else {
                layout_sync_key
            };
        }
        let accent = self.selection_color();
        let low_power = crate::motion::low_power();
        let moving_win = self.moving_window || crate::fusion::is_moving(hwnd);
        let allow_liquid = !(low_power && self.resize_drag.is_some());
        if !skip_groups {
            for (group, cells) in &grid_layout.group_runs {
                let mut row_spans: Vec<[f32; 4]> = Vec::new();
                let mut idx = 0usize;
                while idx < cells.len() {
                    let (col_start, sr) = cells[idx];
                    let mut count = 1usize;
                    while idx + count < cells.len()
                        && cells[idx + count].1 == sr
                        && cells[idx + count].0 == col_start + count
                    {
                        count += 1;
                    }
                    let x = ((16 + col_start as i32 * cell) as f32 * self.dpi).round() / self.dpi;
                    let y = (top + sr as i32 * row) as f32 - scroll;
                    row_spans.push([
                        x,
                        y,
                        x + cell as f32 * count as f32 - 6.,
                        y + row as f32 - 4.,
                    ]);
                    idx += count;
                }
                for comp in crate::render::split_connected_group_spans(&row_spans, 20.0 * 1.5) {
                    let mut by0 = f32::INFINITY;
                    let mut by1 = f32::NEG_INFINITY;
                    for s in &comp {
                        by0 = by0.min(s[1]);
                        by1 = by1.max(s[3]);
                    }
                    if by1 < (top - 14) as f32 || by0 > bottom {
                        continue;
                    }
                    if !moving_win || (!low_power && crate::glass::liquid_glass()) {
                        if (allow_liquid
                            && (self.dock.opacity < 100. || crate::glass::liquid_glass()))
                            || (!allow_liquid && self.dock.opacity < 100.)
                        {
                            if let Some(background) = &self.background {
                                background.restore_glass_group(dc, &comp, 20., allow_liquid);
                            }
                        }
                    }
                    let amount = if scrolling {
                        0.
                    } else {
                        self.motion.group_alpha(group, now)
                    };
                    if self.resize_drag.is_none()
                        && amount <= 0.001
                        && batch.group_poly_surface(&comp, 20., !scrolling)
                    {
                        // Blitted directly via batch
                    } else {
                        let canvas = Canvas::new(dc, self.dpi);
                        canvas.group_poly_surface_live(&comp, 20.);
                        if amount > 0.001 {
                            let verts = crate::render::ortho_group_polygon(&comp);
                            canvas.poly_round(&verts, 20., accent, (amount * 46.) as u8, false);
                            canvas.poly_round(&verts, 20., accent, (amount * 205.) as u8, true);
                        }
                    }
                }
            }
            if can_cache_groups && !grid_layout.group_runs.is_empty() {
                crate::render::flush(dc);
                let cache = self
                    .groups_base
                    .take()
                    .filter(|b| b.width == px_w && b.height == px_h && b.dpi == self.dpi)
                    .or_else(|| crate::compositor::Buffer::plain(px_w, px_h, self.dpi));
                if let Some(cache) = cache {
                    cache.copy_from(dc);
                    self.groups_base = Some(cache);
                    self.groups_base_key = groups_key;
                } else {
                    self.groups_base_key.clear();
                }
            } else {
                self.groups_base_key.clear();
            }
        }
        let group_ms = drawing_started.elapsed().as_secs_f64() * 1000.;
        if visible.is_empty() && !self.query.is_empty() {
            let ink = self.search_ink;
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
            let id_str: &str = visible
                .get(pos)
                .map_or("__add", |&idx| self.dock.items[idx].id.as_str());
            let (col, sr) = grid_layout
                .cells
                .get(pos)
                .copied()
                .unwrap_or((pos % columns, pos / columns));
            let target = [(16 + col as i32 * cell) as f32, (top + sr as i32 * row) as f32];
            let visual = self.motion.visual(id_str, target, now);
            let [x, content_y] = visual.position;
            let y = content_y - scroll;
            let offscreen = y + (row as f32) < (top - 14) as f32 || y > bottom;
            if offscreen
                && (scrolling
                    || self.resize_drag.is_some()
                    || pos >= visible.len()
                    || self.dock.items[visible[pos]].catalog
                    || self.tile_cache.contains_key(id_str))
            {
                continue;
            }
            let r = [x, y, x + cell as f32 - 6., y + row as f32 - 4.];
            let mut p = palette(self.dock.light);
            p.text = self.search_ink;
            p.muted = self.search_muted;
            let popup_selected = self
                .popup
                .as_ref()
                .and_then(popup::Popup::target_item_id)
                == Some(id_str);
            let selected = (self.keyboard_selection && self.selected == pos) || popup_selected;
            let hover = if self.resize_drag.is_some() {
                0.
            } else {
                visual.hover.max(if selected { 1. } else { 0. })
            };
            if hover > 0.001 {
                let bright = self.dock.light || crate::contrast::luminance(p.text) < 0.5;
                let glow_alpha = (visual.hover * if bright { 34. } else { 42. }) as u8;
                if !batch.hover_surface(r, 20., hover, bright) {
                    Canvas::new(dc, self.dpi).hover_surface(r, 20., hover, bright);
                }
                if !batch.hover_glow(r, 20., visual.pointer, glow_alpha) {
                    Canvas::new(dc, self.dpi).glow(r, 20., visual.pointer, 0xffffff, glow_alpha);
                }
            }
            if popup_selected {
                let c = Canvas::new(dc, self.dpi);
                c.round(r, 20., accent, 24, false, false);
                c.round(r, 20., accent, 155, true, false);
            }
            if visual.launch > 0.001 {
                let c = Canvas::new(dc, self.dpi);
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
            if id_str == "__add" {
                let cell_w = r[2] - r[0];
                let cell_h = r[3] - r[1];
                let add_key = (
                    (cell_w * self.dpi).round() as i32,
                    (cell_h * self.dpi).round() as i32,
                    self.dock.icon_size,
                    self.dpi.to_bits(),
                    p.text,
                    p.muted,
                    self.labels_visible(),
                );
                if hover <= 0.001
                    && (visual.scale - 1.0).abs() < 0.001
                    && !(self.dock.opacity < 100. && scrolling)
                {
                    if self.add_tile.as_ref().is_none_or(|(k, _)| *k != add_key) {
                        let cw = add_key.0.max(1) as u32;
                        let ch = add_key.1.max(1) as u32;
                        let half = self.dock.icon_size as f32 / 2.;
                        let cx_local = cell_w / 2.;
                        let cy_local = 8. + half;
                        let add = [
                            cx_local - half,
                            cy_local - half,
                            cx_local + half,
                            cy_local + half,
                        ];
                        let arm = 6.5 * 20. / 24.;
                        let stroke = 1.7 * 20. / 24.;
                        let labels_visible = self.labels_visible();
                        let icon_size = self.dock.icon_size as f32;
                        if let Some(image) = Canvas::render_sprite(cw, ch, self.dpi, |c| {
                            c.round(add, 13., p.text, 12, false, false);
                            c.round(add, 13., p.text, 90, true, true);
                            c.line(
                                &[(cx_local, cy_local - arm), (cx_local, cy_local + arm)],
                                p.text,
                                255,
                                stroke,
                            );
                            c.line(
                                &[(cx_local - arm, cy_local), (cx_local + arm, cy_local)],
                                p.text,
                                255,
                                stroke,
                            );
                            if labels_visible {
                                c.text(
                                    "Add",
                                    [0., icon_size + 14., cell_w, icon_size + 34.],
                                    11.,
                                    true,
                                    p.muted,
                                    1,
                                );
                            }
                        }) {
                            self.add_tile = Some((add_key, image));
                        }
                    }
                    if let Some((_, image)) = &self.add_tile {
                        if batch.blit(image, r, !scrolling) {
                            continue;
                        }
                    }
                }
                let _add_alpha = if self.dock.opacity < 100. && scrolling {
                    render::AlphaBatch::new(dc, self.dpi)
                } else {
                    None
                };
                let c = Canvas::new(dc, self.dpi);
                let half = self.dock.icon_size as f32 * visual.scale * (1. + hover * 0.04) / 2.;
                let center = (r[0] + r[2]) / 2.;
                let cy = y + 8. + self.dock.icon_size as f32 / 2.;
                let add = [center - half, cy - half, center + half, cy + half];
                c.round(add, 13., p.text, 12, false, false);
                c.round(
                    add,
                    13.,
                    p.text,
                    (90. + hover * 60.) as u8,
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
                .is_some_and(|d| d.active && d.id == id_str);
            let landing = self.motion.landing.as_ref().is_some_and(|l| l.id == id_str);
            if dragged || landing {
                if dragged {
                    let c = Canvas::new(dc, self.dpi);
                    c.round(r, 20., p.text, 13, false, false);
                    c.round(r, 20., p.text, 66, true, true);
                }
                continue;
            }
            let item_idx = visible[pos];
            let scale = if self.group_selection.is_some() {
                1.
            } else {
                visual.scale
            };
            let lift = if self.group_selection.is_some() {
                0.
            } else {
                visual.lift
            };
            self.paint_item(dc, Some(&batch), item_idx, r, scale, lift, false, 0.);
            if !offscreen {
                self.paint_group_checkbox(dc, &self.dock.items[item_idx].id, r);
            }
        }
        drop(batch);
        if self.motion.tick(now)
            || scrolling
            || self.tile_drag.as_ref().is_some_and(|d| d.active)
            || self.popup.as_ref().is_some_and(|p| p.moving(now))
            || self.group_selection_moving(now)
            || self.cache_pending
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
            if let Some(item_idx) = self.dock.items.iter().position(|i| i.id == id) {
                self.paint_item(
                    dc,
                    None,
                    item_idx,
                    [x, y, x + cell as f32 - 6., y + row as f32 - 4.],
                    scale,
                    0.,
                    true,
                    rotation,
                );
            }
        }
        if let Some((id, frame)) = self
            .motion
            .launch_frame(now)
            .map(|(id, frame)| (id.to_owned(), frame))
        {
            if frame.alpha > 0.001 {
                let origin = self
                    .motion
                    .nodes
                    .get(&id)
                    .map(|n| [n.x.sample(now), n.y.sample(now)])
                    .or_else(|| {
                        let columns = self.columns(hwnd);
                        let (visible, gl) = self.cached_visible_and_layout(hwnd);
                        visible
                            .iter()
                            .position(|idx| self.dock.items[*idx].id == id)
                            .map(|pos| {
                                let (col, sr) = gl
                                    .cells
                                    .get(pos)
                                    .copied()
                                    .unwrap_or((pos % columns, pos / columns));
                                [
                                    (16 + col as i32 * cell) as f32,
                                    (self.grid_top() + sr as i32 * row) as f32,
                                ]
                            })
                    });
                if let (Some([x, content_y]), Some(item)) = (
                    origin,
                    self.dock.items.iter().find(|i| i.id == id).cloned(),
                ) {
                    let scroll = self.scroll.value(now);
                    let cx = x + (cell as f32 - 6.) * 0.5;
                    let cy = (content_y - scroll) + 8. + self.dock.icon_size as f32 * 0.5;
                    let size = self.dock.icon_size as f32 * frame.scale;
                    let icon_rect = [
                        cx - size * 0.5,
                        cy - size * 0.5,
                        cx + size * 0.5,
                        cy + size * 0.5,
                    ];
                    let canvas = Canvas::new(dc, self.dpi);
                    if let Some(image) = self
                        .icons
                        .get(&item.id)
                        .and_then(|icon| match icon {
                            Icon::Image(img) => Some(img),
                            _ => None,
                        })
                        .or_else(|| self.scaled_icons.get(&item.id).map(|(_, img)| img))
                    {
                        canvas.image_cpu_alpha(image, icon_rect, frame.alpha);
                    } else {
                        let ink = palette(self.dock.light).text;
                        canvas.round(
                            icon_rect,
                            12. * frame.scale,
                            ink,
                            (25. * frame.alpha).round() as u8,
                            false,
                            false,
                        );
                        canvas.text_alpha(
                            &item.name.chars().next().unwrap_or('·').to_string(),
                            icon_rect,
                            size * 0.5,
                            true,
                            ink,
                            1,
                            (255. * frame.alpha).round() as u8,
                        );
                    }
                }
            }
        }
    }
}
