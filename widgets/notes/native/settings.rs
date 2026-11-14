//! Notes schema only. Shell, toggles, sliders and theme cards use the shared kit.
use super::*;
use crate::settings_ui::{Change, Choice, Row, Value};
pub const MESSAGE: u32 = WM_APP + 104;
pub fn rows(b: &model::Board) -> Vec<Row> {
    vec![
        Row::section("Background"),
        Row::toggle(
            1,
            "Frosted glass",
            "Blend this widget with the wallpaper",
            b.frosted,
        ),
        Row::slider(2, "Glass tint", b.tint, 0., 100., "%"),
        Row::slider(3, "Frost strength", b.frost_strength, 0., 100., "%"),
        Row::slider(4, "Blur size", b.blur.unwrap_or(13.), 0., 64., "px"),
        Row::slider(5, "Background opacity", b.opacity, 0., 100., "%"),
        Row::choices(
            6,
            "Surface palette",
            &b.palette,
            crate::settings_ui::palette_choices(),
        ),
        Row::section("Layout"),
        Row::choices(
            7,
            "Design",
            &b.design,
            vec![
                Choice::new("lumen", "Pinboard", Some(0x554336)),
                Choice::new("notebook", "Notebook", Some(0xcfe2eb)),
                Choice::new("index", "Index", Some(0x2a290f)),
                Choice::new("honey", "Honey", Some(0x52cafa)),
            ],
        ),
    ]
}
pub fn apply(b: &mut model::Board, change: Change) -> bool {
    match (change.id, change.value) {
        (1, Value::Bool(v)) if v != b.frosted => {
            b.frosted = v;
            true
        }
        (2, Value::Number(v)) if v.is_finite() => {
            let v = v.clamp(0., 100.);
            let changed = b.tint != v;
            b.tint = v;
            changed
        }
        (3, Value::Number(v)) if v.is_finite() => {
            let v = v.clamp(0., 100.);
            let changed = b.frost_strength != v;
            b.frost_strength = v;
            changed
        }
        (4, Value::Number(v)) if v.is_finite() => {
            let v = Some(v.clamp(0., 64.));
            let changed = b.blur != v;
            b.blur = v;
            changed
        }
        (5, Value::Number(v)) if v.is_finite() => {
            let v = v.clamp(0., 100.);
            let changed = b.opacity != v;
            b.opacity = v;
            changed
        }
        (6, Value::Choice(v)) if crate::surface::valid(&v) && v != b.palette => {
            b.palette = v;
            true
        }
        (7, Value::Choice(v)) if model::DESIGNS.contains(&v.as_str()) && v != b.design => {
            b.design = v;
            true
        }
        (8, Value::Bool(v)) if v != b.floating => {
            b.floating = v;
            true
        }
        _ => false,
    }
}
impl State {
    pub(super) fn surface_color(&self) -> u32 {
        if self.board.palette != crate::accent::THEME {
            return crate::surface::color(
                &self.board.palette,
                self.glass.borrow().wallpaper_accent(),
            );
        }
        match self.board.design.as_str() {
            "notebook" => rgb(235, 226, 207),
            "index" => rgb(15, 41, 42),
            "honey" => rgb(255, 195, 59),
            _ => self.glass.borrow().wallpaper_accent(),
        }
    }
    pub(super) fn source_glass(&self) -> Rc<RefCell<Glass>> {
        self.local_glass.as_ref().unwrap_or(&self.glass).clone()
    }
    pub(super) unsafe fn update_local_blur(&mut self) {
        if let Some(blur) = self.board.blur {
            if let Some(glass) = &self.local_glass {
                glass.borrow_mut().set_blur(blur, self.dpi);
            } else {
                let mut glass = Glass::new(blur, self.dpi);
                glass.refresh();
                self.local_glass = Some(Rc::new(RefCell::new(glass)));
            }
        }
    }
    pub(super) unsafe fn open_settings(&mut self) {
        // Do not hide, move, re-theme or replace the board when settings opens.
        let color = self.surface_color();
        let light = crate::accent::light(color);
        if self.settings.is_none() {
            match crate::settings_ui::Host::create(
                self.hwnd,
                "Notes settings",
                "Changes here apply only to Sticky Notes.",
                self.source_glass(),
                rows(&self.board),
                MESSAGE,
            ) {
                Ok(host) => self.settings = Some(host),
                Err(e) => {
                    self.status = e;
                    return;
                }
            }
        }
        let rows = rows(&self.board);
        if let Some(host) = &mut self.settings {
            host.set_rows(rows);
            host.set_surface(color, light);
            host.show();
        }
    }
    pub(super) unsafe fn settings_changed(&mut self) {
        let changes = self
            .settings
            .as_mut()
            .map(|h| h.take_changes())
            .unwrap_or_default();
        let mut changed = false;
        let mut blur_changed = false;
        let mut backdrop_changed = false;
        let mut structural = false;
        for change in changes {
            let id = change.id;
            if apply(&mut self.board, change) {
                changed = true;
                blur_changed |= id == 4;
                backdrop_changed |= id != 5;
                structural |= !matches!(id, 2 | 3 | 4 | 5);
            }
        }
        if !changed {
            return;
        }
        if blur_changed {
            self.update_local_blur();
        }
        if structural {
            self.apply_floating();
            self.place_editor();
        }
        if backdrop_changed {
            self.background_key.clear();
            self.overlay = None;
        }
        self.schedule();
        let color = self.surface_color();
        let light = crate::accent::light(color);
        let rows = structural.then(|| rows(&self.board));
        if let Some(host) = &mut self.settings {
            if let Some(rows) = rows {
                host.set_rows(rows);
            }
            host.set_surface(color, light);
        }
        InvalidateRect(self.hwnd, null(), 0);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_change_only_notes_appearance_and_preserve_geometry_content() {
        let mut b = model::Board::default();
        b.create().unwrap();
        b.size = [240, 240];
        b.position = Some([-400, 80]);
        let original_body = b.notes[0].body.clone();
        for (id, value) in [
            (1, Value::Bool(false)),
            (2, Value::Number(42.)),
            (3, Value::Number(55.)),
            (4, Value::Number(22.)),
            (5, Value::Number(35.)),
            (6, Value::Choice("gradient:afterglow".into())),
            (7, Value::Choice("notebook".into())),
            (8, Value::Bool(true)),
        ] {
            assert!(apply(&mut b, Change { id, value }));
        }
        assert_eq!(b.size, [240, 240]);
        assert_eq!(b.position, Some([-400, 80]));
        assert_eq!(b.notes[0].body, original_body);
        assert_eq!(b.blur, Some(22.));
        assert_eq!(b.frost_strength, 55.);
        assert_eq!(b.palette, "gradient:afterglow");
        assert!(b.floating);
        assert!(!apply(
            &mut b,
            Change {
                id: 7,
                value: Value::Choice("bad".into())
            }
        ));
        assert!(!apply(
            &mut b,
            Change {
                id: 2,
                value: Value::Number(f32::NAN)
            }
        ));
        for key in ["custom", "custom:#GG7788", "missing-gradient"] {
            assert!(!apply(
                &mut b,
                Change {
                    id: 6,
                    value: Value::Choice(key.into())
                }
            ));
        }
        assert!(apply(
            &mut b,
            Change {
                id: 7,
                value: Value::Choice("honey".into())
            }
        ));
        assert!(apply(
            &mut b,
            Change {
                id: 6,
                value: Value::Choice("custom:#7A4D28".into())
            }
        ));
        assert_eq!(b.notes[0].body, original_body);
        assert_eq!(b.design, "honey");
        b.validate().unwrap();
        let mut restored: model::Board =
            serde_json::from_slice(&serde_json::to_vec(&b).unwrap()).unwrap();
        restored.validate().unwrap();
        assert_eq!(restored.size, [240, 240]);
        assert_eq!(restored.palette, "custom:#7A4D28");
        assert_eq!(restored.blur, Some(22.));
        let schema = rows(&b);
        assert!(schema
            .iter()
            .any(|r| matches!(r, Row::Slider { id: 5, .. })));
        assert!(!schema
            .iter()
            .any(|r| matches!(r, Row::Toggle { id: 8, .. })));
    }
}
