//! App-local magnetic edges and a shared liquid-glass junction/shadow surface.
//! No global hooks, screen capture, synthetic input or cross-process windows.
pub(crate) mod geometry;
pub(crate) mod material;
use crate::{compositor::Buffer, glass::Glass, model::wide};
use geometry::{components, snap_move, Pane};
use std::{
    cell::RefCell,
    collections::HashMap,
    mem::zeroed,
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::GetCapture,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};

const REFRESH: u32 = WM_APP + 110;
const SUBCLASS: usize = 0x4c554d46;
const UNFUSE_DRAG_THRESHOLD_DIP: f32 = 16.0;
#[derive(Clone, Copy)]
struct DragSession {
    anchor_rect: RECT,
    anchor_cursor: POINT,
    raw_rect: RECT,
    last_cursor: POINT,
    detached: bool,
    native_moving: bool,
}
#[derive(Clone)]
struct Member {
    pane: Pane,
    window: [i32; 4],
    visible: bool,
    moving: bool,
    resizing: bool,
    opacity: u8,
    frost: u8,
    drag: Option<DragSession>,
}
struct Bridge {
    hwnd: HWND,
    panes: Vec<Pane>,
    opacities: Vec<f32>,
    frosts: Vec<f32>,
    wallpaper: u64,
    material: u64,
    uploads: u64,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        crate::gpu::remove_surface(self.hwnd);
        unsafe {
            if IsWindow(self.hwnd) != 0 {
                DestroyWindow(self.hwnd);
            }
        }
    }
}
struct Manager {
    members: HashMap<usize, Member>,
    materials: HashMap<usize, material::Material>,
    bridges: Vec<Bridge>,
    glass: Rc<RefCell<Glass>>,
    host: HWND,
    revision: u64,
    pending: bool,
    enabled: bool,
    wallpaper: u64,
    #[cfg(test)]
    fail_surfaces: bool,
}
thread_local! {
    static MANAGER: RefCell<Option<Manager>> = const { RefCell::new(None) };
    static PENDING_MOVE: std::cell::Cell<Option<(usize, RECT)>> = const { std::cell::Cell::new(None) };
    static IN_WM_MOVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static IN_CAPTURED_MOVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(crate) fn exceeds_unfuse_threshold(dx: i32, dy: i32, dpi: f32) -> bool {
    let threshold = UNFUSE_DRAG_THRESHOLD_DIP * dpi.max(1.0);
    let dist_sq = (dx as f32) * (dx as f32) + (dy as f32) * (dy as f32);
    dist_sq >= threshold * threshold
}

pub fn in_wm_moving() -> bool {
    IN_WM_MOVING.with(|f| f.get())
}

pub unsafe fn window_rect(hwnd: HWND) -> RECT {
    if let Some((id, rect)) = PENDING_MOVE.with(|p| p.get()) {
        if id == hwnd as usize {
            return rect;
        }
    }
    let mut r: RECT = zeroed();
    GetWindowRect(hwnd, &mut r);
    r
}

pub fn pending_move_point(hwnd: HWND) -> Option<POINT> {
    PENDING_MOVE.with(|p| {
        p.get().and_then(|(id, r)| {
            (id == hwnd as usize).then_some(POINT {
                x: r.left,
                y: r.top,
            })
        })
    })
}

pub fn is_atomic_moving(hwnd: HWND) -> bool {
    PENDING_MOVE.with(|p| p.get().is_some_and(|(id, _)| id == hwnd as usize))
}

pub fn is_moving(hwnd: HWND) -> bool {
    if is_atomic_moving(hwnd) {
        return true;
    }
    MANAGER.with(|m| {
        m.try_borrow()
            .ok()
            .and_then(|s| s.as_ref()?.members.get(&(hwnd as usize)).map(|p| p.moving))
            .unwrap_or(false)
    })
}

pub unsafe fn commit_atomic_move(hwnd: HWND, target: RECT) {
    let mut cur: RECT = zeroed();
    GetWindowRect(hwnd, &mut cur);
    if target.left == cur.left && target.top == cur.top {
        return;
    }
    PENDING_MOVE.with(|p| p.set(Some((hwnd as usize, target))));
    InvalidateRect(hwnd, null(), 0);
    UpdateWindow(hwnd);
    PENDING_MOVE.with(|p| p.set(None));
    let mut after: RECT = zeroed();
    GetWindowRect(hwnd, &mut after);
    if after.left != target.left || after.top != target.top {
        SetWindowPos(
            hwnd,
            null_mut(),
            target.left,
            target.top,
            target.right - target.left,
            target.bottom - target.top,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

pub unsafe fn move_captured_widget(hwnd: HWND, r: RECT) {
    let mut target = r;
    let mut should_move = true;
    MANAGER.with(|m| {
        if let Ok(mut x) = m.try_borrow_mut() {
            if let Some(s) = x.as_mut() {
                if let Some((t, move_now)) = s.step_drag_move(hwnd, r, None) {
                    target = t;
                    should_move = move_now;
                }
            }
        }
    });
    if !should_move {
        return;
    }
    struct Guard(bool);
    impl Drop for Guard {
        fn drop(&mut self) {
            IN_CAPTURED_MOVE.with(|f| f.set(self.0));
        }
    }
    let _guard = Guard(IN_CAPTURED_MOVE.with(|f| f.replace(true)));
    if crate::gpu::is_gpu_window(hwnd) {
        SetWindowPos(
            hwnd,
            null_mut(),
            target.left,
            target.top,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        if crate::glass::move_paint_due(hwnd) {
            InvalidateRect(hwnd, null(), 0);
        }
    } else {
        commit_atomic_move(hwnd, target);
    }
}

pub unsafe fn init(glass: Rc<RefCell<Glass>>) {
    let class = wide("LumenNativeFusionSurface");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(surface_proc),
        hInstance: GetModuleHandleW(null()),
        lpszClassName: class.as_ptr(),
        ..zeroed()
    };
    RegisterClassW(&wc);
    let host = CreateWindowExW(
        0,
        class.as_ptr(),
        null(),
        0,
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        null_mut(),
        wc.hInstance,
        null(),
    );
    let wallpaper = glass.borrow().revision();
    MANAGER.with(|m| {
        *m.borrow_mut() = Some(Manager {
            members: HashMap::new(),
            materials: HashMap::new(),
            bridges: Vec::new(),
            glass,
            host,
            revision: 1,
            pending: false,
            enabled: true,
            wallpaper,
            #[cfg(test)]
            fail_surfaces: false,
        })
    });
}
pub fn shutdown() {
    MANAGER.with(|m| {
        if let Some(mut s) = m.borrow_mut().take() {
            s.bridges.clear();
            unsafe {
                DestroyWindow(s.host);
            }
        }
    });
}
pub fn enabled() -> bool {
    MANAGER.with(|m| {
        m.try_borrow()
            .ok()
            .and_then(|s| s.as_ref().map(|s| s.enabled))
            .unwrap_or(true)
    })
}
pub fn set_enabled(value: bool) {
    MANAGER.with(|m| {
        if let Ok(mut s) = m.try_borrow_mut() {
            if let Some(s) = s.as_mut() {
                if s.enabled != value {
                    s.enabled = value;
                    unsafe {
                        s.changed();
                    }
                }
            }
        }
    });
}

unsafe fn is_widget(hwnd: HWND) -> bool {
    if !GetPropW(hwnd, wide("LumenSourceWidgetPackage").as_ptr()).is_null() {
        return true;
    }
    let mut name = [0u16; 80];
    let n = GetClassNameW(hwnd, name.as_mut_ptr(), 80).max(0) as usize;
    matches!(
        String::from_utf16_lossy(&name[..n]).as_str(),
        "LumenNativeDockWindow"
            | "LumenNativeClockWindow"
            | "LumenNativeCalendarWindow"
            | "LumenNativeMusic"
            | "LumenNativeNotesBoard"
    )
}
/// The registry marks each package's main HWND; child/settings windows are excluded.
pub unsafe fn register_widget(hwnd: HWND) {
    if !hwnd.is_null() && IsWindow(hwnd) != 0 {
        SetPropW(
            hwnd,
            wide("LumenSourceWidgetPackage").as_ptr(),
            1usize as HANDLE,
        );
    }
}
unsafe fn read_member(hwnd: HWND, radius: f32) -> Member {
    let r = window_rect(hwnd);
    let dpi = crate::geometry::window_dpi(hwnd).max(1.);
    let inset = 4. * dpi;
    Member {
        pane: Pane {
            id: hwnd as usize,
            rect: [
                r.left as f32 + inset,
                r.top as f32 + inset,
                r.right as f32 - inset,
                r.bottom as f32 - inset,
            ],
            radius: radius * dpi,
            dpi,
            topmost: GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0,
        },
        window: [r.left, r.top, r.right, r.bottom],
        visible: IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) == 0,
        moving: false,
        resizing: false,
        opacity: 255,
        frost: 255,
        drag: None,
    }
}
impl Manager {
    fn material_key(&self, panes: &[Pane]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        for pane in panes {
            pane.id.hash(&mut hash);
            self.materials.get(&pane.id).hash(&mut hash);
        }
        hash.finish()
    }
    fn material_at(&self, panes: &[Pane], x: f32, y: f32) -> Option<[f32; 3]> {
        material::blend(
            panes.iter().filter_map(|p| {
                Some((
                    p,
                    self.members.get(&p.id)?.window,
                    self.materials.get(&p.id)?,
                ))
            }),
            x,
            y,
        )
    }
    fn affine_at(&self, panes: &[Pane], x: f32, y: f32) -> Option<(f32, [f32; 3])> {
        material::blend_affine(
            panes.iter().filter_map(|p| {
                Some((
                    p,
                    self.members.get(&p.id)?.window,
                    self.materials.get(&p.id)?,
                ))
            }),
            x,
            y,
        )
    }
    fn delta_at(&self, panes: &[Pane], x: f32, y: f32) -> Option<[f32; 3]> {
        self.affine_at(panes, x, y).map(|(_, b)| b)
    }
    fn overlay_at(&self, panes: &[Pane], x: f32, y: f32) -> Option<[f32; 4]> {
        material::blend_overlay(
            panes.iter().filter_map(|p| {
                Some((
                    p,
                    self.members.get(&p.id)?.window,
                    self.materials.get(&p.id)?,
                ))
            }),
            x,
            y,
        )
    }
    fn frost_at(&self, panes: &[Pane], x: f32, y: f32) -> f32 {
        material::blend_scalar(
            panes.iter().map(|p| {
                (
                    p,
                    self.members.get(&p.id).map_or(255, |m| m.frost) as f32 / 255.0,
                )
            }),
            x,
            y,
        )
        .unwrap_or(1.0)
    }
    unsafe fn changed(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        if !self.pending {
            self.pending = true;
            PostMessageW(self.host, REFRESH, 0, 0);
        }
    }
    fn panes(&self) -> Vec<Pane> {
        if !self.enabled {
            return Vec::new();
        }
        let mut ps: Vec<_> = self
            .members
            .values()
            .filter(|p| p.visible && !p.moving)
            .map(|p| p.pane)
            .collect();
        ps.sort_by_key(|p| p.id);
        ps
    }
    fn group(&self, id: usize) -> Option<Vec<Pane>> {
        let desired = components(&self.panes())
            .into_iter()
            .find(|g| g.len() > 1 && g.iter().any(|p| p.id == id))?;
        // Coverage and shadow suppression only commit AFTER a successful
        // junction upload. Allocation/size failures retain standalone chrome.
        self.bridges
            .iter()
            .any(|b| b.panes == desired)
            .then_some(desired)
    }
    unsafe fn update(&mut self, hwnd: HWND, radius: Option<f32>) {
        let wallpaper = self.glass.borrow().revision();
        if self.wallpaper != wallpaper {
            self.wallpaper = wallpaper;
            self.changed();
        }
        let id = hwnd as usize;
        if let Some(old) = self.members.get(&id).cloned() {
            let mut new = read_member(hwnd, radius.unwrap_or(old.pane.radius / old.pane.dpi));
            new.moving = old.moving;
            new.resizing = old.resizing;
            new.opacity = old.opacity;
            new.frost = old.frost;
            new.drag = old.drag;
            let changed = old.pane != new.pane || old.visible != new.visible;
            self.members.insert(id, new);
            if changed && !old.moving {
                self.materials.remove(&id);
                self.changed();
            }
        } else if is_widget(hwnd) {
            self.members
                .insert(id, read_member(hwnd, radius.unwrap_or(34.)));
            SetWindowSubclass(hwnd, Some(widget_proc), SUBCLASS, 0);
            self.changed();
        }
    }
    unsafe fn begin_resize(&mut self, hwnd: HWND, cursor: POINT) {
        let was_fused = self.enabled && self.group(hwnd as usize).is_some();
        let anchor_rect = window_rect(hwnd);
        if let Some(p) = self.members.get_mut(&(hwnd as usize)) {
            p.resizing = true;
            p.drag = Some(DragSession {
                anchor_rect,
                anchor_cursor: cursor,
                raw_rect: anchor_rect,
                last_cursor: cursor,
                detached: !was_fused,
                native_moving: false,
            });
            if !was_fused && !p.moving {
                p.moving = true;
                self.changed();
            }
        }
    }
    unsafe fn begin_move_drag(&mut self, hwnd: HWND, cursor: POINT) {
        let was_fused = self.enabled && self.group(hwnd as usize).is_some();
        let anchor_rect = window_rect(hwnd);
        if let Some(p) = self.members.get_mut(&(hwnd as usize)) {
            if p.resizing {
                return;
            }
            p.drag = Some(DragSession {
                anchor_rect,
                anchor_cursor: cursor,
                raw_rect: anchor_rect,
                last_cursor: cursor,
                detached: !was_fused,
                native_moving: true,
            });
            if !was_fused && !p.moving {
                p.moving = true;
                self.changed();
            }
        }
    }
    unsafe fn step_drag_move(
        &mut self,
        hwnd: HWND,
        proposed: RECT,
        cursor: Option<POINT>,
    ) -> Option<(RECT, bool)> {
        let was_fused = self.enabled && self.group(hwnd as usize).is_some();
        let panes = self.panes();
        let id = hwnd as usize;
        let Some(p) = self.members.get_mut(&id) else {
            return Some((proposed, true));
        };
        let dpi = p.pane.dpi;
        let pane_template = p.pane;
        let mut need_changed = false;
        let cur = cursor.unwrap_or(POINT { x: 0, y: 0 });
        let drag = p.drag.get_or_insert_with(|| {
            let anchor_rect = RECT {
                left: p.window[0],
                top: p.window[1],
                right: p.window[2],
                bottom: p.window[3],
            };
            if !was_fused && !p.moving {
                p.moving = true;
                need_changed = true;
            }
            DragSession {
                anchor_rect,
                anchor_cursor: cur,
                raw_rect: proposed,
                last_cursor: cur,
                detached: !was_fused,
                native_moving: cursor.is_some(),
            }
        });
        if let Some(cur) = cursor {
            if cur.x != drag.last_cursor.x || cur.y != drag.last_cursor.y {
                let dx = cur.x - drag.last_cursor.x;
                let dy = cur.y - drag.last_cursor.y;
                drag.raw_rect.left += dx;
                drag.raw_rect.right += dx;
                drag.raw_rect.top += dy;
                drag.raw_rect.bottom += dy;
                drag.last_cursor = cur;
            } else if cur.x == drag.anchor_cursor.x && cur.y == drag.anchor_cursor.y {
                // Real cursor did not move (e.g. synthetic WM_MOVING in tests).
                drag.raw_rect = proposed;
            }
        } else {
            drag.raw_rect = proposed;
        }
        if !drag.detached {
            let dx = drag.raw_rect.left - drag.anchor_rect.left;
            let dy = drag.raw_rect.top - drag.anchor_rect.top;
            if !exceeds_unfuse_threshold(dx, dy, dpi) {
                let anchor = drag.anchor_rect;
                if need_changed {
                    self.changed();
                }
                return Some((anchor, false));
            }
            drag.detached = true;
            if !p.moving {
                p.moving = true;
                need_changed = true;
            }
        } else if !p.moving {
            p.moving = true;
            need_changed = true;
        }
        let mut target = drag.raw_rect;
        if self.enabled {
            let inset = 4. * dpi;
            let candidate = Pane {
                rect: [
                    target.left as f32 + inset,
                    target.top as f32 + inset,
                    target.right as f32 - inset,
                    target.bottom as f32 - inset,
                ],
                ..pane_template
            };
            let d = snap_move(&candidate, &panes);
            let sx = d[0].round() as i32;
            let sy = d[1].round() as i32;
            target.left += sx;
            target.right += sx;
            target.top += sy;
            target.bottom += sy;
        }
        if need_changed {
            self.changed();
        }
        Some((target, true))
    }
    unsafe fn step_drag_resize(
        &mut self,
        hwnd: HWND,
        r: &mut RECT,
        edge: u32,
        minimum: [i32; 2],
        cursor: POINT,
    ) -> bool {
        let active = [
            matches!(edge, HTLEFT | HTTOPLEFT | HTBOTTOMLEFT),
            matches!(edge, HTTOP | HTTOPLEFT | HTTOPRIGHT),
            matches!(edge, HTRIGHT | HTTOPRIGHT | HTBOTTOMRIGHT),
            matches!(edge, HTBOTTOM | HTBOTTOMLEFT | HTBOTTOMRIGHT),
        ];
        let mut need_changed = false;
        let pane_template;
        let dpi;
        {
            let Some(p) = self.members.get_mut(&(hwnd as usize)) else {
                return true;
            };
            pane_template = p.pane;
            dpi = p.pane.dpi;
            if p.resizing {
                if let Some(drag) = p.drag.as_mut() {
                    if !drag.detached {
                        let cur_dx = cursor.x - drag.anchor_cursor.x;
                        let cur_dy = cursor.y - drag.anchor_cursor.y;
                        let (dx, dy) = if cur_dx != 0 || cur_dy != 0 {
                            (cur_dx, cur_dy)
                        } else {
                            let edge_dx = if active[0] {
                                r.left - drag.anchor_rect.left
                            } else if active[2] {
                                r.right - drag.anchor_rect.right
                            } else {
                                0
                            };
                            let edge_dy = if active[1] {
                                r.top - drag.anchor_rect.top
                            } else if active[3] {
                                r.bottom - drag.anchor_rect.bottom
                            } else {
                                0
                            };
                            (edge_dx, edge_dy)
                        };
                        if !exceeds_unfuse_threshold(dx, dy, dpi) {
                            *r = drag.anchor_rect;
                            return false;
                        }
                        drag.detached = true;
                        if !p.moving {
                            p.moving = true;
                            need_changed = true;
                        }
                    }
                }
            }
        }
        if need_changed {
            self.changed();
        }
        if !self.enabled {
            return true;
        }
        let inset = 4. * dpi;
        let candidate = Pane {
            rect: [
                r.left as f32 + inset,
                r.top as f32 + inset,
                r.right as f32 - inset,
                r.bottom as f32 - inset,
            ],
            ..pane_template
        };
        let snapped = geometry::snap_resize(
            &candidate,
            &self.panes(),
            active,
            [
                minimum[0] as f32 - 2. * inset,
                minimum[1] as f32 - 2. * inset,
            ],
        );
        let values = [
            (snapped[0] - inset).round() as i32,
            (snapped[1] - inset).round() as i32,
            (snapped[2] + inset).round() as i32,
            (snapped[3] + inset).round() as i32,
        ];
        if values[2] - values[0] >= minimum[0] {
            if active[0] {
                r.left = values[0];
            }
            if active[2] {
                r.right = values[2];
            }
        }
        if values[3] - values[1] >= minimum[1] {
            if active[1] {
                r.top = values[1];
            }
            if active[3] {
                r.bottom = values[3];
            }
        }
        true
    }
    unsafe fn end_drag(&mut self, hwnd: HWND) {
        if let Some(p) = self.members.get_mut(&(hwnd as usize)) {
            p.drag = None;
            p.resizing = false;
            if p.moving {
                p.moving = false;
                self.materials.remove(&(hwnd as usize));
                self.update(hwnd, None);
                self.changed();
            }
        }
    }
}
/// Registers only known app-owned main widgets, never editor/settings/player HWNDs.
pub fn revision(hwnd: HWND) -> u64 {
    use std::hash::{Hash, Hasher};
    MANAGER.with(|m| {
        let Ok(mut x) = m.try_borrow_mut() else {
            return 0;
        };
        let Some(s) = x.as_mut() else { return 0 };
        unsafe {
            s.update(hwnd, None);
        }
        let Some(own) = s.members.get(&(hwnd as usize)) else {
            return 0;
        };
        let group = s.group(hwnd as usize);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        group.is_some().hash(&mut h);
        if let Some(ref panes) = group {
            s.material_key(panes).hash(&mut h);
        }
        for p in group.unwrap_or_else(|| vec![own.pane]) {
            p.id.hash(&mut h);
            for v in p.rect {
                v.to_bits().hash(&mut h);
            }
            p.radius.to_bits().hash(&mut h);
            p.dpi.to_bits().hash(&mut h);
            p.topmost.hash(&mut h);
            s.members.get(&p.id).map_or(255u8, |m| m.frost).hash(&mut h);
        }
        h.finish()
    })
}
pub unsafe fn prepare(hwnd: HWND, radius: f32) {
    MANAGER.with(|m| {
        if let Ok(mut x) = m.try_borrow_mut() {
            if let Some(s) = x.as_mut() {
                s.update(hwnd, Some(radius));
            }
        }
    });
}
pub fn pane_radius(hwnd: HWND) -> Option<f32> {
    MANAGER.with(|m| {
        m.try_borrow().ok().and_then(|s| {
            let p = &s.as_ref()?.members.get(&(hwnd as usize))?.pane;
            Some(p.radius / p.dpi.max(1.0))
        })
    })
}

pub fn frosted_wallpaper_active() -> bool {
    if !crate::glass::frosted_glass() {
        return false;
    }
    MANAGER.with(|m| {
        m.try_borrow()
            .ok()
            .and_then(|s| s.as_ref()?.glass.try_borrow().ok().map(|g| g.available()))
            .unwrap_or(false)
    })
}

/// Publish raw chrome or wallpaper-subtracted chrome delta for frosted widgets.
pub unsafe fn publish_chrome_for(
    hwnd: HWND,
    width: usize,
    height: usize,
    source_bgra: &[u8],
) {
    if width == 0 || height == 0 || source_bgra.len() < width * height * 4 {
        return;
    }
    let info = MANAGER.with(|m| {
        let Ok(guard) = m.try_borrow() else {
            return None;
        };
        let s = guard.as_ref()?;
        let member = s.members.get(&(hwnd as usize))?;
        let panes = s.group(hwnd as usize);
        Some((s.glass.clone(), member.pane.dpi, member.window, panes))
    });
    let Some((glass, dpi, win_rect, fused_panes)) = info else {
        return;
    };
    let (tw, th) = (width.min(64), height.min(64));
    let material = if frosted_wallpaper_active() {
        let win = window_rect(hwnd);
        let rim_mask = if crate::glass::liquid_glass() {
            let mut mask = vec![false; width * height];
            if let Some(ref panes) = fused_panes {
                let rim_band = 2.0 * dpi.max(1.0);
                geometry::with_outline(panes, |outline| {
                    for y in 0..height {
                        let gy = win_rect[1] as f32 + y as f32 + 0.5;
                        for x in 0..width {
                            let gx = win_rect[0] as f32 + x as f32 + 0.5;
                            if outline.distance([gx, gy]) > -rim_band {
                                mask[y * width + x] = true;
                            }
                        }
                    }
                });
            } else {
                let rim_band = 4.0 * dpi.max(1.0);
                for y in 0..height {
                    let dy = y.min(height - 1 - y) as f32;
                    for x in 0..width {
                        let dx = x.min(width - 1 - x) as f32;
                        if dx.min(dy) < rim_band {
                            mask[y * width + x] = true;
                        }
                    }
                }
            }
            Some(mask)
        } else {
            None
        };
        glass
            .try_borrow()
            .ok()
            .and_then(|g| g.sample_blurred_tile(hwnd, win, tw, th))
            .map(|tile| {
                material::Material::from_bgra_delta(
                    width,
                    height,
                    source_bgra,
                    &tile,
                    tw,
                    th,
                    rim_mask.as_deref(),
                )
            })
            .unwrap_or_else(|| material::Material::from_bgra(width, height, source_bgra))
    } else {
        material::Material::from_bgra(width, height, source_bgra)
    };
    store_material(hwnd, material, true);
}

unsafe fn store_material(hwnd: HWND, material: material::Material, only_if_grouped: bool) {
    MANAGER.with(|manager| {
        let Ok(mut manager) = manager.try_borrow_mut() else {
            return;
        };
        let Some(s) = manager.as_mut() else { return };
        if !s.members.contains_key(&(hwnd as usize)) {
            return;
        }
        if s.materials.get(&(hwnd as usize)) == Some(&material) {
            return;
        }
        s.materials.insert(hwnd as usize, material);
        if !only_if_grouped || s.group(hwnd as usize).is_some() {
            s.changed();
        }
    });
}

/// Publish premultiplied chrome-only overlay (without wallpaper) for fused widgets.
pub unsafe fn publish_overlay_for(
    hwnd: HWND,
    width: usize,
    height: usize,
    overlay_bgra: &[u8],
    inset_px: usize,
) {
    if width == 0 || height == 0 || overlay_bgra.len() < width * height * 4 {
        return;
    }
    let material = material::Material::from_overlay_bgra(width, height, overlay_bgra, inset_px);
    store_material(hwnd, material, true);
}

/// Publish only raw chrome, before shared blending, bevel, alpha and content.
/// Equality suppresses repaint feedback and keeps storage below12 KiB/member.
#[allow(dead_code)]
pub unsafe fn publish_material(hwnd: HWND, material: material::Material) {
    MANAGER.with(|manager| {
        let Ok(mut manager) = manager.try_borrow_mut() else {
            return;
        };
        let Some(s) = manager.as_mut() else { return };
        if !s.members.contains_key(&(hwnd as usize)) {
            return;
        }
        if s.materials.get(&(hwnd as usize)) == Some(&material) {
            return;
        }
        s.materials.insert(hwnd as usize, material);
        s.changed();
    });
}
pub fn may_fuse() -> bool {
    MANAGER.with(|m| {
        m.try_borrow()
            .ok()
            .and_then(|x| x.as_ref().map(|s| s.enabled && s.members.len() >= 2))
            .unwrap_or(false)
    })
}
pub fn group(hwnd: HWND) -> Option<Vec<Pane>> {
    MANAGER.with(|m| {
        m.try_borrow()
            .ok()
            .and_then(|s| s.as_ref().and_then(|s| s.group(hwnd as usize)))
    })
}
pub fn group_opacities(hwnd: HWND, own_alpha: u8) -> Option<(Vec<Pane>, Vec<u8>)> {
    MANAGER.with(|m| {
        let s = m.try_borrow().ok()?;
        let s = s.as_ref()?;
        let panes = s.group(hwnd as usize)?;
        let opacities: Vec<u8> = panes
            .iter()
            .map(|p| {
                if p.id == hwnd as usize {
                    own_alpha
                } else {
                    s.members.get(&p.id).map_or(255, |m| m.opacity)
                }
            })
            .collect();
        if opacities.iter().all(|&o| o == own_alpha) {
            None
        } else {
            Some((panes, opacities))
        }
    })
}
pub fn group_frosts(hwnd: HWND, own_frost: u8) -> Option<(Vec<Pane>, Vec<u8>)> {
    MANAGER.with(|m| {
        let s = m.try_borrow().ok()?;
        let s = s.as_ref()?;
        let panes = s.group(hwnd as usize)?;
        let frosts: Vec<u8> = panes
            .iter()
            .map(|p| {
                if p.id == hwnd as usize {
                    own_frost
                } else {
                    s.members.get(&p.id).map_or(255, |m| m.frost)
                }
            })
            .collect();
        if frosts.iter().all(|&f| f == own_frost) {
            None
        } else {
            Some((panes, frosts))
        }
    })
}
pub fn fused(hwnd: HWND) -> bool {
    group(hwnd).is_some()
}
pub fn is_surface(hwnd: HWND) -> bool {
    MANAGER.with(|m| {
        m.try_borrow().ok().is_some_and(|m| {
            m.as_ref()
                .is_some_and(|s| s.bridges.iter().any(|b| b.hwnd == hwnd))
        })
    })
}
unsafe fn restack(hwnd: HWND, panes: &[Pane], rect: Option<[i32; 4]>, show: bool) {
    crate::desktop_layer::register(
        hwnd,
        panes
            .iter()
            .any(|pane| crate::desktop_layer::contains(pane.id as HWND)),
    );
    let mut lowest = null_mut();
    let mut cur = GetTopWindow(null_mut());
    for _ in 0..4096 {
        if cur.is_null() {
            break;
        }
        if panes.iter().any(|p| p.id == cur as usize) {
            lowest = cur;
        }
        cur = GetWindow(cur, GW_HWNDNEXT);
    }
    if lowest.is_null() {
        lowest = panes.last().map_or(null_mut(), |p| p.id as HWND);
    }
    let topmost = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize != 0;
    if topmost != panes[0].topmost {
        SetWindowPos(
            hwnd,
            if panes[0].topmost {
                HWND_TOPMOST
            } else {
                HWND_NOTOPMOST
            },
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
    let [x, y, w, h] = rect.unwrap_or([0, 0, 0, 0]);
    // The unowned decorative surface stays below the lowest cluster member,
    // including its desktop band. Never float a junction over widgets, other apps,
    // or General Settings just because it was repainted or shown last.
    SetWindowPos(
        hwnd,
        lowest,
        x,
        y,
        w,
        h,
        SWP_NOACTIVATE
            | SWP_NOOWNERZORDER
            | if show { SWP_SHOWWINDOW } else { 0 }
            | if rect.is_none() {
                SWP_NOMOVE | SWP_NOSIZE
            } else {
                0
            },
    );
}
pub fn opacity(hwnd: HWND, value: u8) {
    MANAGER.with(|m| {
        if let Ok(mut x) = m.try_borrow_mut() {
            if let Some(s) = x.as_mut() {
                if let Some(p) = s.members.get_mut(&(hwnd as usize)) {
                    if p.opacity != value {
                        p.opacity = value;
                        unsafe {
                            s.changed();
                        }
                    }
                }
            }
        }
    });
}
pub fn frost_strength(hwnd: HWND, value: u8) {
    MANAGER.with(|m| {
        if let Ok(mut x) = m.try_borrow_mut() {
            if let Some(s) = x.as_mut() {
                if let Some(p) = s.members.get_mut(&(hwnd as usize)) {
                    if p.frost != value {
                        p.frost = value;
                        unsafe {
                            s.changed();
                        }
                    }
                }
            }
        }
    });
}
/// Shared by native WM_SIZING and app-local captured resize handlers. The
/// supplied limits belong to the widget, not its detached settings window.
/// Returns `false` while a fused widget is still held below the unfuse drag threshold.
pub unsafe fn snap_resize_rect(hwnd: HWND, r: &mut RECT, edge: u32, minimum: [i32; 2]) -> bool {
    let mut cursor: POINT = zeroed();
    GetCursorPos(&mut cursor);
    MANAGER.with(|manager| {
        let Ok(mut manager) = manager.try_borrow_mut() else {
            return true;
        };
        let Some(manager) = manager.as_mut() else {
            return true;
        };
        manager.step_drag_resize(hwnd, r, edge, minimum, cursor)
    })
}
/// Both sides sample the same mix of their actual unblended chrome colors.
pub unsafe fn shared_material(
    hwnd: HWND,
    width: i32,
    height: i32,
    _dpi: f32,
) -> Option<material::Field> {
    MANAGER.with(|m| {
        let s = m.try_borrow().ok()?;
        let s = s.as_ref()?;
        let panes = s.group(hwnd as usize)?;
        if panes
            .iter()
            .filter(|p| s.materials.contains_key(&p.id))
            .count()
            < 2
        {
            return None;
        }
        let has_opaque = panes
            .iter()
            .any(|p| s.materials.get(&p.id).is_some_and(|m| m.is_opaque()));
        let r = window_rect(hwnd);
        // Global pixel grid: adjacent widgets interpolate identical samples at
        // their seam. Only this small field evaluates distance/exponential;
        // rasterization uses bilinear samples inside the join band.
        let step = 16;
        let l = r.left.div_euclid(step) * step - step;
        let t = r.top.div_euclid(step) * step - step;
        let gw = ((r.left + width - l + step - 1) / step + 1) as usize;
        let gh = ((r.top + height - t + step - 1) / step + 1) as usize;
        let mut bgr = Vec::with_capacity(gw * gh);
        if has_opaque {
            for y in 0..gh {
                for x in 0..gw {
                    let color = s.material_at(
                        &panes,
                        (l + x as i32 * step) as f32 + 8.,
                        (t + y as i32 * step) as f32 + 8.,
                    )?;
                    bgr.push(color);
                }
            }
            Some(material::Field {
                window: [l, t, l + gw as i32 * step, t + gh as i32 * step],
                width: gw,
                height: gh,
                is_delta: false,
                bgr,
            })
        } else {
            let own_mat = s.materials.get(&(hwnd as usize))?;
            let own_win = s
                .members
                .get(&(hwnd as usize))
                .map_or([r.left, r.top, r.right, r.bottom], |m| m.window);
            let own_gain = if own_mat.is_delta {
                own_mat.gain()
            } else {
                1.0
            };
            let field_rect = RECT {
                left: l,
                top: t,
                right: l + gw as i32 * step,
                bottom: t + gh as i32 * step,
            };
            let wall_tile = s
                .glass
                .try_borrow()
                .ok()
                .and_then(|g| g.sample_blurred_tile(hwnd, field_rect, gw, gh));
            for y in 0..gh {
                for x in 0..gw {
                    let gx = (l + x as i32 * step) as f32 + 8.;
                    let gy = (t + y as i32 * step) as f32 + 8.;
                    let (blended_gain, blended_bias) = s.affine_at(&panes, gx, gy)?;
                    let ob = own_mat.sample_delta(own_win, gx, gy);
                    let wb = wall_tile
                        .as_ref()
                        .and_then(|t| t.get(y * gw + x).copied())
                        .unwrap_or([128, 128, 128]);
                    bgr.push(std::array::from_fn(|c| {
                        (blended_gain - own_gain) * wb[c] as f32 + (blended_bias[c] - ob[c])
                    }));
                }
            }
            Some(material::Field {
                window: [l, t, l + gw as i32 * step, t + gh as i32 * step],
                width: gw,
                height: gh,
                is_delta: true,
                bgr,
            })
        }
    })
}

pub unsafe fn shared_overlay(
    hwnd: HWND,
    width: i32,
    height: i32,
) -> Option<material::OverlayField> {
    if width <= 0 || height <= 0 {
        return None;
    }
    MANAGER.with(|m| {
        let s = m.try_borrow().ok()?;
        let s = s.as_ref()?;
        let panes = s.group(hwnd as usize)?;
        if panes
            .iter()
            .filter(|p| s.materials.contains_key(&p.id))
            .count()
            < 2
        {
            return None;
        }
        if !panes
            .iter()
            .any(|p| s.materials.get(&p.id).is_some_and(|m| m.is_overlay))
        {
            return None;
        }
        let r = window_rect(hwnd);
        let step = 16;
        let l = r.left.div_euclid(step) * step - step;
        let t = r.top.div_euclid(step) * step - step;
        let gw = ((r.left + width - l + step - 1) / step + 1) as usize;
        let gh = ((r.top + height - t + step - 1) / step + 1) as usize;
        let mut bgra = Vec::with_capacity(gw * gh);
        for y in 0..gh {
            for x in 0..gw {
                let color = s.overlay_at(
                    &panes,
                    (l + x as i32 * step) as f32 + 8.,
                    (t + y as i32 * step) as f32 + 8.,
                )?;
                bgra.push(color);
            }
        }
        Some(material::OverlayField {
            window: [l, t, l + gw as i32 * step, t + gh as i32 * step],
            width: gw,
            height: gh,
            bgra,
        })
    })
}

pub struct FrostBlend {
    origin: [f32; 2],
    own: f32,
    field: material::FrostField,
    seams: Vec<crate::compositor::Seam>,
    bounds: [i32; 4],
}
impl FrostBlend {
    pub fn sample_local(&self, x: i32, y: i32) -> f32 {
        if x < self.bounds[0]
            || x >= self.bounds[2]
            || y < self.bounds[1]
            || y >= self.bounds[3]
        {
            return self.own;
        }
        let gx = self.origin[0] + x as f32 + 0.5;
        let gy = self.origin[1] + y as f32 + 0.5;
        let w = crate::compositor::seam_weight(&self.seams, x, y, gx, gy);
        if w <= 0.0 {
            self.own
        } else {
            (self.own * (1.0 - w) + self.field.sample(gx, gy) * w).clamp(0.0, 1.0)
        }
    }
}

pub unsafe fn shared_frost(
    hwnd: HWND,
    width: i32,
    height: i32,
    own_frost: u8,
) -> Option<FrostBlend> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let (panes, frosts) = group_frosts(hwnd, own_frost)?;
    let own = panes.iter().find(|p| p.id == hwnd as usize)?;
    let r = window_rect(hwnd);
    let seams = crate::compositor::collect_seams(&panes, own, r, width, height);
    if seams.is_empty() {
        return None;
    }
    let bounds = crate::compositor::seams_bounds(&seams);
    let step = 16;
    let l = r.left.div_euclid(step) * step - step;
    let t = r.top.div_euclid(step) * step - step;
    let gw = ((r.left + width - l + step - 1) / step + 1) as usize;
    let gh = ((r.top + height - t + step - 1) / step + 1) as usize;
    let mut values = Vec::with_capacity(gw * gh);
    for y in 0..gh {
        let gy = (t + y as i32 * step) as f32 + 8.;
        for x in 0..gw {
            let gx = (l + x as i32 * step) as f32 + 8.;
            let v = material::blend_scalar(
                panes
                    .iter()
                    .zip(&frosts)
                    .map(|(p, &f)| (p, f as f32 / 255.0)),
                gx,
                gy,
            )
            .unwrap_or(1.0);
            values.push(v);
        }
    }
    Some(FrostBlend {
        origin: [r.left as f32, r.top as f32],
        own: own_frost as f32 / 255.0,
        field: material::FrostField {
            window: [l, t, l + gw as i32 * step, t + gh as i32 * step],
            width: gw,
            height: gh,
            values,
        },
        seams,
        bounds,
    })
}

unsafe extern "system" fn widget_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    match msg {
        WM_SIZING => {
            let edge = match wp as u32 {
                WMSZ_LEFT => HTLEFT,
                WMSZ_RIGHT => HTRIGHT,
                WMSZ_TOP => HTTOP,
                WMSZ_TOPLEFT => HTTOPLEFT,
                WMSZ_TOPRIGHT => HTTOPRIGHT,
                WMSZ_BOTTOM => HTBOTTOM,
                WMSZ_BOTTOMLEFT => HTBOTTOMLEFT,
                WMSZ_BOTTOMRIGHT => HTBOTTOMRIGHT,
                _ => HTCLIENT,
            };
            let mut limits: MINMAXINFO = zeroed();
            SendMessageW(hwnd, WM_GETMINMAXINFO, 0, &mut limits as *mut _ as isize);
            snap_resize_rect(
                hwnd,
                &mut *(lp as *mut RECT),
                edge,
                [
                    limits.ptMinTrackSize.x.max(1),
                    limits.ptMinTrackSize.y.max(1),
                ],
            );
        }
        WM_NCLBUTTONDOWN => {
            if (HTLEFT..=HTBOTTOMRIGHT).contains(&(wp as u32)) {
                let mut cursor: POINT = zeroed();
                GetCursorPos(&mut cursor);
                MANAGER.with(|m| {
                    if let Ok(mut x) = m.try_borrow_mut() {
                        if let Some(s) = x.as_mut() {
                            s.begin_resize(hwnd, cursor);
                        }
                    }
                });
            }
        }
        WM_ENTERSIZEMOVE => {
            let mut cursor: POINT = zeroed();
            GetCursorPos(&mut cursor);
            MANAGER.with(|m| {
                if let Ok(mut x) = m.try_borrow_mut() {
                    if let Some(s) = x.as_mut() {
                        s.begin_move_drag(hwnd, cursor);
                    }
                }
            });
        }
        WM_WINDOWPOSCHANGING => {
            // Forward any capture-based SetWindowPos moves into the same
            // shared drag threshold and snapping path.
            if GetCapture() == hwnd
                && !is_atomic_moving(hwnd)
                && !IN_CAPTURED_MOVE.with(|f| f.get())
            {
                let pos = &mut *(lp as *mut WINDOWPOS);
                if pos.flags & SWP_NOMOVE == 0 {
                    MANAGER.with(|m| {
                        if let Ok(mut x) = m.try_borrow_mut() {
                            if let Some(s) = x.as_mut() {
                                if let Some(p) = s.members.get(&(hwnd as usize)).cloned() {
                                    if p.drag.is_some_and(|d| d.native_moving) {
                                        return;
                                    }
                                    let w = p.window[2] - p.window[0];
                                    let h = p.window[3] - p.window[1];
                                    let moving =
                                        pos.flags & SWP_NOSIZE != 0 || (pos.cx == w && pos.cy == h);
                                    if moving && (pos.x != p.window[0] || pos.y != p.window[1]) {
                                        let proposed = RECT {
                                            left: pos.x,
                                            top: pos.y,
                                            right: pos.x + w,
                                            bottom: pos.y + h,
                                        };
                                        if let Some((target, _)) =
                                            s.step_drag_move(hwnd, proposed, None)
                                        {
                                            pos.x = target.left;
                                            pos.y = target.top;
                                        }
                                    }
                                }
                            }
                        }
                    });
                }
            }
        }
        WM_MOVING => {
            let r = &mut *(lp as *mut RECT);
            let mut target = *r;
            let mut should_move = true;
            let mut cursor: POINT = zeroed();
            GetCursorPos(&mut cursor);
            MANAGER.with(|m| {
                if let Ok(mut x) = m.try_borrow_mut() {
                    if let Some(s) = x.as_mut() {
                        if let Some((t, move_now)) = s.step_drag_move(hwnd, *r, Some(cursor)) {
                            target = t;
                            should_move = move_now;
                        }
                    }
                }
            });
            *r = target;
            if should_move {
                if crate::gpu::is_gpu_window(hwnd) {
                    if crate::glass::move_paint_due(hwnd) {
                        IN_WM_MOVING.with(|f| f.set(true));
                        PENDING_MOVE.with(|p| p.set(Some((hwnd as usize, target))));
                        InvalidateRect(hwnd, null(), 0);
                        UpdateWindow(hwnd);
                        PENDING_MOVE.with(|p| p.set(None));
                        IN_WM_MOVING.with(|f| f.set(false));
                    }
                } else {
                    commit_atomic_move(hwnd, target);
                }
            }
        }
        WM_EXITSIZEMOVE | WM_CAPTURECHANGED | WM_CANCELMODE => {
            MANAGER.with(|m| {
                if let Ok(mut x) = m.try_borrow_mut() {
                    if let Some(s) = x.as_mut() {
                        s.end_drag(hwnd);
                    }
                }
            });
        }
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(widget_proc), SUBCLASS);
            MANAGER.with(|m| {
                if let Ok(mut x) = m.try_borrow_mut() {
                    if let Some(s) = x.as_mut() {
                        s.members.remove(&(hwnd as usize));
                        s.materials.remove(&(hwnd as usize));
                        s.changed();
                    }
                }
            });
        }
        _ => {}
    }
    let result = DefSubclassProc(hwnd, msg, wp, lp);
    if matches!(msg, WM_WINDOWPOSCHANGED | WM_SHOWWINDOW | WM_DPICHANGED) {
        MANAGER.with(|m| {
            if let Ok(mut x) = m.try_borrow_mut() {
                if let Some(s) = x.as_mut() {
                    s.update(hwnd, None);
                    if msg == WM_WINDOWPOSCHANGED
                        && (*(lp as *const WINDOWPOS)).flags & SWP_NOZORDER == 0
                    {
                        s.changed();
                    }
                }
            }
        });
    }
    result
}
unsafe extern "system" fn surface_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        REFRESH => {
            refresh();
            0
        }
        WM_NCHITTEST => HTTRANSPARENT as isize,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_ERASEBKGND => 1,
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
pub(crate) unsafe fn refresh() {
    // Take the manager out of TLS before Win32 calls: callbacks cannot reborrow it.
    let Some(mut s) = MANAGER.with(|m| m.borrow_mut().take()) else {
        return;
    };
    s.pending = false;
    let mut previous = std::mem::take(&mut s.bridges);
    let mut dirty = std::collections::HashSet::new();
    let all = s.panes();
    if std::env::var_os("LUMEN_TRACE_FUSION").is_some() {
        eprintln!("native-fusion panes={all:?}");
    }
    for ps in components(&all).into_iter().filter(|g| g.len() > 1) {
        #[cfg(test)]
        if s.fail_surfaces {
            dirty.extend(ps.iter().map(|p| p.id));
            continue;
        }
        let opacities: Vec<_> = ps
            .iter()
            .map(|p| s.members.get(&p.id).map_or(255, |m| m.opacity) as f32 / 255.)
            .collect();
        let frosts: Vec<f32> = ps
            .iter()
            .map(|p| s.members.get(&p.id).map_or(255, |m| m.frost) as f32 / 255.)
            .collect();
        let wallpaper = s.glass.borrow().revision();
        let material = s.material_key(&ps);
        let old = previous
            .iter()
            .position(|b| b.panes.iter().map(|p| p.id).eq(ps.iter().map(|p| p.id)));
        let old = old.map(|i| previous.swap_remove(i));
        if old.as_ref().is_some_and(|b| {
            b.panes == ps
                && b.opacities == opacities
                && b.frosts == frosts
                && b.wallpaper == wallpaper
                && b.material == material
        }) {
            let bridge = old.unwrap();
            restack(bridge.hwnd, &ps, None, false);
            s.bridges.push(bridge);
            continue;
        }
        dirty.extend(ps.iter().map(|p| p.id));
        let dpi = ps.iter().map(|p| p.dpi).fold(1f32, f32::max);
        let pad = (48. * dpi).ceil() as i32;
        let l = ps.iter().map(|p| p.rect[0] as i32).min().unwrap() - pad;
        let t = ps.iter().map(|p| p.rect[1] as i32).min().unwrap() - pad;
        let r = ps.iter().map(|p| p.rect[2].ceil() as i32).max().unwrap() + pad;
        let b = ps.iter().map(|p| p.rect[3].ceil() as i32).max().unwrap() + pad;
        let (w, h) = (r - l, b - t);
        // Bound temporary allocations even for pathological multi-monitor layouts.
        if w <= 0 || h <= 0 || w as i64 * h as i64 > 16_000_000 {
            continue;
        }
        let hwnd = old.as_ref().map(|b| b.hwnd).unwrap_or_else(|| {
            CreateWindowExW(
                crate::gpu::window_ex_style()
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW,
                wide("LumenNativeFusionSurface").as_ptr(),
                null(),
                WS_POPUP | WS_DISABLED,
                l,
                t,
                w,
                h,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        });
        if hwnd.is_null() {
            continue;
        }
        let mut bridge = old.unwrap_or_else(|| Bridge {
            hwnd,
            panes: Vec::new(),
            opacities: Vec::new(),
            frosts: Vec::new(),
            wallpaper: 0,
            material: 0,
            uploads: 0,
        });
        let has_opaque_material = ps
            .iter()
            .any(|p| s.materials.get(&p.id).is_some_and(|m| m.is_opaque()));
        let has_overlay_material = ps
            .iter()
            .any(|p| s.materials.get(&p.id).is_some_and(|m| m.is_overlay));
        let windows: Vec<[i32; 4]> = ps
            .iter()
            .map(|p| s.members.get(&p.id).map_or([0; 4], |m| m.window))
            .collect();
        if !cfg!(test) && crate::gpu::is_gpu_window(hwnd) && !has_opaque_material && ps.len() <= 16
        {
            let packed = geometry::with_outline(&ps, |outline| {
                outline.pack_gpu_segments([l as f32, t as f32])
            });
            if let Some((
                fuse_lines,
                fuse_line_count,
                fuse_corner_c,
                fuse_corner_uv,
                fuse_corner_count,
            )) = packed
            {
                let ref_hwnd = ps[0].id as HWND;
                let (desk, accent, has_wallpaper) = {
                    let glass = s.glass.borrow();
                    let area = glass.monitor_origin_size(ref_hwnd);
                    let synced = if crate::gpu::wallpaper_revision_synced(wallpaper, area) {
                        true
                    } else if let Some((pair_area, blurred, sharp)) =
                        glass.gpu_wallpaper_pair_for(ref_hwnd)
                    {
                        crate::gpu::sync_wallpaper_pair(
                            wallpaper,
                            pair_area,
                            &blurred,
                            sharp.as_deref(),
                        )
                    } else {
                        false
                    };
                    (area, glass.wallpaper_accent(), synced)
                };
                let mut constants = crate::gpu::FusionBridgeConstants {
                    bridge_rect: [l as f32, t as f32, w as f32, h as f32],
                    desktop_rect: [
                        desk[0] as f32,
                        desk[1] as f32,
                        desk[2] as f32,
                        desk[3] as f32,
                    ],
                    params: [
                        dpi,
                        pad as f32,
                        if crate::glass::liquid_glass() { 1.0 } else { 0.0 },
                        0.0,
                    ],
                    accent_flags: [
                        (accent & 0xff) as f32 / 255.0,
                        ((accent >> 8) & 0xff) as f32 / 255.0,
                        ((accent >> 16) & 0xff) as f32 / 255.0,
                        if has_wallpaper { 1.0 } else { 0.0 },
                    ],
                    counts: [
                        ps.len() as u32,
                        windows.len() as u32,
                        fuse_line_count,
                        fuse_corner_count,
                    ],
                    fuse_lines,
                    fuse_corner_c,
                    fuse_corner_uv,
                    ..Default::default()
                };
                for (i, (p, &op)) in ps.iter().zip(&opacities).enumerate() {
                    constants.pane_rects[i] = [
                        p.rect[0] - l as f32,
                        p.rect[1] - t as f32,
                        p.rect[2] - l as f32,
                        p.rect[3] - t as f32,
                    ];
                    constants.pane_meta[i] = [p.radius, op, p.dpi, (1.0 - frosts[i]).clamp(0.0, 1.0)];
                    if let Some(mat) = s.materials.get(&p.id).filter(|m| m.is_delta) {
                        let d = mat.avg_rgb_delta();
                        constants.pane_delta[i] = [d[0], d[1], d[2], 1.0];
                    }
                }
                for (i, mw) in windows.iter().enumerate() {
                    constants.member_windows[i] = [
                        (mw[0] - l) as f32,
                        (mw[1] - t) as f32,
                        (mw[2] - l) as f32,
                        (mw[3] - t) as f32,
                    ];
                }
                let has_any_material = ps.iter().any(|p| s.materials.contains_key(&p.id));
                let field_data = if has_any_material {
                    let step = 8i32;
                    let fw = ((w + step - 1) / step + 1).max(2) as usize;
                    let fh = ((h + step - 1) / step + 1).max(2) as usize;
                    let mut field_rgba = vec![0u8; fw * fh * 4];
                    for fy in 0..fh {
                        let gy = (t + fy as i32 * step) as f32;
                        for fx in 0..fw {
                            let gx = (l + fx as i32 * step) as f32;
                            let idx = (fy * fw + fx) * 4;
                            if has_overlay_material {
                                let o = s.overlay_at(&ps, gx, gy).unwrap_or([0.0; 4]);
                                field_rgba[idx] = o[2].round().clamp(0.0, 255.0) as u8;
                                field_rgba[idx + 1] = o[1].round().clamp(0.0, 255.0) as u8;
                                field_rgba[idx + 2] = o[0].round().clamp(0.0, 255.0) as u8;
                                field_rgba[idx + 3] =
                                    (255.0 - o[3]).round().clamp(0.0, 255.0) as u8;
                            } else {
                                let (gain, bias_bgr) =
                                    s.affine_at(&ps, gx, gy).unwrap_or((1.0, [0.0; 3]));
                                field_rgba[idx] = (((bias_bgr[2] / 255.0) * 0.5 + 0.5)
                                    .clamp(0.0, 1.0)
                                    * 255.0)
                                    .round() as u8;
                                field_rgba[idx + 1] = (((bias_bgr[1] / 255.0) * 0.5 + 0.5)
                                    .clamp(0.0, 1.0)
                                    * 255.0)
                                    .round() as u8;
                                field_rgba[idx + 2] = (((bias_bgr[0] / 255.0) * 0.5 + 0.5)
                                    .clamp(0.0, 1.0)
                                    * 255.0)
                                    .round() as u8;
                                field_rgba[idx + 3] = (gain.clamp(0.0, 1.0) * 255.0).round() as u8;
                            }
                        }
                    }
                    constants.field_uv = [
                        fw as f32,
                        fh as f32,
                        step as f32,
                        if has_overlay_material { 2.0 } else { 1.0 },
                    ];
                    Some((fw as u32, fh as u32, field_rgba))
                } else {
                    None
                };
                let field_ref = field_data
                    .as_ref()
                    .map(|(fw, fh, data)| (*fw, *fh, data.as_slice()));
                restack(hwnd, &ps, Some([l, t, w, h]), false);
                if crate::gpu::present_fusion_bridge_with_field(hwnd, &constants, field_ref) {
                    bridge.uploads += 1;
                    restack(hwnd, &ps, None, true);
                    bridge.panes = ps;
                    bridge.opacities = opacities;
                    bridge.frosts = frosts;
                    bridge.wallpaper = wallpaper;
                    bridge.material = material;
                    s.bridges.push(bridge);
                    continue;
                }
            }
        }
        let Some(mut buffer) = Buffer::plain(w, h, dpi) else {
            continue;
        };
        if !has_opaque_material {
            SetWindowPos(
                hwnd,
                null_mut(),
                l,
                t,
                w,
                h,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOREDRAW,
            );
            let brush = CreateSolidBrush(s.glass.borrow().wallpaper_accent());
            let rr = RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            FillRect(buffer.dc, &rr, brush);
            DeleteObject(brush);
            if frosts.iter().all(|&f| (f - 1.0).abs() < 1e-4) {
                s.glass.borrow().draw(buffer.dc, hwnd, w, h);
            } else {
                s.glass.borrow().draw_with_frost(buffer.dc, hwnd, w, h, |x, y| {
                    s.frost_at(&ps, (l + x) as f32 + 0.5, (t + y) as f32 + 0.5)
                });
            }
        }
        let uniform_opacity = opacities
            .first()
            .copied()
            .filter(|&a0| opacities.iter().all(|&a| (a - a0).abs() < 1e-5));
        let pad_f = pad as f32;
        let pixels = buffer.pixels_mut();
        let liquid = crate::glass::liquid_glass();
        let spec_w = crate::compositor::liquid_spec_width(dpi, w as f32, h as f32);
        let bridge_rim = (40.0 * dpi).clamp(12.0, (w.min(h) as f32) * 0.44);
        let underlap = 3.0 * dpi;
        let mut rim_samples: Vec<(i32, i32, f32, f32, f32, f32, f32, f32)> = Vec::new();
        geometry::with_outline(&ps, |outline| {
            let r_max = pad_f + 6. * dpi;
            let mut x_zones: Vec<[f32; 2]> = outline
                .v_lines
                .iter()
                .map(|v| [v[0] - r_max, v[0] + r_max])
                .chain(
                    outline
                        .corners
                        .iter()
                        .map(|c| [c.min[0] - r_max, c.max[0] + r_max]),
                )
                .chain(windows.iter().flat_map(|w| {
                    let u = underlap + 2.0;
                    [
                        [w[0] as f32 - 2., w[0] as f32 + u],
                        [w[2] as f32 - u, w[2] as f32 + 2.],
                    ]
                }))
                .collect();
            x_zones.sort_unstable_by(|a, b| a[0].total_cmp(&b[0]));
            let mut merged_zones: Vec<[f32; 2]> = Vec::with_capacity(x_zones.len());
            for z in x_zones {
                if let Some(last) = merged_zones.last_mut() {
                    if z[0] <= last[1] {
                        last[1] = last[1].max(z[1]);
                        continue;
                    }
                }
                merged_zones.push(z);
            }
            for y in 0..h {
                let ly = t + y;
                let gy = ly as f32 + 0.5;
                let row = (y * w * 4) as usize;
                let mut x = 0i32;
                while x < w {
                    let lx = l + x;
                    let gx = lx as f32 + 0.5;
                    // Jump across horizontal core spans of any pane in O(1).
                    let mut jumped = false;
                    for p in &ps {
                        let [pl, pt, pr, pb] = p.rect;
                        let margin = 4. * p.dpi;
                        if gy < pt + margin || gy >= pb - margin {
                            continue;
                        }
                        let inset = p.radius + margin;
                        let (span_l, span_r) = if gy >= pt + inset && gy < pb - inset {
                            (pl + margin, pr - margin)
                        } else {
                            (pl + inset, pr - inset)
                        };
                        if gx >= span_l && gx < span_r {
                            let end_x = ((span_r - 0.5).floor() as i32 - l + 1).clamp(x + 1, w);
                            if !has_opaque_material {
                                pixels[row + (x as usize * 4)..row + (end_x as usize * 4)].fill(0);
                            }
                            x = end_x;
                            jumped = true;
                            break;
                        }
                    }
                    if jumped {
                        continue;
                    }
                    // Outside all vertical/corner X-zones, distance and shadow are 1D-invariant in X.
                    let in_zone = merged_zones.iter().any(|z| gx >= z[0] && gx <= z[1]);
                    let span_end_x = if !in_zone && uniform_opacity.is_some() {
                        let next_zone_x = merged_zones
                            .iter()
                            .filter(|z| z[0] > gx)
                            .map(|z| (z[0] - 0.5).floor() as i32 - l)
                            .min()
                            .unwrap_or(w);
                        next_zone_x.clamp(x + 1, w)
                    } else {
                        x + 1
                    };
                    let i = row + (x * 4) as usize;
                    if geometry::covered_core(&ps, gx, gy) {
                        if !has_opaque_material {
                            pixels[row + (x as usize * 4)..row + (span_end_x as usize * 4)].fill(0);
                        }
                        x = span_end_x;
                        continue;
                    }
                    let d = outline.distance([gx, gy]);
                    if d >= pad_f {
                        if !has_opaque_material {
                            pixels[row + (x as usize * 4)..row + (span_end_x as usize * 4)].fill(0);
                        }
                        x = span_end_x;
                        continue;
                    }
                    let in_window = windows
                        .iter()
                        .any(|w| lx >= w[0] && lx < w[2] && ly >= w[1] && ly < w[3]);
                    let in_deep_window = in_window
                        && windows.iter().any(|w| {
                            gx >= w[0] as f32 + underlap
                                && gx < w[2] as f32 - underlap
                                && gy >= w[1] as f32 + underlap
                                && gy < w[3] as f32 - underlap
                        });
                    let in_opaque_window = windows
                        .iter()
                        .zip(&opacities)
                        .any(|(w, &op)| {
                            op >= 0.999 && lx >= w[0] && lx < w[2] && ly >= w[1] && ly < w[3]
                        });
                    let in_deep_opaque_window = in_opaque_window
                        && windows.iter().zip(&opacities).any(|(w, &op)| {
                            op >= 0.999
                                && gx >= w[0] as f32 + underlap
                                && gx < w[2] as f32 - underlap
                                && gy >= w[1] as f32 + underlap
                                && gy < w[3] as f32 - underlap
                        });
                    let coverage = outline.coverage_with_distance(gx, gy, d);
                    // Keep a 3dp opaque member underlap band gated strictly on d < -1.0
                    // so the outer rounded AA border never double-blends while inner joins never leak.
                    if (in_window && d >= -1.0 && coverage > 0.) || (in_deep_opaque_window && d < -1.0) || (in_deep_window && d < -1.0 && uniform_opacity.is_some()) {
                        if !has_opaque_material {
                            pixels[row + (x as usize * 4)..row + (span_end_x as usize * 4)].fill(0);
                        }
                        x = span_end_x;
                        continue;
                    }
                    // A transparent neighbor must not erase the entire cluster shadow.
                    // Blend the local members' opacity continuously near the junction.
                    let alpha = if let Some(a) = uniform_opacity {
                        a
                    } else if coverage > 0. {
                        let reach = 120. * dpi;
                        let mut sum_w = 0.0f32;
                        let mut sum_a = 0.0f32;
                        for (p, &a) in ps.iter().zip(&opacities) {
                            let [pl, pt, pr, pb] = p.rect;
                            let rad = (4.0 * p.dpi).min((pr - pl).min(pb - pt) * 0.5);
                            let dp = crate::compositor::distance(
                                gx - pl,
                                gy - pt,
                                pr - pl,
                                pb - pt,
                                rad,
                                0.,
                            )
                            .max(0.);
                            if dp < reach {
                                let t = dp / reach;
                                let weight = (1. - t) * (1. - t);
                                sum_w += weight;
                                sum_a += weight * a;
                            }
                        }
                        let blended = if sum_w > 0. { sum_a / sum_w } else { 0. };
                        if in_window {
                            0.
                        } else {
                            blended
                        }
                    } else {
                        let mut sum = 0.;
                        let mut alpha = 0.;
                        for (p, a) in ps.iter().zip(&opacities) {
                            let weight = (-(p.distance(gx, gy) - d).max(0.) / (20. * dpi)).exp();
                            sum += weight;
                            alpha += weight * a;
                        }
                        if sum > 0. {
                            alpha / sum
                        } else {
                            0.
                        }
                    };
                    if coverage > 0. {
                        let a = coverage * alpha;
                        let edge = outline.rim_light(gx, gy, d, dpi);
                        let (nx, ny, has_n) = if liquid && d > -bridge_rim {
                            let dx =
                                outline.distance([gx + 1., gy]) - outline.distance([gx - 1., gy]);
                            let dy =
                                outline.distance([gx, gy + 1.]) - outline.distance([gx, gy - 1.]);
                            let len = (dx * dx + dy * dy).sqrt().max(1e-5);
                            (dx / len, dy / len, true)
                        } else {
                            (0., 0., false)
                        };
                        let spec = if has_n && -d < spec_w {
                            crate::compositor::liquid_rim_spec(d, nx, ny, spec_w)
                        } else {
                            0.
                        };
                        let material = if has_opaque_material {
                            s.material_at(&ps, gx, gy)
                        } else {
                            None
                        };
                        let affine = if !has_opaque_material && !has_overlay_material {
                            s.affine_at(&ps, gx, gy)
                        } else {
                            None
                        };
                        if material.is_none() && has_n {
                            rim_samples.push((x, y, d, nx, ny, a, edge, spec));
                        } else if has_overlay_material && !has_opaque_material {
                            let ov = s.overlay_at(&ps, gx, gy).unwrap_or([0.0; 4]);
                            let oa = (ov[3] / 255.0).clamp(0.0, 1.0);
                            for c in 0..3 {
                                let blended = (pixels[i + c] as f32 * (1.0 - oa) + ov[c])
                                    .clamp(0.0, 255.0);
                                let lit = (blended + (255.0 - blended) * edge + spec).min(255.0);
                                pixels[i + c] = (lit * a) as u8;
                            }
                        } else {
                            for c in 0..3 {
                                let value = material.map_or_else(
                                    || {
                                        let base = pixels[i + c] as f32;
                                        let (gain, bias) = affine.unwrap_or((1.0, [0.0; 3]));
                                        (base * gain + bias[c]).clamp(0., 255.)
                                    },
                                    |m| m[c],
                                );
                                let lit = (value + (255. - value) * edge + spec).min(255.);
                                pixels[i + c] = (lit * a) as u8;
                            }
                        }
                        pixels[i + 3] = (255. * a) as u8;
                        x += 1;
                    } else {
                        let sd = outline.distance([gx - 5. * dpi, gy - 8. * dpi]);
                        let fade = ((pad_f - d) / (8. * dpi)).clamp(0., 1.);
                        let tx = (sd / (17. * dpi)).clamp(-4.8, 4.8);
                        let tx2 = tx * tx;
                        let tanh = (tx * (135135. + tx2 * (17325. + tx2 * (378. + tx2)))
                            / (135135. + tx2 * (62370. + tx2 * (3150. + tx2 * 28.))))
                        .clamp(-1., 1.);
                        let a = (0.5 * (1. - tanh) * 96. * alpha * fade)
                            .round()
                            .clamp(0., 255.) as u8;
                        for sx in x..span_end_x {
                            let si = row + (sx * 4) as usize;
                            if !has_opaque_material {
                                pixels[si..si + 3].fill(0);
                            }
                            pixels[si + 3] = a;
                        }
                        x = span_end_x;
                    }
                }
            }
        });
        if !rim_samples.is_empty() {
            s.glass.borrow().apply_bridge_refraction_with_frost(
                hwnd,
                RECT {
                    left: l,
                    top: t,
                    right: r,
                    bottom: b,
                },
                w,
                h,
                dpi,
                pixels,
                &rim_samples,
                |x, y| s.frost_at(&ps, (l + x) as f32 + 0.5, (t + y) as f32 + 0.5),
            );
            if !has_opaque_material {
                for &(x, y, _d, _nx, _ny, a, edge, spec) in &rim_samples {
                    if a <= 1e-4 {
                        continue;
                    }
                    let gx = (l + x) as f32 + 0.5;
                    let gy = (t + y) as f32 + 0.5;
                    let i = ((y * w + x) * 4) as usize;
                    if has_overlay_material {
                        if let Some(ov) = s.overlay_at(&ps, gx, gy) {
                            let oa = (ov[3] / 255.0).clamp(0.0, 1.0);
                            for c in 0..3 {
                                let lit = pixels[i + c] as f32 / a;
                                let lit_new = ((lit - 255.0 * edge - spec) * (1.0 - oa)
                                    + ov[c] * (1.0 - edge)
                                    + 255.0 * edge
                                    + spec)
                                    .clamp(0.0, 255.0);
                                pixels[i + c] = (lit_new * a).round().clamp(0.0, 255.0) as u8;
                            }
                        }
                    } else if let Some((gain, bias)) = s.affine_at(&ps, gx, gy) {
                        for c in 0..3 {
                            let lit = pixels[i + c] as f32 / a;
                            let lit_new = ((lit - 255.0 * edge - spec) * gain
                                + bias[c] * (1.0 - edge)
                                + 255.0 * edge
                                + spec)
                                .clamp(0.0, 255.0);
                            pixels[i + c] = (lit_new * a).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
        }
        restack(hwnd, &ps, Some([l, t, w, h]), false);
        if buffer.upload(hwnd) {
            bridge.uploads += 1;
            restack(hwnd, &ps, None, true);
            bridge.panes = ps;
            bridge.opacities = opacities;
            bridge.frosts = frosts;
            bridge.wallpaper = wallpaper;
            bridge.material = material;
            s.bridges.push(bridge);
        }
    }
    for b in &previous {
        dirty.extend(b.panes.iter().map(|p| p.id));
    }
    drop(previous);
    let windows: Vec<_> = dirty.into_iter().collect();
    MANAGER.with(|m| *m.borrow_mut() = Some(s));
    for id in windows {
        InvalidateRect(id as HWND, null(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Local fusion timing diagnostic"]
    fn profile_large_join_and_detach() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            init(Rc::new(RefCell::new(Glass::new(0., 1.))));
            let mut windows = Vec::new();
            for x in [-20000, -19108] {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Lumen fusion timing").as_ptr(),
                    WS_POPUP,
                    x,
                    -20000,
                    900,
                    650,
                    null_mut(),
                    null_mut(),
                    GetModuleHandleW(null()),
                    null(),
                );
                assert!(!hwnd.is_null());
                register_widget(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                prepare(hwnd, 34.);
                windows.push(hwnd);
            }
            let begin = std::time::Instant::now();
            refresh();
            eprintln!(
                "fusion-profile join_ms={:.2}",
                begin.elapsed().as_secs_f64() * 1000.
            );
            assert!(fused(windows[0]));
            let mut moved = window_rect(windows[0]);
            moved.left += 24;
            moved.right += 24;
            let begin = std::time::Instant::now();
            SendMessageW(windows[0], WM_ENTERSIZEMOVE, 0, 0);
            SendMessageW(windows[0], WM_MOVING, 0, &mut moved as *mut _ as isize);
            refresh();
            eprintln!(
                "fusion-profile detach_ms={:.2}",
                begin.elapsed().as_secs_f64() * 1000.
            );
            assert!(!fused(windows[0]));
            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }
    #[test]
    fn actual_join_blends_raw_widget_colors_and_settles_without_feedback() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            init(Rc::new(RefCell::new(Glass::new(0., 1.))));
            let mut windows = Vec::new();
            for y in [-20000, -19808] {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Fusion material regression").as_ptr(),
                    WS_POPUP,
                    -20000,
                    y,
                    200,
                    200,
                    null_mut(),
                    null_mut(),
                    GetModuleHandleW(null()),
                    null(),
                );
                assert!(!hwnd.is_null());
                register_widget(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                prepare(hwnd, 34.);
                windows.push(hwnd);
            }
            let mut frames = [
                Buffer::new(200, 200, 1., 34.).unwrap(),
                Buffer::new(200, 200, 1., 34.).unwrap(),
            ];
            let fill = |frame: &mut Buffer, color| {
                let brush = CreateSolidBrush(color);
                FillRect(
                    frame.dc,
                    &RECT {
                        left: 0,
                        top: 0,
                        right: 200,
                        bottom: 200,
                    },
                    brush,
                );
                DeleteObject(brush);
                frame.background_alpha(255);
            };
            for i in 0..2 {
                fill(&mut frames[i], if i == 0 { 0x0000ff } else { 0xff0000 });
                frames[i].bevel_for(windows[i]);
            }
            refresh();
            assert!(fused(windows[0]));
            let mut key = 0;
            for pass in 0..3 {
                for i in 0..2 {
                    fill(&mut frames[i], if i == 0 { 0x0000ff } else { 0xff0000 });
                    frames[i].bevel_for(windows[i]);
                    assert!(frames[i].present_with_alpha(windows[i], true));
                }
                refresh();
                let pixels = frames[0].snapshot();
                let seam = (195 * 200 + 100) * 4;
                assert!(
                    pixels[seam] > 115 && pixels[seam + 2] > 115,
                    "join mixes both saturated widget colors"
                );
                assert!(
                    pixels[seam + 1] < 3,
                    "no wallpaper/neutral wash at the join"
                );
                assert_eq!(
                    pixels[seam + 3],
                    255,
                    "color blending does not reduce alpha"
                );
                let next = revision(windows[0]);
                if pass > 0 {
                    assert_eq!(
                        key, next,
                        "unblended source tiles must not feed their own blend back"
                    );
                }
                key = next;
            }
            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }
    #[test]
    fn decorative_surface_stays_below_topmost_controls() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let make = || {
                CreateWindowExW(
                    WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Lumen stack test").as_ptr(),
                    WS_POPUP,
                    -18000,
                    -18000,
                    20,
                    20,
                    null_mut(),
                    null_mut(),
                    GetModuleHandleW(null()),
                    null(),
                )
            };
            let a = make();
            let b = make();
            let controls = make();
            let bridge = make();
            assert!([a, b, controls, bridge].iter().all(|h| !h.is_null()));
            for h in [a, b, controls] {
                SetWindowPos(
                    h,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
            let panes = [
                Pane {
                    id: a as usize,
                    rect: [0., 0., 400., 300.],
                    radius: 30.,
                    dpi: 1.,
                    topmost: true,
                },
                Pane {
                    id: b as usize,
                    rect: [400., 0., 700., 300.],
                    radius: 30.,
                    dpi: 1.,
                    topmost: true,
                },
            ];
            restack(bridge, &panes, None, false);
            let mut w = GetTopWindow(null_mut());
            let mut ranks = HashMap::new();
            for rank in 0..4096 {
                if w.is_null() {
                    break;
                }
                ranks.insert(w as usize, rank);
                w = GetWindow(w, GW_HWNDNEXT);
            }
            assert!(
                ranks[&(controls as usize)] < ranks[&(bridge as usize)],
                "bridge must not cover General Settings"
            );
            assert!(
                ranks[&(a as usize)].max(ranks[&(b as usize)]) < ranks[&(bridge as usize)],
                "bridge must sit strictly below every member pane"
            );
            assert!(GetWindow(bridge, GW_OWNER).is_null());
            for h in [bridge, controls, b, a] {
                DestroyWindow(h);
            }
        }
    }
    #[test]
    fn registered_native_windows_fuse_detach_hide_and_release_the_shared_surface() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            init(Rc::new(RefCell::new(Glass::new(12., 1.))));
            let instance = GetModuleHandleW(null());
            let mut windows = Vec::new();
            for (x, y, w, h) in [
                (100, 300, 500, 260),
                (100, 100, 240, 208),
                (332, 100, 240, 208),
            ] {
                // Never replace real widget classes with test wndprocs: the
                // other native host tests run in parallel in this process.
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Lumen fusion test").as_ptr(),
                    WS_POPUP,
                    x - 20000,
                    y - 20000,
                    w,
                    h,
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                // Test-owned layered HWNDs stay far outside every desktop.
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                MANAGER.with(|m| {
                    m.borrow_mut()
                        .as_mut()
                        .unwrap()
                        .members
                        .insert(hwnd as usize, read_member(hwnd, 30.));
                });
                SetWindowSubclass(hwnd, Some(widget_proc), SUBCLASS, 0);
                windows.push(hwnd);
            }
            assert!(
                group(windows[0]).is_none(),
                "do not suppress chrome until upload commits"
            );
            refresh();
            assert_eq!(group(windows[0]).unwrap().len(), 3);
            MANAGER.with(|m| assert_eq!(m.borrow().as_ref().unwrap().bridges.len(), 1));
            // Exercise actual BGRA compositor outputs, not just the ownership
            // predicate: three translucent HWNDs must not darken either seam.
            let a = read_member(windows[0], 30.);
            let b = read_member(windows[1], 30.);
            let mut sizing = RECT {
                left: b.window[0],
                top: b.window[1],
                right: b.window[2] - 3,
                bottom: b.window[3],
            };
            SendMessageW(
                windows[1],
                WM_SIZING,
                WMSZ_RIGHT as usize,
                &mut sizing as *mut _ as isize,
            );
            assert_eq!(
                [sizing.left, sizing.top, sizing.right, sizing.bottom],
                b.window,
                "native resize snaps the grabbed edge without moving its opposite anchor"
            );
            sizing.right -= 3;
            set_enabled(false);
            let unsnapped = sizing.right;
            SendMessageW(
                windows[1],
                WM_SIZING,
                WMSZ_RIGHT as usize,
                &mut sizing as *mut _ as isize,
            );
            assert_eq!(
                sizing.right, unsnapped,
                "disabled snapping leaves resize free"
            );
            set_enabled(true);
            for alpha in [0u8, 64, 128, 255] {
                let mut outputs = Vec::new();
                for hwnd in &windows {
                    let m = read_member(*hwnd, 30.);
                    let [l, t, r, b] = m.window;
                    let mut frame = Buffer::new(r - l, b - t, m.pane.dpi, 30.).unwrap();
                    let rect = RECT {
                        left: 0,
                        top: 0,
                        right: r - l,
                        bottom: b - t,
                    };
                    let brush = CreateSolidBrush(0x405060);
                    FillRect(frame.dc, &rect, brush);
                    DeleteObject(brush);
                    frame.background_alpha(alpha);
                    assert!(frame.present_with_alpha(*hwnd, true));
                    outputs.push((m.window, frame.snapshot()));
                }
                for point in [
                    [a.pane.rect[0] + 80., a.pane.rect[1]],
                    [b.pane.rect[2], b.pane.rect[1] + 80.],
                ] {
                    for offset in -3..=3 {
                        let x = point[0] as i32 + offset;
                        let y = point[1] as i32;
                        let mut composed = 0u32;
                        for (r, pixels) in &outputs {
                            if x < r[0] || x >= r[2] || y < r[1] || y >= r[3] {
                                continue;
                            }
                            let i = (((y - r[1]) * (r[2] - r[0]) + x - r[0]) * 4) as usize;
                            let v = pixels[i + 3] as u32;
                            assert!(pixels[i..i + 3].iter().all(|c| *c as u32 <= v));
                            composed = v + (composed * (255 - v) + 127) / 255;
                        }
                        assert_eq!(
                            composed,
                            alpha.max(1) as u32,
                            "single alpha at straight/T seam"
                        );
                    }
                }
            }
            // Mixed member opacities blend continuously across the seam without a hard step.
            opacity(windows[0], 255);
            opacity(windows[1], 0);
            refresh();
            let mut mixed_outputs = Vec::new();
            for (idx, hwnd) in [windows[0], windows[1]].into_iter().enumerate() {
                let m = read_member(hwnd, 30.);
                let [l, t, r, b] = m.window;
                let mut frame = Buffer::new(r - l, b - t, m.pane.dpi, 30.).unwrap();
                let rect = RECT {
                    left: 0,
                    top: 0,
                    right: r - l,
                    bottom: b - t,
                };
                let brush = CreateSolidBrush(0x405060);
                FillRect(frame.dc, &rect, brush);
                DeleteObject(brush);
                frame.background_alpha_for(hwnd, if idx == 0 { 255 } else { 0 });
                assert!(frame.present_with_alpha(hwnd, true));
                mixed_outputs.push((m.window, frame.snapshot()));
            }
            let seam_x = a.pane.rect[0] as i32 + 80;
            let seam_y = a.pane.rect[1] as i32;
            let mut prev_alpha: Option<i32> = None;
            for y in (seam_y - 10)..=(seam_y + 10) {
                let mut composed = 0u32;
                for (r, pixels) in &mixed_outputs {
                    if seam_x < r[0] || seam_x >= r[2] || y < r[1] || y >= r[3] {
                        continue;
                    }
                    let i = (((y - r[1]) * (r[2] - r[0]) + seam_x - r[0]) * 4) as usize;
                    let v = pixels[i + 3] as u32;
                    composed = v + (composed * (255 - v) + 127) / 255;
                }
                if let Some(prev) = prev_alpha {
                    assert!(
                        (prev - composed as i32).abs() <= 4,
                        "opacity across seam must transition smoothly without a hard step (y={y}, prev={prev}, cur={composed})"
                    );
                }
                prev_alpha = Some(composed as i32);
            }
            let handle = MANAGER.with(|m| m.borrow().as_ref().unwrap().bridges[0].hwnd);
            let uploads = MANAGER.with(|m| m.borrow().as_ref().unwrap().bridges[0].uploads);
            let token = revision(windows[0]);
            // An unrelated, far-away member does not churn this bridge.
            MANAGER.with(|m| {
                let mut s = m.borrow_mut();
                let s = s.as_mut().unwrap();
                let mut p = s.members.values().next().unwrap().clone();
                p.pane.id = usize::MAX;
                p.pane.rect = [9000., 9000., 9500., 9500.];
                s.members.insert(usize::MAX, p);
                s.changed();
            });
            refresh();
            MANAGER.with(|m| assert_eq!(m.borrow().as_ref().unwrap().bridges[0].hwnd, handle));
            MANAGER.with(|m| assert_eq!(m.borrow().as_ref().unwrap().bridges[0].uploads, uploads));
            assert_eq!(
                revision(windows[0]),
                token,
                "unrelated widget keeps component cache revision"
            );
            MANAGER.with(|m| {
                let mut s = m.borrow_mut();
                let s = s.as_mut().unwrap();
                s.members.remove(&usize::MAX);
                s.fail_surfaces = true;
                s.changed();
            });
            refresh();
            assert!(
                !fused(windows[0]),
                "failed upload falls back to standalone chrome"
            );
            MANAGER.with(|m| m.borrow_mut().as_mut().unwrap().fail_surfaces = false);
            refresh();
            assert!(fused(windows[0]));
            set_enabled(false);
            assert!(!fused(windows[0]));
            refresh();
            MANAGER.with(|m| assert!(m.borrow().as_ref().unwrap().bridges.is_empty()));
            set_enabled(true);
            refresh();
            let id = windows[1] as usize;
            MANAGER.with(|m| {
                let mut s = m.borrow_mut();
                let s = s.as_mut().unwrap();
                s.members.get_mut(&id).unwrap().moving = true;
                s.changed();
            });
            refresh();
            assert!(!fused(windows[1]));
            assert!(fused(windows[0]));
            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
            MANAGER.with(|m| assert!(m.borrow().is_none()));
        }
    }

    #[test]
    fn fused_l_junction_liquid_glass_visual_verification() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::glass::set_liquid_glass(self.0);
            }
        }
        let _reset = Reset(crate::glass::liquid_glass());
        crate::glass::set_liquid_glass(true);

        let verify_dir = std::env::var_os("LUMEN_VERIFY_DIR").map(std::path::PathBuf::from);
        if let Some(ref dir) = verify_dir {
            std::fs::create_dir_all(dir).unwrap();
        }
        let temp_dir = std::env::temp_dir().join(format!(
            "lumen-fusion-verify-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        struct CleanupDir(std::path::PathBuf);
        impl Drop for CleanupDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = CleanupDir(temp_dir.clone());

        let (cw, ch) = (620usize, 420usize);
        let wave_path = temp_dir.join("fusion-wave.png");
        let mut wave_img = image::RgbaImage::new(cw as u32, ch as u32);
        let tau = std::f32::consts::TAU;
        for y in 0..ch {
            let py = y as f32 / ch as f32 * tau;
            for x in 0..cw {
                let px = x as f32 / cw as f32 * tau;
                let ribbon1 = ((px + py.sin() * 0.55).sin() * 0.5 + 0.5).powf(2.0);
                let ribbon2 = (((px * 2.0 - 0.8) - py.cos() * 0.6).sin() * 0.5 + 0.5).powf(2.6);
                let crest = (1.0 - ((py.sin() - 0.55 * px.sin()).abs() * 2.4).min(1.0)).powf(2.0);
                let r = (12.0 + 45.0 * ribbon2 + 110.0 * crest).clamp(0.0, 255.0) as u8;
                let g = (44.0 + 130.0 * ribbon1 + 115.0 * crest).clamp(0.0, 255.0) as u8;
                let b = (118.0 + 128.0 * ribbon1.max(ribbon2) + 42.0 * crest).clamp(0.0, 255.0) as u8;
                wave_img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
            }
        }
        wave_img.save(&wave_path).unwrap();

        unsafe {
            let _cap = crate::compositor::test_capture::start();
            let widget_glass = Glass::fixture_tiled_file(&wave_path, 14.0, 1.0);
            init(Rc::new(RefCell::new(Glass::fixture_tiled_file(&wave_path, 14.0, 1.0))));
            let instance = GetModuleHandleW(null());
            let probe = RECT {
                left: -(cw as i32) * 10,
                top: -(ch as i32) * 20,
                right: -(cw as i32) * 9,
                bottom: -(ch as i32) * 19,
            };
            let mon = MonitorFromRect(&probe, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            let ox = mi.rcMonitor.left - (cw as i32) * 10;
            let oy = mi.rcMonitor.top - (ch as i32) * 20;

            // L-junction cluster with deliberate 2px edge step between Clock (x=40) and Notes (x=42):
            // Clock [40..300, 40..230] (pane [44..296, 44..226]) — warm sunset orange chrome
            // Notes [42..222, 222..382] (pane [46..218, 226..378]) — soft cool slate chrome
            // Music [292..580, 40..382] (pane [296..576, 44..378]) — deep royal blue chrome
            // Concave Bridge fillet connects Notes right (x=218) + Clock bottom (y=226) + Music left (x=296).
            struct WidgetSpec {
                rect: [i32; 4],
                top_rgb: [f32; 3],
                bot_rgb: [f32; 3],
                tint_mix: f32,
            }
            let specs = [
                WidgetSpec {
                    rect: [ox + 40, oy + 40, 260, 190],
                    top_rgb: [248.0, 185.0, 138.0],
                    bot_rgb: [242.0, 128.0, 74.0],
                    tint_mix: 0.44,
                },
                WidgetSpec {
                    rect: [ox + 42, oy + 222, 180, 160],
                    top_rgb: [132.0, 168.0, 214.0],
                    bot_rgb: [96.0, 134.0, 186.0],
                    tint_mix: 0.40,
                },
                WidgetSpec {
                    rect: [ox + 292, oy + 40, 288, 342],
                    top_rgb: [42.0, 108.0, 224.0],
                    bot_rgb: [18.0, 62.0, 168.0],
                    tint_mix: 0.42,
                },
            ];
            let mut windows = Vec::new();
            for sp in &specs {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Fusion Liquid Glass verify").as_ptr(),
                    WS_POPUP,
                    sp.rect[0],
                    sp.rect[1],
                    sp.rect[2],
                    sp.rect[3],
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                register_widget(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                prepare(hwnd, 30.0);
                windows.push(hwnd);
            }

            let paint_all = || {
                for (&hwnd, sp) in windows.iter().zip(specs.iter()) {
                    let [_, _, w, h] = sp.rect;
                    let mut frame = Buffer::new(w, h, 1.0, 30.0).unwrap();
                    let brush = CreateSolidBrush(0x281e18);
                    FillRect(frame.dc, &RECT { left: 0, top: 0, right: w, bottom: h }, brush);
                    DeleteObject(brush);
                    widget_glass.draw_blended(frame.dc, hwnd, w, h, 215);
                    {
                        let px = frame.pixels_mut();
                        for y in 0..h as usize {
                            let ty = y as f32 / (h as f32).max(1.0);
                            let cr = sp.top_rgb[0] * (1.0 - ty) + sp.bot_rgb[0] * ty;
                            let cg = sp.top_rgb[1] * (1.0 - ty) + sp.bot_rgb[1] * ty;
                            let cb = sp.top_rgb[2] * (1.0 - ty) + sp.bot_rgb[2] * ty;
                            let row = y * w as usize * 4;
                            for x in 0..w as usize {
                                let i = row + x * 4;
                                px[i] = (px[i] as f32 * (1.0 - sp.tint_mix) + cb * sp.tint_mix).round() as u8;
                                px[i + 1] = (px[i + 1] as f32 * (1.0 - sp.tint_mix) + cg * sp.tint_mix).round() as u8;
                                px[i + 2] = (px[i + 2] as f32 * (1.0 - sp.tint_mix) + cr * sp.tint_mix).round() as u8;
                            }
                        }
                    }
                    frame.background_alpha_for(hwnd, 255);
                    frame.bevel_for(hwnd);
                    assert!(frame.present_with_alpha(hwnd, true));
                }
            };

            paint_all();
            refresh();
            assert!(windows.iter().all(|&h| fused(h)), "all L-junction panes must fuse");
            paint_all();
            refresh();

            let bridge_hwnd = MANAGER.with(|m| {
                let guard = m.borrow();
                let s = guard.as_ref().unwrap();
                assert_eq!(s.bridges.len(), 1);
                s.bridges[0].hwnd
            });
            let bridge_cap = crate::compositor::test_capture::get(bridge_hwnd).expect("bridge upload");
            let member_caps: Vec<_> = windows
                .iter()
                .map(|&h| crate::compositor::test_capture::get(h).expect("member upload"))
                .collect();

            // Composite onto 620x420 wave wallpaper canvas (Bridge below members, matching Z-order).
            let mut canvas = wave_img.clone();
            let blend_cap = |canvas: &mut image::RgbaImage, cap: &crate::compositor::test_capture::CapturedUpload| {
                let lx0 = cap.rect[0] - ox;
                let ly0 = cap.rect[1] - oy;
                for y in 0..cap.h {
                    let cy = ly0 + y;
                    if cy < 0 || cy >= ch as i32 {
                        continue;
                    }
                    for x in 0..cap.w {
                        let cx = lx0 + x;
                        if cx < 0 || cx >= cw as i32 {
                            continue;
                        }
                        let si = ((y * cap.w + x) * 4) as usize;
                        let a = cap.bgra[si + 3] as u32;
                        if a == 0 {
                            continue;
                        }
                        let inv = 255 - a;
                        let p = canvas.get_pixel_mut(cx as u32, cy as u32);
                        p.0[0] = (cap.bgra[si + 2] as u32 + (p.0[0] as u32 * inv + 127) / 255).min(255) as u8;
                        p.0[1] = (cap.bgra[si + 1] as u32 + (p.0[1] as u32 * inv + 127) / 255).min(255) as u8;
                        p.0[2] = (cap.bgra[si] as u32 + (p.0[2] as u32 * inv + 127) / 255).min(255) as u8;
                        p.0[3] = 255;
                    }
                }
            };
            blend_cap(&mut canvas, &bridge_cap);
            for mc in &member_caps {
                blend_cap(&mut canvas, mc);
            }

            if let Some(ref dir) = verify_dir {
                canvas.save(dir.join("verify_fused_l_junction.png")).unwrap();
            }

            // 1) Smooth blend across Clock | Music and across Music interior without seam/tile cuts.
            let mut max_seam_step = 0i32;
            for x in 270u32..555 {
                let a = canvas.get_pixel(x, 140).0;
                let b = canvas.get_pixel(x + 1, 140).0;
                for c in 0..3 {
                    max_seam_step = max_seam_step.max((a[c] as i32 - b[c] as i32).abs());
                }
            }
            assert!(
                max_seam_step <= 8,
                "fused Clock | Music seam & interior must blend smoothly without L-cut discontinuities (max 1px jump={max_seam_step})"
            );
            let mut max_vert_step = 0i32;
            for y in 65u32..355 {
                let a = canvas.get_pixel(350, y).0;
                let b = canvas.get_pixel(350, y + 1).0;
                for c in 0..3 {
                    max_vert_step = max_vert_step.max((a[c] as i32 - b[c] as i32).abs());
                }
            }
            assert!(
                max_vert_step <= 8,
                "fused Music vertical slice must remain smooth without cutoff steps (max 1px jump={max_vert_step})"
            );

            // 2) Zero square corner regression on sub-2dp offset: Notes outer corner bbox (x=42, y=381) must stay rounded (transparent), not filled square!
            let notes_cap = &member_caps[1];
            let corner_bbox = (notes_cap.h as usize - 1) * notes_cap.w as usize * 4;
            assert_eq!(
                notes_cap.bgra[corner_bbox + 3],
                0,
                "bottom-left outer corner must stay transparent with rounded corner arc"
            );
            // Also check inward corner around offset (8, h-9): should be inside rounded radius 30? Wait: radius=30, margin=4 -> (4 + 30*(1 - cos 45°)) = 4 + 8.8 = 12.8px. So (6, h-7) is outside rounded arc (alpha=0)!
            let outer_arc_px = ((notes_cap.h as usize - 7) * notes_cap.w as usize + 6) * 4;
            assert_eq!(
                notes_cap.bgra[outer_arc_px + 3],
                0,
                "notes rounded corner must NOT collapse square despite 2px left edge mismatch with clock"
            );

            // 3) Concave Bridge fillet pixel in the L-notch (x = 223, y = 232) must be opaque/blended (Bridge filled the notch!).
            let bx = 223 - (bridge_cap.rect[0] - ox);
            let by = 232 - (bridge_cap.rect[1] - oy);
            let bi = ((by * bridge_cap.w + bx) * 4) as usize;
            assert!(
                bridge_cap.bgra[bi + 3] > 200,
                "bridge must smoothly fill concave L-junction fillet (alpha={})",
                bridge_cap.bgra[bi + 3]
            );

            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
            MANAGER.with(|m| assert!(m.borrow().is_none()));
        }
    }

    #[test]
    fn gpu_bridge_is_click_through_and_below_all_members() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            init(Rc::new(RefCell::new(Glass::new(0., 1.))));
            let instance = GetModuleHandleW(null());
            let make_widget = |x: i32, y: i32| {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                    wide("STATIC").as_ptr(),
                    wide("Lumen click-through test widget").as_ptr(),
                    WS_POPUP | WS_VISIBLE,
                    x,
                    y,
                    60,
                    60,
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
                hwnd
            };
            let w1 = make_widget(8, 8);
            let w2 = make_widget(68, 8);
            let bridge = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP
                    | WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST,
                wide("LumenNativeFusionSurface").as_ptr(),
                null(),
                WS_POPUP | WS_DISABLED,
                0,
                0,
                140,
                80,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            assert!(!bridge.is_null());
            let mut constants = crate::gpu::FusionBridgeConstants {
                bridge_rect: [0., 0., 140., 80.],
                desktop_rect: [0., 0., 1920., 1080.],
                params: [1., 8., 0., 0.],
                accent_flags: [0.2, 0.3, 0.4, 0.],
                counts: [2, 2, 0, 0],
                ..Default::default()
            };
            constants.pane_rects[0] = [8., 8., 68., 68.];
            constants.pane_rects[1] = [68., 8., 128., 68.];
            constants.pane_meta[0] = [16., 1., 1., 0.];
            constants.pane_meta[1] = [16., 1., 1., 0.];
            constants.member_windows[0] = [8., 8., 68., 68.];
            constants.member_windows[1] = [68., 8., 128., 68.];
            if crate::gpu::available() {
                assert!(
                    crate::gpu::present_fusion_bridge(bridge, &constants),
                    "DirectComposition bridge must accept WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT"
                );
            }
            let panes = [
                Pane {
                    id: w1 as usize,
                    rect: [8., 8., 68., 68.],
                    radius: 16.,
                    dpi: 1.,
                    topmost: true,
                },
                Pane {
                    id: w2 as usize,
                    rect: [68., 8., 128., 68.],
                    radius: 16.,
                    dpi: 1.,
                    topmost: true,
                },
            ];
            // Even if placed above both widgets, the bridge must remain click-through.
            SetWindowPos(
                bridge,
                HWND_TOP,
                0,
                0,
                140,
                80,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            assert_ne!(
                WindowFromPoint(POINT { x: 38, y: 38 }),
                bridge,
                "bridge HWND must be click-through"
            );
            // Showing via restack must place bridge visible and strictly below every member.
            ShowWindow(bridge, SW_HIDE);
            restack(bridge, &panes, Some([0, 0, 140, 80]), true);
            assert_ne!(IsWindowVisible(bridge), 0);
            let mut w = GetTopWindow(null_mut());
            let mut ranks = HashMap::new();
            for rank in 0..4096 {
                if w.is_null() {
                    break;
                }
                ranks.insert(w as usize, rank);
                w = GetWindow(w, GW_HWNDNEXT);
            }
            assert!(
                ranks[&(w1 as usize)].max(ranks[&(w2 as usize)]) < ranks[&(bridge as usize)],
                "restack(..., show=true) must keep bridge below all member widgets"
            );
            assert_ne!(WindowFromPoint(POINT { x: 38, y: 38 }), bridge);
            crate::gpu::remove_surface(bridge);
            DestroyWindow(bridge);
            DestroyWindow(w2);
            DestroyWindow(w1);
            shutdown();
        }
    }

    #[test]
    fn frosted_l_junction_over_diagonal_wallpaper_preserves_diagonal_variation() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::glass::set_frosted_glass(self.0);
                crate::glass::set_liquid_glass(self.1);
            }
        }
        let _reset = Reset(
            crate::glass::frosted_glass(),
            crate::glass::liquid_glass(),
        );
        crate::glass::set_frosted_glass(true);
        crate::glass::set_liquid_glass(true);

        let temp_dir = std::env::temp_dir().join(format!(
            "lumen-fusion-diag-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        struct CleanupDir(std::path::PathBuf);
        impl Drop for CleanupDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = CleanupDir(temp_dir.clone());

        let (cw, ch) = (620usize, 420usize);
        let diag_path = temp_dir.join("diag-stripe.png");
        let mut diag_img = image::RgbaImage::new(cw as u32, ch as u32);
        for y in 0..ch {
            for x in 0..cw {
                let phase = (x as f32 + y as f32) * std::f32::consts::TAU / 120.0;
                let s = phase.sin() * 0.5 + 0.5;
                let r = (35.0 + 195.0 * s).round().clamp(0.0, 255.0) as u8;
                let g = (65.0 + 135.0 * (1.0 - s)).round().clamp(0.0, 255.0) as u8;
                let b = (215.0 - 155.0 * s).round().clamp(0.0, 255.0) as u8;
                diag_img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
            }
        }
        diag_img.save(&diag_path).unwrap();

        unsafe {
            let _cap = crate::compositor::test_capture::start();
            let widget_glass = Glass::fixture_tiled_file(&diag_path, 14.0, 1.0);
            init(Rc::new(RefCell::new(Glass::fixture_tiled_file(
                &diag_path, 14.0, 1.0,
            ))));
            let instance = GetModuleHandleW(null());
            let probe = RECT {
                left: -(cw as i32) * 10,
                top: -(ch as i32) * 20,
                right: -(cw as i32) * 9,
                bottom: -(ch as i32) * 19,
            };
            let mon = MonitorFromRect(&probe, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            let ox = mi.rcMonitor.left - (cw as i32) * 10;
            let oy = mi.rcMonitor.top - (ch as i32) * 20;
            let specs = [
                ([ox + 40, oy + 40, 260, 190], [230.0f32, 160.0, 120.0]),
                ([ox + 42, oy + 222, 180, 160], [110.0f32, 150.0, 210.0]),
                ([ox + 292, oy + 40, 288, 342], [45.0f32, 100.0, 210.0]),
            ];
            let mut windows = Vec::new();
            for (rect, _) in &specs {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Frosted diagonal verify").as_ptr(),
                    WS_POPUP,
                    rect[0],
                    rect[1],
                    rect[2],
                    rect[3],
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                register_widget(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                prepare(hwnd, 30.0);
                windows.push(hwnd);
            }

            let paint_all = || {
                for (&hwnd, (rect, tint)) in windows.iter().zip(specs.iter()) {
                    let [_, _, w, h] = *rect;
                    let mut frame = Buffer::new(w, h, 1.0, 30.0).unwrap();
                    let brush = CreateSolidBrush(0x281e18);
                    FillRect(
                        frame.dc,
                        &RECT {
                            left: 0,
                            top: 0,
                            right: w,
                            bottom: h,
                        },
                        brush,
                    );
                    DeleteObject(brush);
                    widget_glass.draw_blended(frame.dc, hwnd, w, h, 215);
                    {
                        let px = frame.pixels_mut();
                        let mix = 0.25f32;
                        for p in px.chunks_exact_mut(4) {
                            p[0] = (p[0] as f32 * (1.0 - mix) + tint[2] * mix).round() as u8;
                            p[1] = (p[1] as f32 * (1.0 - mix) + tint[1] * mix).round() as u8;
                            p[2] = (p[2] as f32 * (1.0 - mix) + tint[0] * mix).round() as u8;
                        }
                    }
                    frame.background_alpha_for(hwnd, 255);
                    frame.bevel_for(hwnd);
                    assert!(frame.present_with_alpha(hwnd, true));
                }
            };

            paint_all();
            refresh();
            assert!(windows.iter().all(|&h| fused(h)));
            paint_all();
            refresh();

            let bridge_hwnd = MANAGER.with(|m| {
                let guard = m.borrow();
                let s = guard.as_ref().unwrap();
                assert_eq!(s.bridges.len(), 1);
                s.bridges[0].hwnd
            });
            let bridge_cap =
                crate::compositor::test_capture::get(bridge_hwnd).expect("bridge upload");
            let member_caps: Vec<_> = windows
                .iter()
                .map(|&h| crate::compositor::test_capture::get(h).expect("member upload"))
                .collect();

            // Sample directly inside both opaque concave bridge fillets at y = 231
            // (outside member windows: left fillet x=223..=229, right fillet x=285..=291)
            // and along the left concave bridge vertical edge (x=223, y=231..=237).
            let sample_bridge = |gx: i32, gy: i32| -> [u8; 4] {
                let bx = gx - (bridge_cap.rect[0] - ox);
                let by = gy - (bridge_cap.rect[1] - oy);
                let bi = ((by * bridge_cap.w + bx) * 4) as usize;
                [
                    bridge_cap.bgra[bi],
                    bridge_cap.bgra[bi + 1],
                    bridge_cap.bgra[bi + 2],
                    bridge_cap.bgra[bi + 3],
                ]
            };

            let mut min_r_bridge = 255u8;
            let mut max_r_bridge = 0u8;
            let mut min_b_bridge = 255u8;
            let mut max_b_bridge = 0u8;
            for x in (223..=229).chain(285..=291) {
                let p = sample_bridge(x, 231);
                assert!(
                    p[3] > 200,
                    "concave bridge pixel ({x}, 231) must be opaque (alpha={})",
                    p[3]
                );
                min_r_bridge = min_r_bridge.min(p[2]);
                max_r_bridge = max_r_bridge.max(p[2]);
                min_b_bridge = min_b_bridge.min(p[0]);
                max_b_bridge = max_b_bridge.max(p[0]);
            }
            assert!(
                (max_r_bridge - min_r_bridge) >= 30 && (max_b_bridge - min_b_bridge) >= 30,
                "concave bridge fillets at y=231 must preserve diagonal stripe variation (R={min_r_bridge}..{max_r_bridge}, B={min_b_bridge}..{max_b_bridge})"
            );
            for y in 231..=237 {
                let p = sample_bridge(223, y);
                assert!(
                    p[3] > 200,
                    "concave bridge pixel (223, {y}) must be opaque (alpha={})",
                    p[3]
                );
            }

            // Composite bridge + member uploads to verify that fused seam blending
            // never smears column 0 horizontally across neighbor panels or concave joins.
            let mut canvas = diag_img.clone();
            let blend_cap =
                |canvas: &mut image::RgbaImage, cap: &crate::compositor::test_capture::CapturedUpload| {
                    let lx0 = cap.rect[0] - ox;
                    let ly0 = cap.rect[1] - oy;
                    for y in 0..cap.h {
                        let cy = ly0 + y;
                        if cy < 0 || cy >= ch as i32 {
                            continue;
                        }
                        for x in 0..cap.w {
                            let cx = lx0 + x;
                            if cx < 0 || cx >= cw as i32 {
                                continue;
                            }
                            let si = ((y * cap.w + x) * 4) as usize;
                            let a = cap.bgra[si + 3] as u32;
                            if a == 0 {
                                continue;
                            }
                            let inv = 255 - a;
                            let p = canvas.get_pixel_mut(cx as u32, cy as u32);
                            p.0[0] = (cap.bgra[si + 2] as u32
                                + (p.0[0] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[1] = (cap.bgra[si + 1] as u32
                                + (p.0[1] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[2] = (cap.bgra[si] as u32 + (p.0[2] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[3] = 255;
                        }
                    }
                };
            blend_cap(&mut canvas, &bridge_cap);
            for mc in &member_caps {
                blend_cap(&mut canvas, mc);
            }

            let mut min_r_h = 255u8;
            let mut max_r_h = 0u8;
            let mut min_b_h = 255u8;
            let mut max_b_h = 0u8;
            for x in 240u32..=360 {
                let p = canvas.get_pixel(x, 140).0;
                min_r_h = min_r_h.min(p[0]);
                max_r_h = max_r_h.max(p[0]);
                min_b_h = min_b_h.min(p[2]);
                max_b_h = max_b_h.max(p[2]);
            }
            assert!(
                (max_r_h - min_r_h) >= 40 && (max_b_h - min_b_h) >= 40,
                "horizontal slice across fused seam band must preserve diagonal stripe variation (R={min_r_h}..{max_r_h}, B={min_b_h}..{max_b_h})"
            );

            let mut min_r_v = 255u8;
            let mut max_r_v = 0u8;
            for y in 160u32..=235 {
                let p = canvas.get_pixel(224, y).0;
                min_r_v = min_r_v.min(p[0]);
                max_r_v = max_r_v.max(p[0]);
            }
            assert!(
                (max_r_v - min_r_v) >= 25,
                "vertical slice from member into concave bridge must preserve diagonal stripe variation (R={min_r_v}..{max_r_v})"
            );

            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }

    #[test]
    fn controls_zero_tint_fused_with_clock_tinted_gradient_has_smooth_seam_and_concave_corner() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::glass::set_frosted_glass(self.0);
                crate::glass::set_liquid_glass(self.1);
            }
        }
        let _reset = Reset(
            crate::glass::frosted_glass(),
            crate::glass::liquid_glass(),
        );
        crate::glass::set_frosted_glass(true);
        crate::glass::set_liquid_glass(true);

        let temp_dir = std::env::temp_dir().join(format!(
            "lumen-fusion-affine-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        struct CleanupDir(std::path::PathBuf);
        impl Drop for CleanupDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = CleanupDir(temp_dir.clone());

        let (cw, ch) = (520usize, 360usize);
        let wall_path = temp_dir.join("cool-gradient.png");
        let mut wall_img = image::RgbaImage::new(cw as u32, ch as u32);
        for y in 0..ch {
            for x in 0..cw {
                let t = (x as f32 / cw as f32 * 0.6 + y as f32 / ch as f32 * 0.4).clamp(0.0, 1.0);
                let r = (32.0 + 28.0 * t).round() as u8;
                let g = (74.0 + 36.0 * t).round() as u8;
                let b = (182.0 + 40.0 * t).round() as u8;
                wall_img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
            }
        }
        wall_img.save(&wall_path).unwrap();

        unsafe {
            let _cap = crate::compositor::test_capture::start();
            let widget_glass = Glass::fixture_tiled_file(&wall_path, 14.0, 1.0);
            init(Rc::new(RefCell::new(Glass::fixture_tiled_file(
                &wall_path, 14.0, 1.0,
            ))));
            let instance = GetModuleHandleW(null());
            let probe = RECT {
                left: -(cw as i32) * 10,
                top: -(ch as i32) * 20,
                right: -(cw as i32) * 9,
                bottom: -(ch as i32) * 19,
            };
            let mon = MonitorFromRect(&probe, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            let ox = mi.rcMonitor.left - (cw as i32) * 10;
            let oy = mi.rcMonitor.top - (ch as i32) * 20;
            // Left: 0% tint (Control Center style); Right: 26% warm tint (Clock style),
            // shorter vertically to form a concave L-junction corner at (236, 196).
            let specs = [
                ([ox + 40, oy + 40, 200, 260], 0.0f32),
                ([ox + 232, oy + 40, 200, 160], 0.26f32),
            ];
            let warm_tint = [245.0f32, 165.0, 75.0];
            let mut windows = Vec::new();
            for (rect, _) in &specs {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Affine seam verify").as_ptr(),
                    WS_POPUP,
                    rect[0],
                    rect[1],
                    rect[2],
                    rect[3],
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                register_widget(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                prepare(hwnd, 30.0);
                windows.push(hwnd);
            }

            let paint_all = || {
                for (&hwnd, (rect, mix)) in windows.iter().zip(specs.iter()) {
                    let [_, _, w, h] = *rect;
                    let mut frame = Buffer::new(w, h, 1.0, 30.0).unwrap();
                    let brush = CreateSolidBrush(0x281e18);
                    FillRect(
                        frame.dc,
                        &RECT {
                            left: 0,
                            top: 0,
                            right: w,
                            bottom: h,
                        },
                        brush,
                    );
                    DeleteObject(brush);
                    widget_glass.draw_blended(frame.dc, hwnd, w, h, 255);
                    if *mix > 0.0 {
                        let px = frame.pixels_mut();
                        for p in px.chunks_exact_mut(4) {
                            p[0] = (p[0] as f32 * (1.0 - mix) + warm_tint[2] * mix).round() as u8;
                            p[1] = (p[1] as f32 * (1.0 - mix) + warm_tint[1] * mix).round() as u8;
                            p[2] = (p[2] as f32 * (1.0 - mix) + warm_tint[0] * mix).round() as u8;
                        }
                    }
                    frame.background_alpha_for(hwnd, 255);
                    frame.bevel_for(hwnd);
                    assert!(frame.present_with_alpha(hwnd, true));
                }
            };

            paint_all();
            refresh();
            assert!(windows.iter().all(|&h| fused(h)));
            paint_all();
            refresh();

            let bridge_hwnd = MANAGER.with(|m| {
                let guard = m.borrow();
                let s = guard.as_ref().unwrap();
                assert_eq!(s.bridges.len(), 1);
                s.bridges[0].hwnd
            });
            let bridge_cap =
                crate::compositor::test_capture::get(bridge_hwnd).expect("bridge upload");
            let member_caps: Vec<_> = windows
                .iter()
                .map(|&h| crate::compositor::test_capture::get(h).expect("member upload"))
                .collect();

            let mut canvas = wall_img.clone();
            let blend_cap =
                |canvas: &mut image::RgbaImage, cap: &crate::compositor::test_capture::CapturedUpload| {
                    let lx0 = cap.rect[0] - ox;
                    let ly0 = cap.rect[1] - oy;
                    for y in 0..cap.h {
                        let cy = ly0 + y;
                        if cy < 0 || cy >= ch as i32 {
                            continue;
                        }
                        for x in 0..cap.w {
                            let cx = lx0 + x;
                            if cx < 0 || cx >= cw as i32 {
                                continue;
                            }
                            let si = ((y * cap.w + x) * 4) as usize;
                            let a = cap.bgra[si + 3] as u32;
                            if a == 0 {
                                continue;
                            }
                            let inv = 255 - a;
                            let p = canvas.get_pixel_mut(cx as u32, cy as u32);
                            p.0[0] = (cap.bgra[si + 2] as u32
                                + (p.0[0] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[1] = (cap.bgra[si + 1] as u32
                                + (p.0[1] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[2] = (cap.bgra[si] as u32 + (p.0[2] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[3] = 255;
                        }
                    }
                };
            blend_cap(&mut canvas, &bridge_cap);
            for mc in &member_caps {
                blend_cap(&mut canvas, mc);
            }

            // Left interior (0% tint) vs right interior (26% warm tint) must remain distinct.
            let left_core = canvas.get_pixel(120, 110).0;
            let right_core = canvas.get_pixel(350, 110).0;
            assert!(
                right_core[0] > left_core[0] + 25 && left_core[2] > right_core[2] + 15,
                "expected 0% left ({left_core:?}) vs 26% warm right ({right_core:?})"
            );

            // Across the seam (x = 216..=256 at y = 110), adjacent pixel steps must stay smooth.
            let mut max_step = 0i32;
            for x in 216u32..256 {
                let p0 = canvas.get_pixel(x, 110).0;
                let p1 = canvas.get_pixel(x + 1, 110).0;
                for c in 0..3 {
                    max_step = max_step.max((p1[c] as i32 - p0[c] as i32).abs());
                }
            }
            assert!(
                max_step <= 10,
                "seam transition between 0% and 26% tint panes must be smooth (max_step={max_step})"
            );

            // Concave L-junction fillet (outside both member windows at x=242, y=202) must be
            // opaque (no corner notch) and hand over smoothly from the left member across x=234..242.
            let bx = 242 - (bridge_cap.rect[0] - ox);
            let by = 202 - (bridge_cap.rect[1] - oy);
            let bi = ((by * bridge_cap.w + bx) * 4) as usize;
            assert!(
                bridge_cap.bgra[bi + 3] > 200,
                "concave L-junction corner must be opaque without notch (alpha={})",
                bridge_cap.bgra[bi + 3]
            );
            let mut corner_step = 0i32;
            for x in 234u32..242 {
                let p0 = canvas.get_pixel(x, 202).0;
                let p1 = canvas.get_pixel(x + 1, 202).0;
                for c in 0..3 {
                    corner_step = corner_step.max((p1[c] as i32 - p0[c] as i32).abs());
                }
            }
            assert!(
                corner_step <= 12,
                "concave L-junction member-to-bridge handover must stay smooth (corner_step={corner_step})"
            );
            let fillet_px = canvas.get_pixel(242, 202).0;
            let neighbor_px = canvas.get_pixel(236, 202).0;
            let corner_diff = (0..3)
                .map(|c| (fillet_px[c] as i32 - neighbor_px[c] as i32).abs())
                .max()
                .unwrap_or(0);
            assert!(
                corner_diff <= 24,
                "concave corner ({fillet_px:?}) must transition smoothly from neighbor ({neighbor_px:?})"
            );

            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }

    #[test]
    fn high_tint_afterglow_controls_fused_with_wide_zero_tint_dock_has_no_seam_step() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::glass::set_frosted_glass(self.0);
                crate::glass::set_liquid_glass(self.1);
            }
        }
        let _reset = Reset(
            crate::glass::frosted_glass(),
            crate::glass::liquid_glass(),
        );
        crate::glass::set_frosted_glass(true);
        crate::glass::set_liquid_glass(true);

        let temp_dir = std::env::temp_dir().join(format!(
            "lumen-fusion-afterglow-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        struct CleanupDir(std::path::PathBuf);
        impl Drop for CleanupDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = CleanupDir(temp_dir.clone());

        let (cw, ch) = (1500usize, 420usize);
        let wall_path = temp_dir.join("wave-wallpaper.png");
        let mut wall_img = image::RgbaImage::new(cw as u32, ch as u32);
        for y in 0..ch {
            for x in 0..cw {
                let wx = (x as f32 * 0.032).sin() * 0.5 + 0.5;
                let wy = (y as f32 * 0.028).cos() * 0.5 + 0.5;
                let w = 0.65 * wx + 0.35 * wy;
                let r = (30.0 + 200.0 * w).round() as u8;
                let g = (40.0 + 185.0 * (1.0 - w * 0.7)).round() as u8;
                let b = (225.0 - 190.0 * w).round() as u8;
                wall_img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
            }
        }
        wall_img.save(&wall_path).unwrap();

        unsafe {
            let _cap = crate::compositor::test_capture::start();
            let widget_glass = Glass::fixture_tiled_file(&wall_path, 14.0, 1.0);
            init(Rc::new(RefCell::new(Glass::fixture_tiled_file(
                &wall_path, 14.0, 1.0,
            ))));
            let instance = GetModuleHandleW(null());
            let probe = RECT {
                left: -(cw as i32) * 10,
                top: -(ch as i32) * 20,
                right: -(cw as i32) * 9,
                bottom: -(ch as i32) * 19,
            };
            let mon = MonitorFromRect(&probe, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            GetMonitorInfoW(mon, &mut mi);
            let ox = mi.rcMonitor.left - (cw as i32) * 10;
            let oy = mi.rcMonitor.top - (ch as i32) * 20;
            // Left: 65% 2D gradient orange/lavender tint (Controls style);
            // Right: wide (1120px, so gw > 64) 0% tint pane (Dock style).
            let specs = [
                ([ox + 40, oy + 40, 260, 320], 0.65f32),
                ([ox + 292, oy + 40, 1120, 240], 0.0f32),
            ];
            let mut windows = Vec::new();
            for (rect, _) in &specs {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Afterglow wide dock verify").as_ptr(),
                    WS_POPUP,
                    rect[0],
                    rect[1],
                    rect[2],
                    rect[3],
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                register_widget(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                prepare(hwnd, 30.0);
                windows.push(hwnd);
            }

            let paint_all = || {
                for (&hwnd, (rect, mix)) in windows.iter().zip(specs.iter()) {
                    let [_, _, w, h] = *rect;
                    let mut frame = Buffer::new(w, h, 1.0, 30.0).unwrap();
                    let brush = CreateSolidBrush(0x281e18);
                    FillRect(
                        frame.dc,
                        &RECT {
                            left: 0,
                            top: 0,
                            right: w,
                            bottom: h,
                        },
                        brush,
                    );
                    DeleteObject(brush);
                    widget_glass.draw_blended(frame.dc, hwnd, w, h, 255);
                    if *mix > 0.0 {
                        let px = frame.pixels_mut();
                        for y in 0..h {
                            let ty = y as f32 / h as f32;
                            for x in 0..w {
                                let tx = x as f32 / w as f32;
                                let tr = 245.0 * (1.0 - ty) + 185.0 * ty;
                                let tg = 140.0 * (1.0 - tx) + 130.0 * tx;
                                let tb = 60.0 * (1.0 - ty) + 235.0 * ty;
                                let i = ((y * w + x) * 4) as usize;
                                px[i] = (px[i] as f32 * (1.0 - mix) + tb * mix).round() as u8;
                                px[i + 1] =
                                    (px[i + 1] as f32 * (1.0 - mix) + tg * mix).round() as u8;
                                px[i + 2] =
                                    (px[i + 2] as f32 * (1.0 - mix) + tr * mix).round() as u8;
                            }
                        }
                    }
                    frame.background_alpha_for(hwnd, 255);
                    frame.bevel_for(hwnd);
                    assert!(frame.present_with_alpha(hwnd, true));
                }
            };

            paint_all();
            refresh();
            assert!(windows.iter().all(|&h| fused(h)));
            paint_all();
            refresh();

            let dock_field =
                shared_material(windows[1], specs[1].0[2], specs[1].0[3], 1.0).expect("wide field");
            assert!(
                dock_field.width > 64,
                "wide dock field must exceed 64 grid cells (width={})",
                dock_field.width
            );
            let mut min_b = f32::MAX;
            let mut max_b = f32::MIN;
            for y in 0..dock_field.height {
                let v = dock_field.bgr[y * dock_field.width][0];
                min_b = min_b.min(v);
                max_b = max_b.max(v);
            }
            assert!(
                max_b - min_b > 12.0,
                "wide shared_material must sample real wave wallpaper instead of flat 128 fallback (delta range={min_b:.2}..{max_b:.2})"
            );

            let bridge_hwnd = MANAGER.with(|m| {
                let guard = m.borrow();
                let s = guard.as_ref().unwrap();
                assert_eq!(s.bridges.len(), 1);
                s.bridges[0].hwnd
            });
            let bridge_cap =
                crate::compositor::test_capture::get(bridge_hwnd).expect("bridge upload");
            let member_caps: Vec<_> = windows
                .iter()
                .map(|&h| crate::compositor::test_capture::get(h).expect("member upload"))
                .collect();

            let mut canvas = wall_img.clone();
            let blend_cap =
                |canvas: &mut image::RgbaImage, cap: &crate::compositor::test_capture::CapturedUpload| {
                    let lx0 = cap.rect[0] - ox;
                    let ly0 = cap.rect[1] - oy;
                    for y in 0..cap.h {
                        let cy = ly0 + y;
                        if cy < 0 || cy >= ch as i32 {
                            continue;
                        }
                        for x in 0..cap.w {
                            let cx = lx0 + x;
                            if cx < 0 || cx >= cw as i32 {
                                continue;
                            }
                            let si = ((y * cap.w + x) * 4) as usize;
                            let a = cap.bgra[si + 3] as u32;
                            if a == 0 {
                                continue;
                            }
                            let inv = 255 - a;
                            let p = canvas.get_pixel_mut(cx as u32, cy as u32);
                            p.0[0] = (cap.bgra[si + 2] as u32
                                + (p.0[0] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[1] = (cap.bgra[si + 1] as u32
                                + (p.0[1] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[2] = (cap.bgra[si] as u32 + (p.0[2] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[3] = 255;
                        }
                    }
                };
            blend_cap(&mut canvas, &bridge_cap);
            for mc in &member_caps {
                blend_cap(&mut canvas, mc);
            }

            // Left interior (65% tint) stays strongly shifted from underlying wave wallpaper,
            // while far-right interior (0% tint) stays close to the wallpaper.
            let left_diff = (0..3)
                .map(|c| {
                    (canvas.get_pixel(110, 140).0[c] as i32 - wall_img.get_pixel(110, 140).0[c] as i32)
                        .abs()
                })
                .max()
                .unwrap_or(0);
            let right_diff = (0..3)
                .map(|c| {
                    (canvas.get_pixel(900, 140).0[c] as i32 - wall_img.get_pixel(900, 140).0[c] as i32)
                        .abs()
                })
                .max()
                .unwrap_or(0);
            assert!(
                left_diff > 70 && right_diff < 45 && left_diff > right_diff + 40,
                "expected distinct 65% left tint (diff={left_diff}) vs 0% right dock (diff={right_diff})"
            );

            // Vertical seam row at y=140 crossing x=276..316 must have no hard step.
            let mut max_step = 0i32;
            for x in 276u32..316 {
                let p0 = canvas.get_pixel(x, 140).0;
                let p1 = canvas.get_pixel(x + 1, 140).0;
                for c in 0..3 {
                    max_step = max_step.max((p1[c] as i32 - p0[c] as i32).abs());
                }
            }
            assert!(
                max_step <= 8,
                "high-tint 65% Controls to 0% wide Dock seam must have no step (max_step={max_step})"
            );

            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }

    #[test]
    fn pure_overlay_high_tint_controls_and_zero_tint_dock_have_zero_seam_and_zero_stretch() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::glass::set_frosted_glass(self.0);
                crate::glass::set_liquid_glass(self.1);
            }
        }
        let _reset = Reset(
            crate::glass::frosted_glass(),
            crate::glass::liquid_glass(),
        );
        crate::glass::set_frosted_glass(true);
        crate::glass::set_liquid_glass(true);

        let temp_dir = std::env::temp_dir().join(format!(
            "lumen-fusion-pure-overlay-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        struct CleanupDir(std::path::PathBuf);
        impl Drop for CleanupDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = CleanupDir(temp_dir.clone());

        let (cw, ch) = (800usize, 400usize);
        let wall_path = temp_dir.join("diag-wave-wallpaper.png");
        let mut wall_img = image::RgbaImage::new(cw as u32, ch as u32);
        for y in 0..ch {
            for x in 0..cw {
                let phase = (x as f32 * 0.09 + y as f32 * 0.07).sin();
                let ridge = (phase * 0.5 + 0.5).powf(2.0);
                let r = (25.0 + 210.0 * ridge).round() as u8;
                let g = (45.0 + 185.0 * (1.0 - ridge * 0.65)).round() as u8;
                let b = (225.0 - 180.0 * ridge).round() as u8;
                wall_img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
            }
        }
        wall_img.save(&wall_path).unwrap();

        unsafe {
            let _cap = crate::compositor::test_capture::start();
            let widget_glass = Glass::fixture_tiled_file(&wall_path, 4.0, 1.0);
            init(Rc::new(RefCell::new(Glass::fixture_tiled_file(
                &wall_path, 4.0, 1.0,
            ))));
            let instance = GetModuleHandleW(null());
            let (ox, oy) = (-15000, -8400);
            let specs = [
                ([ox, oy, 269, 300], true),
                ([ox + 261, oy, 413, 300], false),
            ];
            let mut windows = Vec::new();
            for (rect, _) in &specs {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Pure overlay fusion verify").as_ptr(),
                    WS_POPUP,
                    rect[0],
                    rect[1],
                    rect[2],
                    rect[3],
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                register_widget(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                prepare(hwnd, 30.0);
                windows.push(hwnd);
            }

            let paint_all = || {
                for (&hwnd, (rect, is_controls)) in windows.iter().zip(specs.iter()) {
                    let [_, _, w, h] = *rect;
                    let mut frame = Buffer::new(w, h, 1.0, 30.0).unwrap();
                    widget_glass.draw_blended(frame.dc, hwnd, w, h, 255);
                    let is_fused = fused(hwnd);
                    let pane = if is_fused {
                        [0.0, 0.0, w as f32, h as f32]
                    } else {
                        [4.0, 4.0, w as f32 - 4.0, h as f32 - 4.0]
                    };
                    let r_rad = if is_fused { 0.0 } else { 30.0 };
                    frame.paint_frosted_chrome_for(hwnd, true, |dc| {
                        let c = crate::render::Canvas::new(dc, 1.0);
                        if *is_controls {
                            crate::surface::paint(&c, pane, "gradient:afterglow", 1.0);
                            c.coating(pane, r_rad);
                            c.draw_pane_overlay([w as u32, h as u32], 1.0, 1.0, false, pane);
                        } else {
                            c.draw_pane_overlay([w as u32, h as u32], 1.0, 0.0, false, pane);
                        }
                    });
                    frame.background_alpha_for(hwnd, 255);
                    assert!(frame.present_with_alpha(hwnd, true));
                }
            };

            paint_all();
            refresh();
            assert!(windows.iter().all(|&h| fused(h)));
            paint_all();
            refresh();

            let bridge_hwnd = MANAGER.with(|m| {
                let guard = m.borrow();
                let s = guard.as_ref().unwrap();
                assert_eq!(s.bridges.len(), 1);
                s.bridges[0].hwnd
            });
            let bridge_cap =
                crate::compositor::test_capture::get(bridge_hwnd).expect("bridge upload");
            let left_cap =
                crate::compositor::test_capture::get(windows[0]).expect("left upload");
            let right_cap =
                crate::compositor::test_capture::get(windows[1]).expect("right upload");

            let mut canvas = wall_img.clone();
            let blend_cap =
                |canvas: &mut image::RgbaImage, cap: &crate::compositor::test_capture::CapturedUpload| {
                    let lx0 = cap.rect[0] - ox;
                    let ly0 = cap.rect[1] - oy;
                    for y in 0..cap.h {
                        let cy = ly0 + y;
                        if cy < 0 || cy >= ch as i32 {
                            continue;
                        }
                        for x in 0..cap.w {
                            let cx = lx0 + x;
                            if cx < 0 || cx >= cw as i32 {
                                continue;
                            }
                            let si = ((y * cap.w + x) * 4) as usize;
                            let a = cap.bgra[si + 3] as u32;
                            if a == 0 {
                                continue;
                            }
                            let inv = 255 - a;
                            let p = canvas.get_pixel_mut(cx as u32, cy as u32);
                            p.0[0] = (cap.bgra[si + 2] as u32
                                + (p.0[0] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[1] = (cap.bgra[si + 1] as u32
                                + (p.0[1] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[2] = (cap.bgra[si] as u32 + (p.0[2] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[3] = 255;
                        }
                    }
                };
            blend_cap(&mut canvas, &bridge_cap);
            blend_cap(&mut canvas, &left_cap);
            blend_cap(&mut canvas, &right_cap);

            // 1. Across the vertical seam: discrete 2nd-difference kink across interior rows <= 8
            // (isolates any seam step/discontinuity without conflating smooth diagonal wallpaper slope).
            let mut max_seam_kink = 0i32;
            for y in 40..260 {
                let p263 = canvas.get_pixel(263, y as u32).0;
                let p264 = canvas.get_pixel(264, y as u32).0;
                let p265 = canvas.get_pixel(265, y as u32).0;
                let p266 = canvas.get_pixel(266, y as u32).0;
                for c in 0..3 {
                    let d_left = p264[c] as i32 - p263[c] as i32;
                    let d_seam = p265[c] as i32 - p264[c] as i32;
                    let d_right = p266[c] as i32 - p265[c] as i32;
                    let kink = (d_seam - d_left).abs().max((d_seam - d_right).abs());
                    max_seam_kink = max_seam_kink.max(kink);
                }
                let li = ((y * left_cap.w + (left_cap.w - 5)) * 4) as usize;
                let ri = ((y * right_cap.w + 4) * 4) as usize;
                let ri_next = ((y * right_cap.w + 5) * 4) as usize;
                for c in 0..3 {
                    let d_seam = right_cap.bgra[ri + c] as i32 - left_cap.bgra[li + c] as i32;
                    let d_right = right_cap.bgra[ri_next + c] as i32 - right_cap.bgra[ri + c] as i32;
                    max_seam_kink = max_seam_kink.max((d_seam - d_right).abs());
                }
            }
            assert!(
                max_seam_kink <= 8,
                "pure overlay seam kink must be <= 8 across interior rows (max_seam_kink={max_seam_kink})"
            );

            // 2. Inside the blend zone on left widget (x in (w - 110)..(w - 15)):
            // emerging diagonal ridges must have genuine diagonal variation rather than a flat stretch bar.
            let lw = left_cap.w;
            let mut step_sum = 0.0f32;
            let mut step_count = 0usize;
            let mut row_phase_diff = 0.0f32;
            let mut row_phase_count = 0usize;
            for y in 80..220 {
                for x in (lw - 110)..(lw - 15) {
                    let i0 = ((y * lw + x) * 4) as usize;
                    let i1 = ((y * lw + x + 1) * 4) as usize;
                    let diff = (0..3)
                        .map(|c| (left_cap.bgra[i1 + c] as f32 - left_cap.bgra[i0 + c] as f32).abs())
                        .fold(0.0f32, f32::max);
                    step_sum += diff;
                    step_count += 1;
                    if y + 32 < 220 {
                        let i2 = (((y + 32) * lw + x) * 4) as usize;
                        let vdiff = (0..3)
                            .map(|c| {
                                (left_cap.bgra[i2 + c] as f32 - left_cap.bgra[i0 + c] as f32).abs()
                            })
                            .fold(0.0f32, f32::max);
                        row_phase_diff += vdiff;
                        row_phase_count += 1;
                    }
                }
            }
            let mut interior_dx_sum = 0.0f32;
            let mut interior_dx_count = 0usize;
            for y in 80..220 {
                for x in 12..52 {
                    let i0 = ((y * lw + x) * 4) as usize;
                    let i1 = ((y * lw + x + 1) * 4) as usize;
                    let diff = (0..3)
                        .map(|c| (left_cap.bgra[i1 + c] as f32 - left_cap.bgra[i0 + c] as f32).abs())
                        .fold(0.0f32, f32::max);
                    interior_dx_sum += diff;
                    interior_dx_count += 1;
                }
            }
            let mean_dx = step_sum / step_count.max(1) as f32;
            let mean_dy = row_phase_diff / row_phase_count.max(1) as f32;
            let interior_dx = interior_dx_sum / interior_dx_count.max(1) as f32;
            // Compare retained texture contrast, not a required contrast boost.
            // Bounded rim magnification now preserves more texture in the
            // reference strip too; that must not count as a stretched bridge.
            assert!(
                mean_dx > 0.6 && mean_dy > 4.0 && mean_dx >= interior_dx * 0.90,
                "blend zone must reveal genuine diagonal wallpaper ridges (mean_dx={mean_dx:.3}, mean_dy={mean_dy:.3}, interior_dx={interior_dx:.3})"
            );

            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }

    #[test]
    fn fused_frost_strength_blends_smoothly_across_seam_from_zero_to_full() {
        let _ui = crate::native_ui_test_lock();
        struct Reset(bool, bool);
        impl Drop for Reset {
            fn drop(&mut self) {
                crate::glass::set_frosted_glass(self.0);
                crate::glass::set_liquid_glass(self.1);
            }
        }
        let _reset = Reset(
            crate::glass::frosted_glass(),
            crate::glass::liquid_glass(),
        );
        crate::glass::set_frosted_glass(true);
        crate::glass::set_liquid_glass(true);

        let temp_dir = std::env::temp_dir().join(format!(
            "lumen-fusion-frost-blend-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();
        struct CleanupDir(std::path::PathBuf);
        impl Drop for CleanupDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = CleanupDir(temp_dir.clone());

        let (cw, ch) = (800usize, 400usize);
        let wall_path = temp_dir.join("frost-stripe-wallpaper.png");
        let mut wall_img = image::RgbaImage::new(cw as u32, ch as u32);
        for y in 0..ch {
            for x in 0..cw {
                let v = (x as f32 * 0.22 + y as f32 * 0.12).sin() * 0.5 + 0.5;
                let r = (20.0 + 220.0 * v).round() as u8;
                let g = (40.0 + 190.0 * (1.0 - v)).round() as u8;
                let b = (220.0 - 160.0 * v).round() as u8;
                wall_img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, 255]));
            }
        }
        wall_img.save(&wall_path).unwrap();

        unsafe {
            let _cap = crate::compositor::test_capture::start();
            let widget_glass = Glass::fixture_tiled_file(&wall_path, 18.0, 1.0);
            init(Rc::new(RefCell::new(Glass::fixture_tiled_file(
                &wall_path, 18.0, 1.0,
            ))));
            let instance = GetModuleHandleW(null());
            let (ox, oy) = (-15000, -8400);
            let specs = [
                ([ox, oy, 269, 300], 0u8),
                ([ox + 261, oy, 413, 300], 255u8),
            ];
            let mut windows = Vec::new();
            for (rect, frost) in &specs {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Frost blend verify").as_ptr(),
                    WS_POPUP,
                    rect[0],
                    rect[1],
                    rect[2],
                    rect[3],
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                register_widget(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                prepare(hwnd, 30.0);
                frost_strength(hwnd, *frost);
                windows.push(hwnd);
            }

            let paint_all = || {
                for (&hwnd, (rect, frost)) in windows.iter().zip(specs.iter()) {
                    let [_, _, w, h] = *rect;
                    let mut frame = Buffer::new(w, h, 1.0, 30.0).unwrap();
                    if let Some(blend) = shared_frost(hwnd, w, h, *frost) {
                        widget_glass.draw_with_frost(frame.dc, hwnd, w, h, |x, y| {
                            blend.sample_local(x, y)
                        });
                    } else {
                        widget_glass.draw_blended(frame.dc, hwnd, w, h, *frost);
                    }
                    let is_fused = fused(hwnd);
                    let pane = if is_fused {
                        [0.0, 0.0, w as f32, h as f32]
                    } else {
                        [4.0, 4.0, w as f32 - 4.0, h as f32 - 4.0]
                    };
                    frame.paint_frosted_chrome_for(hwnd, true, |dc| {
                        let c = crate::render::Canvas::new(dc, 1.0);
                        c.draw_pane_overlay([w as u32, h as u32], 1.0, 0.0, false, pane);
                    });
                    frame.background_alpha_for(hwnd, 255);
                    assert!(frame.present_with_alpha(hwnd, true));
                }
            };

            paint_all();
            refresh();
            assert!(windows.iter().all(|&h| fused(h)));
            paint_all();
            refresh();

            let bridge_hwnd = MANAGER.with(|m| {
                let guard = m.borrow();
                let s = guard.as_ref().unwrap();
                assert_eq!(s.bridges.len(), 1);
                s.bridges[0].hwnd
            });
            let bridge_cap =
                crate::compositor::test_capture::get(bridge_hwnd).expect("bridge upload");
            let left_cap =
                crate::compositor::test_capture::get(windows[0]).expect("left upload");
            let right_cap =
                crate::compositor::test_capture::get(windows[1]).expect("right upload");

            let mut canvas = wall_img.clone();
            let blend_cap =
                |canvas: &mut image::RgbaImage, cap: &crate::compositor::test_capture::CapturedUpload| {
                    let lx0 = cap.rect[0] - ox;
                    let ly0 = cap.rect[1] - oy;
                    for y in 0..cap.h {
                        let cy = ly0 + y;
                        if cy < 0 || cy >= ch as i32 {
                            continue;
                        }
                        for x in 0..cap.w {
                            let cx = lx0 + x;
                            if cx < 0 || cx >= cw as i32 {
                                continue;
                            }
                            let si = ((y * cap.w + x) * 4) as usize;
                            let a = cap.bgra[si + 3] as u32;
                            if a == 0 {
                                continue;
                            }
                            let inv = 255 - a;
                            let p = canvas.get_pixel_mut(cx as u32, cy as u32);
                            p.0[0] = (cap.bgra[si + 2] as u32
                                + (p.0[0] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[1] = (cap.bgra[si + 1] as u32
                                + (p.0[1] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[2] = (cap.bgra[si] as u32 + (p.0[2] as u32 * inv + 127) / 255)
                                .min(255) as u8;
                            p.0[3] = 255;
                        }
                    }
                };
            blend_cap(&mut canvas, &bridge_cap);
            blend_cap(&mut canvas, &left_cap);
            blend_cap(&mut canvas, &right_cap);

            let red_contrast = |cap: &crate::compositor::test_capture::CapturedUpload,
                                x0: i32,
                                x1: i32,
                                y: i32| {
                let mut mn = 255i32;
                let mut mx = 0i32;
                for x in x0..x1 {
                    let r = cap.bgra[((y * cap.w + x) * 4 + 2) as usize] as i32;
                    mn = mn.min(r);
                    mx = mx.max(r);
                }
                mx - mn
            };
            let left_interior = red_contrast(&left_cap, 30, 90, 150);
            let right_interior = red_contrast(&right_cap, 150, 250, 150);
            assert!(
                left_interior > 120,
                "0% frost left interior must keep sharp stripe contrast (contrast={left_interior})"
            );
            assert!(
                right_interior < 35,
                "100% frost right interior must be strongly blurred (contrast={right_interior})"
            );

            let left_seam = red_contrast(&left_cap, left_cap.w - 35, left_cap.w - 4, 150);
            let right_seam = red_contrast(&right_cap, 4, 35, 150);
            assert!(
                left_seam > right_interior + 25 && left_seam < left_interior - 25,
                "left seam zone must have intermediate (~50%) contrast (left_seam={left_seam}, left_interior={left_interior}, right_interior={right_interior})"
            );
            assert!(
                right_seam > right_interior + 25 && right_seam < left_interior - 25,
                "right seam zone must have intermediate (~50%) contrast (right_seam={right_seam}, left_interior={left_interior}, right_interior={right_interior})"
            );

            let mut max_seam_kink = 0i32;
            for y in 60..240 {
                let p263 = canvas.get_pixel(263, y as u32).0;
                let p264 = canvas.get_pixel(264, y as u32).0;
                let p265 = canvas.get_pixel(265, y as u32).0;
                let p266 = canvas.get_pixel(266, y as u32).0;
                for c in 0..3 {
                    let d_left = p264[c] as i32 - p263[c] as i32;
                    let d_seam = p265[c] as i32 - p264[c] as i32;
                    let d_right = p266[c] as i32 - p265[c] as i32;
                    max_seam_kink = max_seam_kink
                        .max((d_seam - d_left).abs())
                        .max((d_seam - d_right).abs());
                }
                let li = ((y * left_cap.w + (left_cap.w - 5)) * 4) as usize;
                let ri = ((y * right_cap.w + 4) * 4) as usize;
                let ri_next = ((y * right_cap.w + 5) * 4) as usize;
                for c in 0..3 {
                    let d_seam = right_cap.bgra[ri + c] as i32 - left_cap.bgra[li + c] as i32;
                    let d_right = right_cap.bgra[ri_next + c] as i32 - right_cap.bgra[ri + c] as i32;
                    max_seam_kink = max_seam_kink.max((d_seam - d_right).abs());
                }
            }
            assert!(
                max_seam_kink <= 8,
                "frost seam transition must have no abrupt discontinuity (max_seam_kink={max_seam_kink})"
            );

            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }

    #[test]
    fn mixed_opacity_zero_neighbor_has_smooth_rounded_outer_corners_and_concave_handover() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            let _cap = crate::compositor::test_capture::start();
            init(Rc::new(RefCell::new(Glass::new(0., 1.))));
            let instance = GetModuleHandleW(null());
            let (ox, oy) = (-15000, -8400);
            // Dock (255 opacity) below Music (0 opacity), forming a concave L-junction corner
            // on the left of Music's bottom-left rounded corner.
            let specs = [
                ([ox, oy + 374, 848, 391], 255u8),
                ([ox + 391, oy + 8, 457, 374], 0u8),
            ];
            let mut windows = Vec::new();
            for (rect, op) in &specs {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Mixed opacity corner verify").as_ptr(),
                    WS_POPUP,
                    rect[0],
                    rect[1],
                    rect[2],
                    rect[3],
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                register_widget(hwnd);
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                prepare(hwnd, 34.0);
                opacity(hwnd, *op);
                windows.push(hwnd);
            }

            let paint_all = || {
                for (&hwnd, (rect, op)) in windows.iter().zip(specs.iter()) {
                    let [_, _, w, h] = *rect;
                    let mut frame = Buffer::new(w, h, 1.0, 34.0).unwrap();
                    let brush = CreateSolidBrush(0x405060);
                    FillRect(
                        frame.dc,
                        &RECT {
                            left: 0,
                            top: 0,
                            right: w,
                            bottom: h,
                        },
                        brush,
                    );
                    DeleteObject(brush);
                    frame.background_alpha_for(hwnd, *op);
                    frame.bevel_for(hwnd);
                    assert!(frame.present_with_alpha(hwnd, true));
                }
            };

            paint_all();
            refresh();
            assert!(windows.iter().all(|&h| fused(h)));
            paint_all();
            refresh();

            let bridge_hwnd = MANAGER.with(|m| {
                let guard = m.borrow();
                let s = guard.as_ref().unwrap();
                assert_eq!(s.bridges.len(), 1);
                s.bridges[0].hwnd
            });
            let bridge_cap =
                crate::compositor::test_capture::get(bridge_hwnd).expect("bridge upload");
            let dock_cap =
                crate::compositor::test_capture::get(windows[0]).expect("dock upload");
            let music_cap =
                crate::compositor::test_capture::get(windows[1]).expect("music upload");

            // 1. Inside Music's bottom-left rounded corner (local x=4..25, y=345..369):
            // Bridge must NOT draw a square 3px underlap box under the transparent member!
            for ly in 340..370 {
                for lx in 0..30 {
                    let gx = specs[1].0[0] + lx;
                    let gy = specs[1].0[1] + ly;
                    let bx = gx - bridge_cap.rect[0];
                    let by = gy - bridge_cap.rect[1];
                    let bi = ((by * bridge_cap.w + bx) * 4) as usize;
                    assert_eq!(
                        bridge_cap.bgra[bi + 2],
                        0,
                        "bridge must not leak visible foreground underlap inside transparent member corner at local ({lx}, {ly})"
                    );
                }
            }

            // 2. Across the seam between Dock (y=378) and Music (y=378) near Music's left edge (lx=6..45),
            // Dock's top row and Music's bottom row must agree within <= 4 alpha units (no 87->50 corner notch).
            for lx in 6..45 {
                let gx = specs[1].0[0] + lx;
                let gy_music = specs[1].0[1] + 369; // Music local y = 369 (gy = oy + 377)
                let gy_dock = specs[0].0[1] + 4;    // Dock local y = 4 (gy = oy + 378)
                let mx = gx - music_cap.rect[0];
                let my = gy_music - music_cap.rect[1];
                let dx = gx - dock_cap.rect[0];
                let dy = gy_dock - dock_cap.rect[1];
                let ma = music_cap.bgra[((my * music_cap.w + mx) * 4 + 3) as usize] as i32;
                let da = dock_cap.bgra[((dy * dock_cap.w + dx) * 4 + 3) as usize] as i32;
                assert!(
                    (ma - da).abs() <= 6,
                    "opacity across seam near rounded corner must match smoothly without corner notch at lx={lx}: music_a={ma}, dock_a={da}"
                );
            }

            // 3. Handover from Bridge concave fillet (gx = ox + 390) into Music left edge (gx = ox + 395)
            // at gy = oy + 365 must have smooth alpha without a sudden jump.
            let gy = oy + 365;
            let bx = (ox + 390) - bridge_cap.rect[0];
            let by = gy - bridge_cap.rect[1];
            let ba = bridge_cap.bgra[((by * bridge_cap.w + bx) * 4 + 3) as usize] as i32;
            let mx = (ox + 395) - music_cap.rect[0];
            let my = gy - music_cap.rect[1];
            let ma = music_cap.bgra[((my * music_cap.w + mx) * 4 + 3) as usize] as i32;
            assert!(
                (ba - ma).abs() <= 16,
                "concave fillet to member edge alpha handover must be smooth: bridge_a={ba}, music_a={ma}"
            );

            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }

    #[test]
    fn unfuse_drag_threshold_scales_with_dpi() {
        assert!(!exceeds_unfuse_threshold(0, 0, 1.0));
        assert!(!exceeds_unfuse_threshold(10, 0, 1.0));
        assert!(!exceeds_unfuse_threshold(15, 0, 1.0));
        assert!(!exceeds_unfuse_threshold(11, 11, 1.0));
        assert!(exceeds_unfuse_threshold(16, 0, 1.0));
        assert!(exceeds_unfuse_threshold(-16, 0, 1.0));
        assert!(exceeds_unfuse_threshold(12, 12, 1.0));

        assert!(!exceeds_unfuse_threshold(23, 0, 1.5));
        assert!(exceeds_unfuse_threshold(24, 0, 1.5));
    }

    #[test]
    fn fused_widget_requires_drag_distance_before_detaching_in_both_native_and_capture_paths() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            init(Rc::new(RefCell::new(Glass::new(12., 1.))));
            let instance = GetModuleHandleW(null());
            let mut windows = Vec::new();
            for (x, y, w, h) in [(100, 100, 240, 208), (332, 100, 240, 208)] {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Lumen unfuse threshold test").as_ptr(),
                    WS_POPUP,
                    x - 20000,
                    y - 20000,
                    w,
                    h,
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                MANAGER.with(|m| {
                    m.borrow_mut()
                        .as_mut()
                        .unwrap()
                        .members
                        .insert(hwnd as usize, read_member(hwnd, 30.));
                });
                SetWindowSubclass(hwnd, Some(widget_proc), SUBCLASS, 0);
                windows.push(hwnd);
            }
            refresh();
            assert!(fused(windows[0]));
            assert!(fused(windows[1]));
            let anchor = window_rect(windows[0]);
            let dpi = (GetDpiForWindow(windows[0]) as f32 / 96.).max(1.);
            let small_dx = (8. * dpi).round() as i32;
            let small_dy = (6. * dpi).round() as i32;
            let large_dx = (24. * dpi).round() as i32;

            // 1. Native HTCAPTION / WM_ENTERSIZEMOVE + WM_MOVING path:
            // Clicking/holding and dragging < 16 DIP keeps the widget fused and stationary.
            SendMessageW(windows[0], WM_ENTERSIZEMOVE, 0, 0);
            refresh();
            assert!(fused(windows[0]));
            assert!(!is_moving(windows[0]));

            let mut small = RECT {
                left: anchor.left + small_dx,
                top: anchor.top + small_dy,
                right: anchor.right + small_dx,
                bottom: anchor.bottom + small_dy,
            };
            SendMessageW(windows[0], WM_MOVING, 0, &mut small as *mut _ as isize);
            refresh();
            assert_eq!([small.left, small.top], [anchor.left, anchor.top]);
            assert!(fused(windows[0]));
            assert!(!is_moving(windows[0]));

            SendMessageW(windows[0], WM_EXITSIZEMOVE, 0, 0);
            refresh();
            assert!(fused(windows[0]));

            // Dragging >= 16 DIP detaches and moves the widget cleanly.
            SendMessageW(windows[0], WM_ENTERSIZEMOVE, 0, 0);
            let mut large = RECT {
                left: anchor.left - large_dx,
                top: anchor.top,
                right: anchor.right - large_dx,
                bottom: anchor.bottom,
            };
            SendMessageW(windows[0], WM_MOVING, 0, &mut large as *mut _ as isize);
            refresh();
            assert!(!fused(windows[0]));
            assert!(is_moving(windows[0]));
            assert_eq!(large.left, anchor.left - large_dx);
            SendMessageW(windows[0], WM_EXITSIZEMOVE, 0, 0);
            assert!(!is_moving(windows[0]));

            // Restore position to re-fuse both widgets for the capture-based test.
            SetWindowPos(
                windows[0],
                null_mut(),
                anchor.left,
                anchor.top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            refresh();
            assert!(fused(windows[0]));

            // 2. Capture-based drag path (Calendar / Controls):
            // Dragging < 16 DIP keeps the widget fused and stationary.
            let small_cap = RECT {
                left: anchor.left + small_dx,
                top: anchor.top + small_dy,
                right: anchor.right + small_dx,
                bottom: anchor.bottom + small_dy,
            };
            move_captured_widget(windows[0], small_cap);
            refresh();
            let after_small = window_rect(windows[0]);
            assert_eq!([after_small.left, after_small.top], [anchor.left, anchor.top]);
            assert!(fused(windows[0]));
            assert!(!is_moving(windows[0]));

            // Dragging >= 16 DIP detaches and moves the captured widget.
            let large_cap = RECT {
                left: anchor.left - large_dx,
                top: anchor.top,
                right: anchor.right - large_dx,
                bottom: anchor.bottom,
            };
            move_captured_widget(windows[0], large_cap);
            refresh();
            let after_large = window_rect(windows[0]);
            assert_eq!(after_large.left, anchor.left - large_dx);
            assert!(!fused(windows[0]));
            assert!(is_moving(windows[0]));

            SendMessageW(windows[0], WM_CAPTURECHANGED, 0, 0);
            assert!(!is_moving(windows[0]));

            // 3. Standalone (unfused) widget still moves immediately on small drags.
            assert!(!fused(windows[0]));
            let solo_rect = window_rect(windows[0]);
            let solo_step = RECT {
                left: solo_rect.left - 6,
                top: solo_rect.top,
                right: solo_rect.right - 6,
                bottom: solo_rect.bottom,
            };
            move_captured_widget(windows[0], solo_step);
            let after_solo = window_rect(windows[0]);
            assert_eq!(after_solo.left, solo_rect.left - 6);
            SendMessageW(windows[0], WM_CAPTURECHANGED, 0, 0);

            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }

    #[test]
    fn fused_widget_requires_drag_distance_before_detaching_on_resize() {
        let _ui = crate::native_ui_test_lock();
        unsafe {
            init(Rc::new(RefCell::new(Glass::new(12., 1.))));
            let instance = GetModuleHandleW(null());
            let mut windows = Vec::new();
            for (x, y, w, h) in [(100, 100, 240, 208), (332, 100, 240, 208)] {
                let hwnd = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOOLWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("Lumen unfuse resize threshold test").as_ptr(),
                    WS_POPUP,
                    x - 20000,
                    y - 20000,
                    w,
                    h,
                    null_mut(),
                    null_mut(),
                    instance,
                    null(),
                );
                assert!(!hwnd.is_null());
                ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                MANAGER.with(|m| {
                    m.borrow_mut()
                        .as_mut()
                        .unwrap()
                        .members
                        .insert(hwnd as usize, read_member(hwnd, 30.));
                });
                SetWindowSubclass(hwnd, Some(widget_proc), SUBCLASS, 0);
                windows.push(hwnd);
            }
            refresh();
            assert!(fused(windows[0]));
            assert!(fused(windows[1]));
            let anchor = window_rect(windows[0]);
            let dpi = (GetDpiForWindow(windows[0]) as f32 / 96.).max(1.);
            let small_dx = (8. * dpi).round() as i32;
            let large_dx = (24. * dpi).round() as i32;
            let min_size = [(160. * dpi).round() as i32, (112. * dpi).round() as i32];

            // 1. Native WM_NCLBUTTONDOWN(HTRIGHT) + WM_ENTERSIZEMOVE + WM_SIZING path:
            // Clicking/holding border and dragging < 16 DIP keeps widget fused and unresized.
            SendMessageW(windows[0], WM_NCLBUTTONDOWN, HTRIGHT as usize, 0);
            SendMessageW(windows[0], WM_ENTERSIZEMOVE, 0, 0);
            refresh();
            assert!(fused(windows[0]));
            assert!(!is_moving(windows[0]));

            let mut small_sizing = RECT {
                left: anchor.left,
                top: anchor.top,
                right: anchor.right + small_dx,
                bottom: anchor.bottom,
            };
            SendMessageW(
                windows[0],
                WM_SIZING,
                WMSZ_RIGHT as usize,
                &mut small_sizing as *mut _ as isize,
            );
            refresh();
            assert_eq!(small_sizing.right, anchor.right);
            assert!(fused(windows[0]));
            assert!(!is_moving(windows[0]));

            SendMessageW(windows[0], WM_EXITSIZEMOVE, 0, 0);
            refresh();
            assert!(fused(windows[0]));

            // Dragging border >= 16 DIP detaches and allows resize.
            SendMessageW(windows[0], WM_NCLBUTTONDOWN, HTRIGHT as usize, 0);
            SendMessageW(windows[0], WM_ENTERSIZEMOVE, 0, 0);
            let mut large_sizing = RECT {
                left: anchor.left,
                top: anchor.top,
                right: anchor.right + large_dx,
                bottom: anchor.bottom,
            };
            SendMessageW(
                windows[0],
                WM_SIZING,
                WMSZ_RIGHT as usize,
                &mut large_sizing as *mut _ as isize,
            );
            refresh();
            assert_eq!(large_sizing.right, anchor.right + large_dx);
            assert!(!fused(windows[0]));
            assert!(is_moving(windows[0]));
            SendMessageW(windows[0], WM_EXITSIZEMOVE, 0, 0);
            assert!(!is_moving(windows[0]));

            // 2. Captured resize (WM_NCLBUTTONDOWN + snap_resize_rect) path (Dock / Calendar / Controls):
            refresh();
            assert!(fused(windows[0]));
            SendMessageW(windows[0], WM_NCLBUTTONDOWN, HTRIGHT as usize, 0);
            refresh();
            assert!(fused(windows[0]));
            assert!(!is_moving(windows[0]));

            let mut small_cap = RECT {
                left: anchor.left,
                top: anchor.top,
                right: anchor.right + small_dx,
                bottom: anchor.bottom,
            };
            assert!(!snap_resize_rect(windows[0], &mut small_cap, HTRIGHT, min_size));
            refresh();
            assert_eq!(small_cap.right, anchor.right);
            assert!(fused(windows[0]));
            assert!(!is_moving(windows[0]));

            let mut large_cap = RECT {
                left: anchor.left,
                top: anchor.top,
                right: anchor.right + large_dx,
                bottom: anchor.bottom,
            };
            assert!(snap_resize_rect(windows[0], &mut large_cap, HTRIGHT, min_size));
            refresh();
            assert_eq!(large_cap.right, anchor.right + large_dx);
            assert!(!fused(windows[0]));
            assert!(is_moving(windows[0]));

            SendMessageW(windows[0], WM_CAPTURECHANGED, 0, 0);
            assert!(!is_moving(windows[0]));

            // 3. Standalone (unfused) widget resizes immediately on small drags.
            SetWindowPos(
                windows[0],
                null_mut(),
                anchor.left - 400,
                anchor.top,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            refresh();
            assert!(!fused(windows[0]));
            let solo_anchor = window_rect(windows[0]);
            SendMessageW(windows[0], WM_NCLBUTTONDOWN, HTRIGHT as usize, 0);
            let mut solo_resize = RECT {
                left: solo_anchor.left,
                top: solo_anchor.top,
                right: solo_anchor.right + 6,
                bottom: solo_anchor.bottom,
            };
            assert!(snap_resize_rect(windows[0], &mut solo_resize, HTRIGHT, min_size));
            assert_eq!(solo_resize.right, solo_anchor.right + 6);
            SendMessageW(windows[0], WM_CAPTURECHANGED, 0, 0);

            for hwnd in windows {
                DestroyWindow(hwnd);
            }
            shutdown();
        }
    }
}
