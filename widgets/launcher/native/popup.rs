//! Glass menus painted inside the dock; no system-menu visual mismatch.
use super::*;
pub(super) const EDIT_ID: usize = 201;
pub(super) const POPUP_KEY: u32 = WM_APP + 6;
#[derive(Clone)]
pub(super) enum Kind {
    Add,
    Item(String),
    Rename(String),
}
#[derive(Clone)]
pub(super) enum Action {
    Files,
    Folder,
    System(bool),
    Open(String),
    EmptyRecycleBin(String),
    Rename(String),
    Icon(String),
    ResetIcon(String),
    Groups(String),
    Ungroup(String),
    Remove(String),
    Save(String),
    Pin(String),
}
pub(super) struct Row {
    label: String,
    action: Action,
    icon: u8,
    enabled: bool,
    danger: bool,
}
pub(super) struct Popup {
    pub rect: [f32; 4],
    kind: Kind,
    title: String,
    subtitle: String,
    rows: Vec<Row>,
    pub hover: Option<usize>,
    pub pressed: Option<usize>,
    hover_tweens: Vec<crate::motion::Tween>,
    open_alpha: crate::motion::Tween,
    open_scale: crate::motion::Tween,
    scroll: usize,
    slots: usize,
    pub edit: HWND,
    pub brush: HBRUSH,
}
impl Drop for Popup {
    fn drop(&mut self) {
        unsafe {
            if !self.edit.is_null() {
                DestroyWindow(self.edit);
            }
            if !self.brush.is_null() {
                DeleteObject(self.brush);
            }
        }
    }
}
impl Popup {
    fn hit(&self, p: [f32; 2]) -> Option<usize> {
        if matches!(self.kind, Kind::Rename(_)) {
            return (p[0] >= self.rect[0] + 8.
                && p[0] <= self.rect[2] - 8.
                && p[1] >= self.rect[1] + 110.
                && p[1] < self.rect[3] - 8.)
                .then_some(0);
        }
        if p[0] < self.rect[0] + 8. || p[0] > self.rect[2] - 8. || p[1] < self.rect[1] + 60. {
            return None;
        }
        let row = ((p[1] - self.rect[1] - 60.) / 32.) as usize;
        let index = row + self.scroll;
        (row < self.slots && index < self.rows.len() && self.rows[index].enabled).then_some(index)
    }
    fn inside(&self, p: [f32; 2]) -> bool {
        p[0] >= self.rect[0] && p[0] <= self.rect[2] && p[1] >= self.rect[1] && p[1] <= self.rect[3]
    }
    fn retarget_hover(&mut self, now: f64) {
        const EASE: [f32; 4] = [0.25, 0.1, 0.25, 1.];
        for (i, tween) in self.hover_tweens.iter_mut().enumerate() {
            let active = self.hover == Some(i) || self.pressed == Some(i);
            tween.target(if active { 1. } else { 0. }, now, 180., EASE);
        }
    }
    fn row_hover(&self, index: usize, now: f64) -> f32 {
        self.hover_tweens.get(index).map_or(0., |t| t.sample(now))
    }
    pub(super) fn moving(&self, now: f64) -> bool {
        self.open_alpha.moving(now)
            || self.open_scale.moving(now)
            || self.hover_tweens.iter().any(|t| t.moving(now))
    }
}
unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    parent: usize,
) -> LRESULT {
    if message == WM_KEYDOWN && (wparam == 13 || wparam == 27) {
        PostMessageW(parent as HWND, POPUP_KEY, wparam, 0);
        return 0;
    }
    let result = DefSubclassProc(hwnd, message, wparam, lparam);
    if matches!(
        message,
        WM_CHAR | WM_KEYUP | WM_LBUTTONUP | WM_MOUSEMOVE | WM_SETFOCUS | WM_KILLFOCUS
    ) {
        PostMessageW(parent as HWND, WIDGET_REDRAW, 0, 0);
    }
    result
}
impl App {
    pub(super) unsafe fn sync_desktop_items(&mut self, hwnd: HWND) {
        if self.moving_window || self.tile_drag.is_some() { return; }
        let changed = if self.dock.sync_desktop {
            let fresh = self.desktop_watch.is_none();
            let watch = self.desktop_watch.get_or_insert_with(crate::desktop_sync::Watch::new);
            (fresh || watch.changed()) && watch.sync(&mut self.dock)
        } else {
            self.desktop_watch = None;
            let count = self.dock.items.len();
            self.dock.items.retain(|i| !i.id.starts_with("desktop-sync:"));
            count != self.dock.items.len()
        };
        if changed {
            self.search_cache.get_mut().take();
            let ids: std::collections::HashSet<_> = self.dock.items.iter().map(|i| i.id.clone()).collect();
            self.icons.retain(|id, _| ids.contains(id));
            self.scaled_icons.retain(|id, _| ids.contains(id));
            self.tile_cache.retain(|id, _| ids.contains(id));
            self.icon_shadows.retain(|id, _| ids.contains(id));
            self.labels.retain(|id, _| ids.contains(id));
            self.label_shadows.retain(|id, _| ids.contains(id));
            self.hover = None;
            self.popup = None;
            self.save();
            self.arm_motion(hwnd);
        }
    }
    pub(super) unsafe fn open_popup(&mut self, hwnd: HWND, kind: Kind, anchor: [f32; 2]) {
        self.popup = None;
        self.cancel_pointer(hwnd);
        SetFocus(hwnd);
        let (mut title, mut subtitle, mut rows) = (String::new(), String::new(), Vec::new());
        let mut push = |label: &str, action: Action, icon: u8, enabled: bool, danger: bool| {
            rows.push(Row {
                label: label.into(),
                action,
                icon,
                enabled,
                danger,
            })
        };
        match &kind {
            Kind::Add => {
                title = "Add anything".into();
                subtitle = "FILES · SHORTCUTS · PLACES".into();
                push("Browse files & apps...", Action::Files, 0, true, false);
                push("Browse folder...", Action::Folder, 0, true, false);
                push("Recycle Bin", Action::System(false), 5, true, false);
                push("This PC", Action::System(true), 6, true, false);
            }
            Kind::Item(id) => {
                if let Some(item) = self.dock.items.iter().find(|i| i.id == *id) {
                    title = item.name.clone();
                    subtitle = match item.kind.as_str() {
                        "aumid" => "MICROSOFT STORE APP",
                        "folder" => "FOLDER",
                        "shell" => "WINDOWS LOCATION",
                        _ => "PINNED ITEM",
                    }
                    .into();
                    push("Open", Action::Open(id.clone()), 1, true, false);
                    if item.target.eq_ignore_ascii_case("shell:RecycleBinFolder") {
                        push(
                            "Empty Recycle Bin",
                            Action::EmptyRecycleBin(id.clone()),
                            5,
                            true,
                            true,
                        );
                    }
                    if item.catalog {
                        push("Pin to dock", Action::Pin(id.clone()), 0, true, false);
                    }
                    if !item.catalog {
                        push("Rename", Action::Rename(id.clone()), 2, true, false);
                        push("Change icon...", Action::Icon(id.clone()), 3, true, false);
                        if item.icon.as_deref().is_some_and(|name| name.starts_with("native-custom-")) {
                            push("Reset icon", Action::ResetIcon(id.clone()), 3, true, false);
                        }
                        push(
                            "Group with...",
                            Action::Groups(id.clone()),
                            0,
                            self.dock.items.len() > 1,
                            false,
                        );
                        push(
                            "Ungroup",
                            Action::Ungroup(id.clone()),
                            0,
                            item.group.is_some(),
                            false,
                        );
                        push(
                            "Remove from Lumen",
                            Action::Remove(id.clone()),
                            5,
                            true,
                            true,
                        );
                    }
                }
            }
            Kind::Rename(id) => {
                title = "Rename".into();
                subtitle = "NAME IN YOUR DOCK".into();
                push("Save name", Action::Save(id.clone()), 2, true, false);
            }
        }
        let (w, h) = self.dimensions(hwnd);
        let width = 244_f32.min(w as f32 - 32.);
        let slots = rows.len().min(((h - 100) / 32).max(1) as usize);
        let height = if matches!(kind, Kind::Rename(_)) {
            152.
        } else {
            70. + slots as f32 * 32.
        };
        let x = anchor[0].clamp(16., (w as f32 - width - 16.).max(16.));
        let y = (anchor[1] - height - 8.).clamp(16., (h as f32 - height - 16.).max(16.));
        let hover_tweens = vec![crate::motion::Tween::fixed(0.); rows.len()];
        let now = self.now();
        let mut open_alpha = crate::motion::Tween::fixed(0.);
        let mut open_scale = crate::motion::Tween::fixed(0.95);
        open_alpha.target(1., now, 160., [0.2, 0.9, 0.2, 1.]);
        open_scale.target(1., now, 160., [0.2, 0.9, 0.2, 1.]);
        let mut popup = Popup {
            rect: [x, y, x + width, y + height],
            kind: kind.clone(),
            title,
            subtitle,
            rows,
            hover: None,
            pressed: None,
            hover_tweens,
            open_alpha,
            open_scale,
            scroll: 0,
            slots,
            edit: null_mut(),
            brush: null_mut(),
        };
        if let Kind::Rename(id) = &kind {
            let name = self
                .dock
                .items
                .iter()
                .find(|i| i.id == *id)
                .map_or("", |i| i.name.as_str());
            popup.brush = CreateSolidBrush(rgb(28, 27, 39));
            popup.edit = CreateWindowExW(
                0,
                wide("EDIT").as_ptr(),
                wide(name).as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL as u32,
                self.px(x as i32 + 14),
                self.px(y as i32 + 65),
                self.px(width as i32 - 28),
                self.px(28),
                hwnd,
                EDIT_ID as _,
                GetModuleHandleW(null()),
                null(),
            );
            SendMessageW(popup.edit, WM_SETFONT, self.search_font as usize, 1);
            SendMessageW(popup.edit, EM_LIMITTEXT, 128, 0);
            SetWindowSubclass(popup.edit, Some(edit_proc), EDIT_ID, hwnd as usize);
            popup.slots = 1;
        }
        let edit = popup.edit;
        self.popup = Some(popup);
        if !edit.is_null() {
            SetFocus(edit);
            SendMessageW(edit, windows_sys::Win32::UI::Controls::EM_SETSEL, 0, -1);
        }
        self.arm_motion(hwnd);
    }
    pub(super) unsafe fn popup_down(&mut self, hwnd: HWND, p: [f32; 2]) -> bool {
        let now = self.now();
        let Some(popup) = &mut self.popup else {
            return false;
        };
        if !popup.inside(p) {
            self.popup = None;
            self.arm_motion(hwnd);
            return true;
        }
        if !popup.edit.is_null() {
            if p[1] >= popup.rect[1] + 110. {
                popup.pressed = Some(0);
                SetCapture(hwnd);
            }
        } else {
            popup.pressed = popup.hit(p);
            SetCapture(hwnd);
        }
        popup.retarget_hover(now);
        self.arm_motion(hwnd);
        true
    }
    pub(super) unsafe fn popup_up(&mut self, hwnd: HWND, p: [f32; 2]) -> bool {
        let now = self.now();
        let Some(popup) = &mut self.popup else {
            return false;
        };
        let pressed = popup.pressed.take();
        let over = if !popup.edit.is_null() && p[1] >= popup.rect[1] + 110. && popup.inside(p) {
            Some(0)
        } else {
            popup.hit(p)
        };
        popup.retarget_hover(now);
        if GetCapture() == hwnd {
            ReleaseCapture();
        }
        if pressed.is_some() && pressed == over {
            let action = popup.rows[pressed.unwrap()].action.clone();
            self.popup_action(hwnd, action);
        } else {
            self.arm_motion(hwnd);
        }
        true
    }
    pub(super) unsafe fn popup_move(&mut self, hwnd: HWND, p: [f32; 2]) -> bool {
        let now = self.now();
        let Some(popup) = &mut self.popup else {
            return false;
        };
        let hover = popup.hit(p);
        if hover != popup.hover {
            popup.hover = hover;
            popup.retarget_hover(now);
            self.arm_motion(hwnd);
        }
        true
    }
    pub(super) unsafe fn popup_leave(&mut self, hwnd: HWND) {
        let now = self.now();
        if let Some(popup) = &mut self.popup {
            if popup.hover.is_some() {
                popup.hover = None;
                popup.retarget_hover(now);
                self.arm_motion(hwnd);
            }
        }
    }
    pub(super) unsafe fn popup_wheel(&mut self, hwnd: HWND, delta: i32) -> bool {
        let now = self.now();
        let Some(popup) = &mut self.popup else {
            return false;
        };
        if delta < 0 {
            popup.scroll = (popup.scroll + 1).min(popup.rows.len().saturating_sub(popup.slots));
        } else {
            popup.scroll = popup.scroll.saturating_sub(1);
        }
        popup.hover = None;
        popup.retarget_hover(now);
        self.arm_motion(hwnd);
        true
    }
    pub(super) unsafe fn popup_key(&mut self, hwnd: HWND, key: u32) -> bool {
        let now = self.now();
        let Some(popup) = &mut self.popup else {
            return false;
        };
        if key == 27 {
            self.popup = None;
            SetFocus(hwnd);
            self.arm_motion(hwnd);
            return true;
        }
        if key == 13 {
            let index = if popup.edit.is_null() {
                popup.hover.unwrap_or(popup.scroll)
            } else {
                0
            };
            if let Some(row) = popup.rows.get(index).filter(|r| r.enabled) {
                let action = row.action.clone();
                self.popup_action(hwnd, action);
            }
        } else if key == 38 || key == 40 {
            let old = popup.hover.unwrap_or(popup.scroll);
            let next = if key == 40 {
                (old + 1).min(popup.rows.len().saturating_sub(1))
            } else {
                old.saturating_sub(1)
            };
            popup.hover = Some(next);
            popup.retarget_hover(now);
            if next < popup.scroll {
                popup.scroll = next;
            } else if next >= popup.scroll + popup.slots {
                popup.scroll = next + 1 - popup.slots;
            }
            self.arm_motion(hwnd);
        }
        true
    }
    unsafe fn popup_action(&mut self, hwnd: HWND, action: Action) {
        let anchor = self
            .popup
            .as_ref()
            .map_or([30., 180.], |p| [p.rect[0], p.rect[3] + 8.]);
        let rename = self.popup.as_ref().filter(|p| !p.edit.is_null()).map(|p| {
            let len = GetWindowTextLengthW(p.edit).clamp(0, 128) as usize;
            let mut text = vec![0u16; len + 1];
            GetWindowTextW(p.edit, text.as_mut_ptr(), text.len() as i32);
            String::from_utf16_lossy(&text[..len]).trim().to_owned()
        });
        self.popup = None;
        match action {
            Action::Pin(id) => {
                if let Some(item) = self.dock.items.iter_mut().find(|i| i.id == id) {
                    item.catalog = false;
                }
                self.save();
            }
            Action::Files => self.add_files(hwnd),
            Action::Folder => self.add_folder(hwnd),
            Action::System(pc) => {
                self.context_command(hwnd, if pc { CMD_PC } else { CMD_BIN }, None)
            }
            Action::Open(id) => {
                if let Some(i) = self.dock.items.iter().position(|i| i.id == id) {
                    self.launch(hwnd, i);
                }
            }
            Action::EmptyRecycleBin(id) => {
                let res = SHEmptyRecycleBinW(hwnd, null(), 0);
                if res == S_OK {
                    self.icons.remove(&id);
                    self.scaled_icons.remove(&id);
                    self.tile_cache.remove(&id);
                    self.icon_shadows.remove(&id);
                    self.status = "Emptied Recycle Bin".into();
                    InvalidateRect(hwnd, null(), 0);
                }
            }
            Action::Rename(id) => self.open_popup(hwnd, Kind::Rename(id), anchor),
            Action::Groups(id) => self.begin_group_selection(hwnd, &id),
            Action::Icon(id) => self.change_icon(hwnd, &id),
            Action::ResetIcon(id) => {
                if let Some(item) = self.dock.items.iter_mut().find(|i| i.id == id) {
                    item.icon = None;
                }
                self.icons.remove(&id);
                self.scaled_icons.remove(&id);
                self.tile_cache.remove(&id);
                self.icon_shadows.remove(&id);
                self.save();
            }
            Action::Remove(id) => {
                if let Some(i) = self.dock.items.iter().position(|i| i.id == id) {
                    self.context_command(hwnd, CMD_REMOVE, Some(i));
                }
            }
            Action::Ungroup(id) => {
                self.dock.ungroup(&id);
                self.save();
            }
            Action::Save(id) => {
                if let Some(name) = rename.filter(|s| !s.is_empty()) {
                    if let Some(item) = self.dock.items.iter_mut().find(|i| i.id == id) {
                        item.name = name;
                        self.save();
                    }
                }
                SetFocus(hwnd);
            }
        }
        self.arm_motion(hwnd);
    }
    unsafe fn change_icon(&mut self, hwnd: HWND, id: &str) {
        let mut buffer = vec![0u16; 32768];
        let filter = wide("Images and icons\0*.png;*.jpg;*.jpeg;*.bmp;*.ico\0\0");
        let title = wide("Choose an icon image");
        let mut dialog: OPENFILENAMEW = zeroed();
        dialog.lStructSize = size_of::<OPENFILENAMEW>() as u32;
        dialog.hwndOwner = hwnd;
        dialog.lpstrFilter = filter.as_ptr();
        dialog.lpstrFile = buffer.as_mut_ptr();
        dialog.nMaxFile = buffer.len() as u32;
        dialog.lpstrTitle = title.as_ptr();
        dialog.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_NOCHANGEDIR;
        if GetOpenFileNameW(&mut dialog) == 0 {
            return;
        }
        let len = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        let path = String::from_utf16_lossy(&buffer[..len]);
        let result = (|| -> Result<String, String> {
            let reader = image::ImageReader::open(&path).map_err(|e| e.to_string())?;
            let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
            if w > 4096 || h > 4096 {
                return Err("Choose an image up to 4096 pixels per side".into());
            }
            let image = image::open(&path).map_err(|e| e.to_string())?;
            let dir = self.root.join("config/icons");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let file = format!(
                "native-custom-{}-{}.png",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            );
            image.save(dir.join(&file)).map_err(|e| e.to_string())?;
            Ok(file)
        })();
        match result {
            Ok(file) => {
                if let Some(item) = self.dock.items.iter_mut().find(|i| i.id == id) {
                    item.icon = Some(file);
                }
                self.icons.remove(id);
                self.scaled_icons.remove(id);
                self.tile_cache.remove(id);
                self.icon_shadows.remove(id);
                self.save();
            }
            Err(error) => self.status = error,
        }
    }
    pub(super) unsafe fn paint_popup(&mut self, dc: HDC, hwnd: HWND) {
        let Some(popup) = &self.popup else {
            return;
        };
        let now = self.paint_time;
        let open_a = popup.open_alpha.sample(now).clamp(0., 1.);
        if open_a <= 0.001 {
            return;
        }
        let open_s = popup.open_scale.sample(now);
        let raw = popup.rect;
        let cx = (raw[0] + raw[2]) * 0.5;
        let cy = (raw[1] + raw[3]) * 0.5;
        let hw = (raw[2] - raw[0]) * 0.5 * open_s;
        let hh = (raw[3] - raw[1]) * 0.5 * open_s;
        let r = [cx - hw, cy - hh, cx + hw, cy + hh];
        let light = crate::contrast::luminance(self.search_ink) < 0.5;
        let canvas = Canvas::new(dc, self.dpi);
        canvas.soft_shadow(r, 18., 64. * open_a, 10., 4.);
        canvas.round(
            r,
            18.,
            if light {
                rgb(239, 239, 246)
            } else {
                rgb(29, 27, 40)
            },
            (248. * open_a).round() as u8,
            false,
            false,
        );
        canvas.coating(r, 18.);
        canvas.round(r, 18., 0xffffff, (32. * open_a).round() as u8, true, false);
        let text = if light {
            rgb(33, 31, 45)
        } else {
            rgb(232, 230, 240)
        };
        let muted = if light {
            rgb(105, 102, 121)
        } else {
            rgb(133, 128, 150)
        };
        canvas.text(
            &popup.title,
            [r[0] + 16., r[1] + 12., r[2] - 16., r[1] + 34.],
            12.,
            true,
            text,
            0,
        );
        canvas.text(
            &popup.subtitle,
            [r[0] + 16., r[1] + 34., r[2] - 16., r[1] + 49.],
            8.5,
            true,
            muted,
            0,
        );
        let now = self.paint_time;
        let rename = !popup.edit.is_null();
        for index in popup.scroll..(popup.scroll + popup.slots).min(popup.rows.len()) {
            let row = &popup.rows[index];
            let y = if rename {
                r[1] + 111.
            } else {
                r[1] + 60. + (index - popup.scroll) as f32 * 32.
            };
            let hover = popup.row_hover(index, now);
            if hover > 0.001 {
                let alpha = (hover * if popup.pressed == Some(index) { 26. } else { 18. }).round() as u8;
                if alpha > 0 {
                    canvas.round(
                        [r[0] + 8., y, r[2] - 8., y + 30.],
                        9.,
                        text,
                        alpha,
                        false,
                        false,
                    );
                }
            }
            if row.danger {
                canvas.line(
                    &[(r[0] + 14., y - 3.), (r[2] - 14., y - 3.)],
                    muted,
                    40,
                    0.7,
                );
            }
            let color = if !row.enabled {
                muted
            } else if row.danger {
                rgb(248, 148, 150)
            } else {
                text
            };
            let x = r[0] + 18.;
            let iy = y + 8.;
            match row.icon {
                0 => {
                    canvas.line(
                        &[
                            (x, iy + 3.),
                            (x, iy + 13.),
                            (x + 12., iy + 13.),
                            (x + 12., iy + 3.),
                            (x + 6., iy + 3.),
                            (x + 4., iy),
                            (x, iy),
                            (x, iy + 3.),
                        ],
                        color,
                        180,
                        1.1,
                    );
                }
                1 => canvas.line(
                    &[
                        (x + 2., iy),
                        (x + 2., iy + 13.),
                        (x + 12., iy + 6.5),
                        (x + 2., iy),
                    ],
                    color,
                    200,
                    1.1,
                ),
                2 => {
                    canvas.line(
                        &[
                            (x, iy + 12.),
                            (x + 3., iy + 12.),
                            (x + 12., iy + 3.),
                            (x + 9., iy),
                            (x, iy + 9.),
                            (x, iy + 12.),
                        ],
                        color,
                        200,
                        1.1,
                    );
                }
                3 => {
                    canvas.round([x, iy, x + 13., iy + 13.], 2., color, 180, true, false);
                    canvas.line(
                        &[
                            (x + 2., iy + 10.),
                            (x + 5., iy + 6.),
                            (x + 8., iy + 9.),
                            (x + 11., iy + 5.),
                        ],
                        color,
                        180,
                        1.,
                    );
                }
                5 => {
                    canvas.line(
                        &[
                            (x + 1., iy + 3.),
                            (x + 12., iy + 3.),
                            (x + 11., iy + 14.),
                            (x + 3., iy + 14.),
                            (x + 2., iy + 3.),
                            (x, iy + 3.),
                            (x + 13., iy + 3.),
                            (x + 8., iy + 3.),
                            (x + 8., iy),
                            (x + 5., iy),
                            (x + 5., iy + 3.),
                        ],
                        color,
                        190,
                        1.1,
                    );
                }
                _ => {
                    canvas.round([x, iy, x + 14., iy + 10.], 2., color, 180, true, false);
                    canvas.line(
                        &[
                            (x + 7., iy + 10.),
                            (x + 7., iy + 14.),
                            (x + 3., iy + 14.),
                            (x + 11., iy + 14.),
                        ],
                        color,
                        190,
                        1.1,
                    );
                }
            }
            canvas.text_medium(
                &row.label,
                [r[0] + 42., y + 5., r[2] - 12., y + 28.],
                11.,
                color,
                0,
            );
        }
        if popup.rows.len() > popup.slots {
            let ratio = popup.scroll as f32 / (popup.rows.len() - popup.slots) as f32;
            let height = (r[3] - r[1] - 75.) * popup.slots as f32 / popup.rows.len() as f32;
            let y = r[1] + 61. + ratio * (r[3] - r[1] - 75. - height);
            canvas.round(
                [r[2] - 6., y, r[2] - 4., y + height],
                1.,
                muted,
                120,
                false,
                false,
            );
        }
        if rename {
            canvas.round(
                [r[0] + 11., r[1] + 60., r[2] - 11., r[1] + 98.],
                9.,
                0,
                40,
                false,
                false,
            );
            drop(canvas);
            let mut rect: RECT = zeroed();
            GetClientRect(popup.edit, &mut rect);
            let edc = CreateCompatibleDC(dc);
            let bmp = CreateCompatibleBitmap(dc, rect.right, rect.bottom);
            let old = SelectObject(edc, bmp);
            let bg = CreateSolidBrush(rgb(28, 27, 39));
            FillRect(edc, &rect, bg);
            DeleteObject(bg);
            SEARCH_PALETTE.with(|p| p.set(Some((hwnd, popup.edit, popup.brush, text))));
            SendMessageW(
                popup.edit,
                WM_PRINTCLIENT,
                edc as usize,
                (PRF_CLIENT | PRF_ERASEBKGND) as isize,
            );
            SEARCH_PALETTE.with(|p| p.set(None));
            render::flush(dc);
            BitBlt(
                dc,
                self.px(r[0] as i32 + 14),
                self.px(r[1] as i32 + 65),
                rect.right,
                rect.bottom,
                edc,
                0,
                0,
                SRCCOPY,
            );
            Canvas::new(dc, self.dpi).mark_opaque([
                r[0] + 14.,
                r[1] + 65.,
                r[0] + 14. + rect.right as f32 / self.dpi,
                r[1] + 65. + rect.bottom as f32 / self.dpi,
            ]);
            SelectObject(edc, old);
            DeleteObject(bmp);
            DeleteDC(edc);
        }
    }
}
