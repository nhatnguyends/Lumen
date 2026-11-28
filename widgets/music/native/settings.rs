//! Music schema and callbacks for the shared Dock settings window/control kit.
use super::*;
use crate::settings_ui::{Change, Choice, Host as SettingsHost, Row, Value};
pub const NOTIFY: u32 = WM_APP + 83;
pub const FLUSH_TIMER: usize = 92;
fn missing_browser_notice(selected: &str, installed: &[&str]) -> Option<String> {
    if selected == "auto" || installed.contains(&selected) {
        return None;
    }
    let label = backend::browser_label(selected);
    Some(match installed.first() {
        Some(&fallback) => format!(
            "{label} is not installed on this PC — using {} instead.",
            backend::browser_label(fallback)
        ),
        None => format!(
            "{label} is not installed on this PC. Install Brave, Chrome, or Edge to play Music."
        ),
    })
}

impl State {
    pub(super) fn surface_color(&self) -> u32 {
        crate::surface::color(
            &self.config.palette,
            self.glass.borrow().wallpaper_accent_for(self.hwnd),
        )
    }
    pub(super) fn settings_rows(&self) -> Vec<Row> {
        let installed = backend::installed_browsers();
        let mut browser_keys: Vec<String> = vec![
            "auto".into(),
            "brave".into(),
            "chrome".into(),
            "edge".into(),
        ];
        for &browser in &installed {
            if !browser_keys.iter().any(|k| k == browser) {
                browser_keys.push(browser.into());
            }
        }
        if !self.config.browser.is_empty()
            && !browser_keys.iter().any(|k| k == &self.config.browser)
        {
            browser_keys.push(self.config.browser.clone());
        }
        let browser_choices: Vec<Choice> = browser_keys
            .into_iter()
            .map(|key| {
                let label = backend::browser_label(&key);
                Choice::new(key, label, None)
            })
            .collect();

        let mut rows = vec![
            Row::section("Account"),
            Row::accent_button(204, if self.playback.signed_in { "Sign out of YouTube Music" } else { "Sign in to YouTube Music" }, 0x0000ff),
            Row::note(if self.playback.signed_in { "Sign out clears the session only in Music's dedicated browser profile." } else { "Sign in in the browser, close it, then press Play to reconnect." }),
            Row::choices(205, "Player browser", &self.config.browser, browser_choices),
        ];
        if let Some(notice) = missing_browser_notice(&self.config.browser, &installed) {
            rows.push(Row::highlight_note(notice, 0x1ea0f5));
        }
        rows.extend([
            Row::note("Auto prefers Brave (ad-free) when installed, then Chrome or Edge. Switching browsers uses a separate YouTube Music sign-in profile."),
            Row::section("Background"),
            Row::slider(102, "Glass tint", self.config.tint, 0., 100., "%"),
            Row::slider(103, "Frost strength", self.config.frost_strength, 0., 100., "%"),
            Row::slider(105, "Background opacity", self.config.opacity, 0., 100., "%"),
            Row::choices(106, "Surface palette", &self.config.palette, crate::settings_ui::palette_choices()),
            Row::section("Layout"),
            Row::toggle(
                203,
                "Floating mode",
                "Pop out above all windows and detach from fuse.",
                self.config.floating,
            ),
            Row::choices(201, "Music design", &self.config.design, vec![
                Choice::new("coverflow", "Coverflow", None),
                Choice::new("compact", "Compact", None),
                Choice::new("turntable", "Turntable", None),
                Choice::new("orbit", "Orbit", None),
                Choice::new("disc", "Disc", None),
            ]),

            Row::note("Themes scale down in place before switching to the pocket player. Minimum size: 168 × 104 DIP. Playback follows the web player."),
        ]);
        rows
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
            (204, Value::Action) => {
                self.send(serde_json::json!({"action": if self.playback.signed_in { "signout" } else { "signin" }}));
                return false;
            }
            (205, Value::Choice(value)) if model::BROWSERS.contains(&value.as_str()) => {
                if self.config.browser == value {
                    return false;
                }
                self.config.browser = value.clone();
                if self.backend.is_some() {
                    self.send(serde_json::json!({"action": "setBrowser", "browser": value}));
                }
                self.sync_settings();
                self.settings_dirty = true;
                return true;
            }
            (101, Value::Bool(value)) => {
                self.config.frosted_glass = value;
                self.local_glass = None;
            }
            (102, Value::Number(value)) if value.is_finite() => {
                self.config.tint = value.clamp(0., 100.)
            }
            (103, Value::Number(value)) if value.is_finite() => {
                self.config.frost_strength = value.clamp(0., 100.)
            }
            (104, Value::Number(value)) if value.is_finite() => {
                self.config.blur = value.clamp(0., 40.);
                self.local_glass = None;
                self.blur_dirty = false;
                self.background = None;
                self.overlay = None;
                self.settings_dirty = true;
                return true;
            }
            (105, Value::Number(value)) if value.is_finite() => {
                self.config.opacity = value.clamp(0., 100.);
                self.settings_dirty = true;
                return true;
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
                self.background = None;
                self.overlay = None;
                self.clear_spin_base();
                self.frost = None;
                self.card_cache.clear();
                self.window_origin = [i32::MIN, i32::MIN];
                self.fusion_revision = 0;
                self.save_floating();
                unsafe {
                    PostMessageW(self.hwnd, WM_APP + 82, 0, 0);
                }
                return true;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_browser_notice_reports_fallback_or_install_hint() {
        assert_eq!(missing_browser_notice("auto", &["chrome", "edge"]), None);
        assert_eq!(missing_browser_notice("brave", &["brave", "chrome"]), None);
        assert_eq!(
            missing_browser_notice("brave", &["chrome", "edge"]).as_deref(),
            Some("Brave is not installed on this PC — using Chrome instead.")
        );
        assert_eq!(
            missing_browser_notice("brave", &[]).as_deref(),
            Some("Brave is not installed on this PC. Install Brave, Chrome, or Edge to play Music.")
        );
    }
}
