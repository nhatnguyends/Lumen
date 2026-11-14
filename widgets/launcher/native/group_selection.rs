//! Staged checkbox selection: only Done changes persisted group membership.
use super::*;
use std::collections::{HashMap, HashSet};
#[derive(Clone, PartialEq, Eq)]
pub(super) enum Hit {
    Cancel,
    Done,
    Item(String),
}
pub(super) struct State {
    pub checked: HashSet<String>,
    pub pressed: Option<Hit>,
    pub hover: Option<Hit>,
    button_hover: [crate::motion::Tween; 2],
    check_tweens: HashMap<String, crate::motion::Tween>,
    query: String,
    scroll: f32,
    selected: usize,
}
impl App {
    pub(super) unsafe fn begin_group_selection(&mut self, hwnd: HWND, id: &str) {
        if !self.dock.items.iter().any(|i| i.id == id && !i.catalog) {
            return;
        }
        self.cancel_pointer(hwnd);
        self.popup = None;
        let mut check_tweens = HashMap::new();
        let mut initial = crate::motion::Tween::fixed(0.);
        initial.target(1., self.now(), 160., [0.25, 0.1, 0.25, 1.]);
        check_tweens.insert(id.to_owned(), initial);
        self.group_selection = Some(State {
            checked: [id.to_owned()].into_iter().collect(),
            pressed: None,
            hover: None,
            button_hover: [crate::motion::Tween::fixed(0.); 2],
            check_tweens,
            query: self.query.clone(),
            scroll: self.scroll.value(self.now()),
            selected: self.selected,
        });
        self.query.clear();
        self.search_cache.get_mut().take();
        SetWindowTextW(self.edit, wide("").as_ptr());
        self.selected = self
            .visible(hwnd)
            .iter()
            .position(|index| self.dock.items[*index].id == id)
            .unwrap_or(0);
        let anchor = (self.selected / self.columns(hwnd)) as f32 * self.row_height() as f32;
        let rows = self.visible(hwnd).len().div_ceil(self.columns(hwnd));
        let max = (self.grid_top() as f32 + rows as f32 * self.row_height() as f32
            - (self.dimensions(hwnd).1 - 14) as f32
            + 28.)
            .max(0.);
        self.scroll.bounds(max, self.now());
        self.scroll.snap(anchor);
        self.keyboard_selection = false;
        ShowWindow(self.edit, SW_HIDE);
        SetFocus(hwnd);
        self.arm_motion(hwnd);
        InvalidateRect(hwnd, null(), 0);
    }
    unsafe fn group_selection_hit(&self, hwnd: HWND, p: [f32; 2]) -> Option<Hit> {
        let w = self.dimensions(hwnd).0 as f32;
        if (22. ..52.).contains(&p[1]) {
            if (w - 92. ..w - 24.).contains(&p[0]) {
                return Some(Hit::Done);
            }
            if (w - 170. ..w - 100.).contains(&p[0]) {
                return Some(Hit::Cancel);
            }
        }
        self.hit(hwnd, p[0] as i32, p[1] as i32).and_then(|pos| {
            self.visible(hwnd)
                .get(pos)
                .map(|i| Hit::Item(self.dock.items[*i].id.clone()))
        })
    }
    pub(super) unsafe fn group_selection_down(&mut self, hwnd: HWND, p: [f32; 2]) -> bool {
        if self.group_selection.is_none() {
            return false;
        }
        let hit = self.group_selection_hit(hwnd, p);
        if let Some(state) = &mut self.group_selection {
            state.pressed = hit;
        }
        SetFocus(hwnd);
        SetCapture(hwnd);
        InvalidateRect(hwnd, null(), 0);
        true
    }
    pub(super) unsafe fn group_selection_move(&mut self, hwnd: HWND, p: [f32; 2]) -> bool {
        if self.group_selection.is_none() {
            return false;
        }
        let now = self.now();
        let hit = self.group_selection_hit(hwnd, p);
        if let Some(state) = &mut self.group_selection {
            if state.hover != hit {
                state.hover = hit;
                const EASE: [f32; 4] = [0.25, 0.1, 0.25, 1.];
                let cancel_hot = state.hover == Some(Hit::Cancel);
                let done_hot = state.hover == Some(Hit::Done) && state.checked.len() >= 2;
                state.button_hover[0].target(if cancel_hot { 1. } else { 0. }, now, 180., EASE);
                state.button_hover[1].target(if done_hot { 1. } else { 0. }, now, 180., EASE);
                self.arm_motion(hwnd);
            }
        }
        false
    }
    pub(super) fn group_selection_moving(&self, now: f64) -> bool {
        self.group_selection.as_ref().is_some_and(|s| {
            s.button_hover.iter().any(|t| t.moving(now))
                || s.check_tweens.values().any(|t| t.moving(now))
        })
    }
    pub(super) unsafe fn group_selection_up(&mut self, hwnd: HWND, p: [f32; 2]) -> bool {
        if self.group_selection.is_none() {
            return false;
        }
        let hit = self.group_selection_hit(hwnd, p);
        let Some(pressed) = self.group_selection.as_mut().and_then(|s| s.pressed.take()) else {
            return false;
        };
        ReleaseCapture();
        if Some(&pressed) == hit.as_ref() {
            match pressed {
                Hit::Cancel => self.finish_group_selection(hwnd, false),
                Hit::Done => self.finish_group_selection(hwnd, true),
                Hit::Item(id) => {
                    let now = self.now();
                    if let Some(state) = &mut self.group_selection {
                        let checked = if !state.checked.remove(&id) {
                            state.checked.insert(id.clone());
                            true
                        } else {
                            false
                        };
                        state
                            .check_tweens
                            .entry(id)
                            .or_insert_with(|| crate::motion::Tween::fixed(if checked { 0. } else { 1. }))
                            .target(if checked { 1. } else { 0. }, now, 160., [0.25, 0.1, 0.25, 1.]);
                    }
                    self.arm_motion(hwnd);
                }
            }
        }
        InvalidateRect(hwnd, null(), 0);
        true
    }
    pub(super) unsafe fn finish_group_selection(&mut self, hwnd: HWND, commit: bool) {
        if commit
            && self
                .group_selection
                .as_ref()
                .is_some_and(|s| s.checked.len() < 2)
        {
            return;
        }
        let Some(state) = self.group_selection.take() else {
            return;
        };
        if commit && self.dock.group_selection(&state.checked) {
            self.save();
        }
        self.query = state.query;
        self.search_cache.get_mut().take();
        SetWindowTextW(self.edit, wide(&self.query).as_ptr());
        self.selected = state.selected;
        self.scroll
            .bounds(self.scroll.max.max(state.scroll), self.now());
        self.scroll.snap(state.scroll);
        self.layout_edit(hwnd);
        ShowWindow(
            self.edit,
            if self.dock.show_search {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        SetFocus(hwnd);
        self.arm_motion(hwnd);
        InvalidateRect(hwnd, null(), 0);
    }
    pub(super) unsafe fn group_selection_key(&mut self, hwnd: HWND, key: u32) -> bool {
        if self.group_selection.is_none() {
            return false;
        }
        match key {
            27 => self.finish_group_selection(hwnd, false),
            13 => self.finish_group_selection(hwnd, true),
            32 => {
                if let Some(index) = self.visible(hwnd).get(self.selected) {
                    let id = self.dock.items[*index].id.clone();
                    if let Some(s) = &mut self.group_selection {
                        if !s.checked.remove(&id) {
                            s.checked.insert(id);
                        }
                    }
                }
            }
            _ => return false,
        }
        InvalidateRect(hwnd, null(), 0);
        true
    }
    pub(super) unsafe fn paint_group_selection_header(&self, dc: HDC, w: i32) {
        let Some(state) = &self.group_selection else {
            return;
        };
        let c = Canvas::new(dc, self.dpi);
        let ink = self.search_ink;
        c.search_surface(
            [16., 14., w as f32 - 16., 60.],
            23.,
            self.theme_color(),
            self.dock.light,
        );
        if w >= 360 {
            c.text(
                &format!("Group apps · {} selected", state.checked.len()),
                [30., 22., w as f32 - 180., 52.],
                11.,
                true,
                ink,
                0,
            );
        }
        let now = self.paint_time;
        for (i, (hit, label, r)) in [
            (
                Hit::Cancel,
                "Cancel",
                [w as f32 - 170., 22., w as f32 - 100., 52.],
            ),
            (
                Hit::Done,
                "Done",
                [w as f32 - 92., 22., w as f32 - 24., 52.],
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let enabled = hit != Hit::Done || state.checked.len() >= 2;
            let hover = state.button_hover[i].sample(now);
            let alpha = if hit == Hit::Done && enabled {
                (220. + hover * 30.).round() as u8
            } else if enabled {
                (16. + hover * 18.).round() as u8
            } else {
                16
            };
            c.round(
                r,
                15.,
                if hit == Hit::Done && enabled {
                    self.selection_color()
                } else {
                    ink
                },
                alpha,
                false,
                false,
            );
            c.text(
                label,
                r,
                11.,
                true,
                if hit == Hit::Done && enabled {
                    0xffffff
                } else if enabled {
                    ink
                } else {
                    self.search_muted
                },
                1,
            );
        }
    }
    pub(super) unsafe fn paint_group_checkbox(&self, dc: HDC, id: &str, r: [f32; 4]) {
        let Some(state) = &self.group_selection else {
            return;
        };
        let c = Canvas::new(dc, self.dpi);
        let checked = state.checked.contains(id);
        let t = state
            .check_tweens
            .get(id)
            .map_or(if checked { 1. } else { 0. }, |tw| tw.sample(self.paint_time));
        let accent = self.selection_color();
        if t > 0.01 {
            c.round(r, 20., accent, (135. * t).round() as u8, true, false);
        }
        let b = [r[2] - 24., r[1] + 5., r[2] - 6., r[1] + 23.];
        c.round(
            b,
            6.,
            if self.dock.light {
                0xffffff
            } else {
                rgb(31, 37, 47)
            },
            235,
            false,
            false,
        );
        if t > 0.01 {
            c.round(b, 6., accent, (235. * t).round() as u8, false, false);
        }
        c.round(
            b,
            6.,
            if t > 0.5 { 0xffffff } else { self.search_ink },
            (150. - 40. * t).round() as u8,
            true,
            false,
        );
        if t > 0.01 {
            c.line(
                &[
                    (b[0] + 4., b[1] + 9.),
                    (b[0] + 8., b[1] + 13.),
                    (b[0] + 14., b[1] + 5.),
                ],
                0xffffff,
                (255. * t).round() as u8,
                1.8,
            );
        }
    }
}
