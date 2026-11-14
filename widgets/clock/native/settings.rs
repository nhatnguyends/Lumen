//! Clock schema/adaptor for the single shared App Dock settings implementation.
use super::*;
use crate::settings_ui::{Change, Choice, Row, Value};
pub(super) const SETTINGS_CHANGED: u32 = WM_APP + 185;
const FROST: u32 = 1;
const TINT: u32 = 2;
const STRENGTH: u32 = 3;
const BLUR: u32 = 4;
const OPACITY: u32 = 5;
const PALETTE: u32 = 6;
const THEME: u32 = 10;
const ZONE: u32 = 11;
const HOUR12: u32 = 12;
const SECONDS: u32 = 13;
const FLOAT: u32 = 14;
const MOTION: u32 = 20;
const ANIMATION_MODE: u32 = 21;
impl State {
    pub(super) fn surface_color(&self) -> u32 {
        if self.config.palette == "theme" {
            face::color(&self.config.theme)
        } else {
            crate::surface::color(&self.config.palette, self.glass.borrow().wallpaper_accent())
        }
    }
    fn palette_choices(&self) -> Vec<Choice> {
        let mut choices = vec![Choice::new(
            "theme",
            "Theme material",
            Some(face::color(&self.config.theme)),
        )];
        choices.extend(crate::settings_ui::palette_choices());
        for item in &mut choices {
            if item.key == crate::accent::THEME {
                item.color = Some(self.glass.borrow().wallpaper_accent());
            }
        }
        choices
    }
    pub(super) fn settings_rows(&self) -> Vec<Row> {
        let c = &self.config;
        let mut rows = vec![
            Row::section("Background"),
            Row::toggle(
                FROST,
                "Frosted glass",
                "Use the wallpaper behind this clock",
                c.frosted_glass,
            ),
            Row::slider(TINT, "Glass tint", c.tint, 0., 100., "%"),
            Row::slider(STRENGTH, "Frost strength", c.frost_strength, 0., 100., "%"),
            Row::slider(BLUR, "Blur size", c.blur, 0., 80., " px"),
            Row::slider(OPACITY, "Background opacity", c.opacity, 0., 100., "%"),
            Row::choices(
                PALETTE,
                "Surface palette",
                &c.palette,
                self.palette_choices(),
            ),
            Row::section("Clock face"),
            Row::choices(
                THEME,
                "Theme",
                &c.theme,
                THEMES
                    .iter()
                    .map(|(key, name)| Choice::new(*key, *name, Some(face::color(key))))
                    .collect(),
            ),
            Row::toggle(
                HOUR12,
                "12-hour time",
                "Show AM or PM alongside the time",
                c.hour12,
            ),
            Row::toggle(
                SECONDS,
                "Show seconds",
                "Update seconds and the companion dial",
                c.show_seconds,
            ),
            Row::section("Window"),
        ];
        if c.theme == "sunset" {
            let at = rows.len() - 2;
            rows.splice(
                at..at,
                [
                    Row::section("Afterglow motion"),
                    Row::toggle(
                        MOTION,
                        "Dynamic animation",
                        "Gentle sun rays, stars and a sun-to-moon transition",
                        c.dynamic_animation,
                    ),
                    Row::choices(
                        ANIMATION_MODE,
                        "Sun and moon",
                        &c.animation_mode,
                        vec![
                            Choice::new("clock", "Follow clock time", None),
                            Choice::new("loop", "Decorative loop", None),
                        ],
                    ),
                ],
            );
        }
        rows
    }
    pub(super) unsafe fn open_settings(&mut self) {
        if self.settings.is_none() {
            match crate::settings_ui::Host::create(
                self.hwnd,
                "Clock settings",
                "Changes here apply only to Clock.",
                self.glass.clone(),
                self.settings_rows(),
                SETTINGS_CHANGED,
            ) {
                Ok(host) => self.settings = Some(host),
                Err(e) => {
                    self.error = e;
                    eprintln!("native-clock-settings: {}", self.error);
                    return;
                }
            }
        }
        let color = self.surface_color();
        let light = crate::accent::light(color);
        let rows = self.settings_rows();
        if let Some(host) = &mut self.settings {
            host.set_rows(rows);
            host.set_surface(color, light);
            host.show();
        }
    }
    pub(super) fn sync_settings(&mut self) {
        let rows = self.settings_rows();
        let color = self.surface_color();
        if let Some(host) = &mut self.settings {
            host.set_rows(rows);
            host.set_surface(color, crate::accent::light(color));
        }
    }
    pub(super) unsafe fn settings_changed(&mut self) {
        let changes = self
            .settings
            .as_mut()
            .map(|h| h.take_changes())
            .unwrap_or_default();
        if changes.is_empty() {
            return;
        }
        let mut changed = false;
        let mut mode = false;
        let mut backdrop_changed = false;
        let mut structural = false;
        for change in changes {
            let window_mode = change.id == FLOAT;
            let is_opacity = change.id == OPACITY;
            let is_slider = matches!(change.id, TINT | STRENGTH | BLUR | OPACITY);
            if self.apply_setting(change) {
                changed = true;
                mode |= window_mode;
                backdrop_changed |= !is_opacity;
                structural |= !is_slider;
            }
        }
        if changed {
            if backdrop_changed {
                self.key.clear();
            }
            if !self.config.frosted_glass {
                self.frost = None;
                self.frost_key.clear();
            }
            self.arm();
            SetTimer(self.hwnd, 99, 180, None);
            if mode {
                crate::window::apply_window_mode(self.hwnd, self.config.floating, self.inspect);
            }
            if structural {
                self.sync_settings();
            }
            InvalidateRect(self.hwnd, null(), 0);
        }
    }
    pub(super) fn apply_setting(&mut self, change: Change) -> bool {
        match (change.id, change.value) {
            (MOTION, Value::Bool(v)) => self.config.dynamic_animation = v,
            (ANIMATION_MODE, Value::Choice(v)) if ["clock", "loop"].contains(&v.as_str()) => {
                self.config.animation_mode = v
            }
            (FROST, Value::Bool(v)) => self.config.frosted_glass = v,
            (HOUR12, Value::Bool(v)) => self.config.hour12 = v,
            (SECONDS, Value::Bool(v)) => self.config.show_seconds = v,
            (FLOAT, Value::Bool(v)) => self.config.floating = v,
            (id, Value::Number(v)) if v.is_finite() => {
                let slot = match id {
                    TINT => &mut self.config.tint,
                    STRENGTH => &mut self.config.frost_strength,
                    BLUR => &mut self.config.blur,
                    OPACITY => &mut self.config.opacity,
                    _ => return false,
                };
                *slot = v.clamp(0., if id == BLUR { 80. } else { 100. });
            }
            (THEME, Value::Choice(v)) if THEMES.iter().any(|t| t.0 == v) => self.config.theme = v,
            (ZONE, Value::Choice(v)) => match zone(&v) {
                Ok(z) => {
                    self.location_label = model::location_label(&v);
                    self.config.time_zone = v;
                    self.zone = z
                }
                Err(e) => {
                    self.error = e;
                    return false;
                }
            },
            (PALETTE, Value::Choice(v)) if v == "theme" || crate::surface::valid(&v) => {
                self.config.palette = if v == "theme" {
                    v
                } else {
                    crate::surface::normalize(&v).unwrap_or(v)
                }
            }
            _ => return false,
        }
        true
    }
}
