//! Music schema and callbacks for the shared Dock settings window/control kit.
use super::*;
use crate::settings_ui::{Change, Choice, Host as SettingsHost, Row, Value};
pub const NOTIFY: u32 = WM_APP + 83;
pub const FLUSH_TIMER: usize = 92;
impl State {
    pub(super) fn surface_color(&self) -> u32 {
        crate::surface::color(&self.config.palette, self.glass.borrow().wallpaper_accent())
    }
    pub(super) fn settings_rows(&self) -> Vec<Row> {
        vec![
            Row::section("Background"),
            Row::toggle(101, "Frosted glass", "Blend Music with the wallpaper", self.config.frosted_glass),
            Row::slider(102, "Glass tint", self.config.tint, 0., 100., "%"),
            Row::slider(103, "Frost strength", self.config.frost_strength, 0., 100., "%"),
            Row::slider(104, "Blur size", self.config.blur, 0., 40., "px"),
            Row::slider(105, "Background opacity", self.config.opacity, 0., 100., "%"),
            Row::choices(106, "Surface palette", &self.config.palette, crate::settings_ui::palette_choices()),
            Row::section("Layout"),
            Row::choices(201, "Music design", &self.config.design, vec![
                Choice::new("coverflow", "Coverflow", Some(0x6c4952)),
                Choice::new("lumen", "Compact", None),
                Choice::new("sleeve", "Sleeve", Some(0x4b3349)),
                Choice::new("turntable", "Turntable", Some(0x8eabbd)),
                Choice::new("orbit", "Orbit", Some(0xd7d5d2)),
                Choice::new("disc", "Disc", Some(0xf0eeed)),
                Choice::new("sky", "Sky", Some(0xc28c18)),
            ]),
            Row::toggle(202, "Show queue", "Scroll the loaded player queue inside Music", self.config.queue),

            Row::note("Themes scale down in place before switching to the pocket player. Minimum size: 168 × 104 DIP. Queue collapses first when space is tight."),
        ]
    }
    pub(super) unsafe fn ensure_settings(&mut self) -> bool {
        let rows = self.settings_rows();
        if self.settings.is_none() {
            match SettingsHost::create(
                self.hwnd,
                "Music settings",
                "Changes here apply only to Music.",
                self.glass.clone(),
                rows,
                NOTIFY,
            ) {
                Ok(host) => self.settings = Some(host),
                Err(error) => {
                    self.error = error;
                    InvalidateRect(self.hwnd, null(), 0);
                    return false;
                }
            }
        } else {
            self.settings.as_mut().unwrap().set_rows(rows);
        }
        let color = self.surface_color();
        if let Some(host) = &mut self.settings {
            host.set_surface(color, crate::accent::light(color));
        }
        true
    }
    pub(super) unsafe fn open_settings(&mut self) {
        if self.ensure_settings() {
            self.settings.as_mut().unwrap().show();
        }
    }
    pub(super) fn apply_setting(&mut self, change: Change) -> bool {
        match (change.id, change.value) {
            (101, Value::Bool(value)) => {
                self.config.frosted_glass = value;
                if !value {
                    self.local_glass = None;
                }
            }
            (102, Value::Number(value)) if value.is_finite() => {
                self.config.tint = value.clamp(0., 100.)
            }
            (103, Value::Number(value)) if value.is_finite() => {
                self.config.frost_strength = value.clamp(0., 100.)
            }
            (104, Value::Number(value)) if value.is_finite() => {
                self.config.blur = value.clamp(0., 40.);
                self.blur_dirty = true;
            }
            (105, Value::Number(value)) if value.is_finite() => {
                self.config.opacity = value.clamp(0., 100.)
            }
            (106, Value::Choice(value)) if crate::surface::valid(&value) => {
                self.config.palette = crate::surface::normalize(&value).unwrap_or(value)
            }
            (201, Value::Choice(value)) if model::DESIGNS.contains(&value.as_str()) => {
                self.config.design = value
            }
            (202, Value::Bool(value)) => {
                self.config.queue = value;
                self.queue_offset = 0;
            }
            (203, Value::Bool(value)) => {
                self.config.floating = value;
                unsafe {
                    PostMessageW(self.hwnd, WM_APP + 82, 0, 0);
                }
            }
            _ => return false,
        }
        self.background = None;
        self.overlay = None;
        self.settings_dirty = true;
        true
    }
    pub(super) unsafe fn settings_changed(&mut self) {
        let changes = self
            .settings
            .as_mut()
            .map(|host| host.take_changes())
            .unwrap_or_default();
        let changed = changes.into_iter().fold(false, |changed, change| {
            self.apply_setting(change) || changed
        });
        if changed {
            // Coalesce disk writes and wallpaper decoding while dragging sliders.
            SetTimer(self.hwnd, FLUSH_TIMER, 180, None);
            let color = self.surface_color();
            if let Some(host) = &mut self.settings {
                host.set_surface(color, crate::accent::light(color));
            }
            if IsWindowVisible(self.hwnd) != 0 {
                InvalidateRect(self.hwnd, null(), 0);
            }
        }
    }
    pub(super) unsafe fn flush_settings(&mut self) {
        KillTimer(self.hwnd, FLUSH_TIMER);
        if self.blur_dirty {
            self.local_glass = None;
            self.blur_dirty = false;
            self.background = None;
        }
        if self.settings_dirty {
            self.save();
            self.settings_dirty = false;
        }
        if IsWindowVisible(self.hwnd) != 0 {
            InvalidateRect(self.hwnd, null(), 0);
        }
    }
    pub(super) fn sync_settings(&mut self) {
        if self.settings.is_some() {
            let rows = self.settings_rows();
            self.settings.as_mut().unwrap().set_rows(rows);
        }
    }
}
