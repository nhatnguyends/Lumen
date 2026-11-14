use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Display,
    Hdr,
    Source,
    Audio,
    Volume,
    Mute,
    Microphone,
    MicVolume,
    MicMute,
}
pub const KINDS: [Kind; 9] = [
    Kind::Volume,
    Kind::Audio,
    Kind::Mute,
    Kind::Display,
    Kind::Hdr,
    Kind::Source,
    Kind::Microphone,
    Kind::MicVolume,
    Kind::MicMute,
];
impl Kind {
    pub fn group(self, target: &str) -> usize {
        match self {
            Self::Audio if !target.is_empty() => 1,
            Self::Volume | Self::Audio | Self::Mute => 0,
            Self::Microphone if !target.is_empty() => 3,
            Self::Microphone | Self::MicVolume | Self::MicMute => 2,
            Self::Display | Self::Source => 4,
            Self::Hdr => 5,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Display => "Display",
            Self::Hdr => "HDR",
            Self::Source => "Monitor input",
            Self::Audio => "Audio output",
            Self::Volume => "Volume",
            Self::Mute => "Mute",
            Self::Microphone => "Microphone",
            Self::MicVolume => "Microphone volume",
            Self::MicMute => "Microphone mute",
        }
    }
    pub fn is_group(self) -> bool {
        matches!(
            self,
            Self::Audio | Self::Microphone | Self::Display | Self::Source
        )
    }
    pub fn is_volume(self) -> bool {
        matches!(self, Self::Volume | Self::MicVolume)
    }
    pub fn is_input(self) -> bool {
        matches!(self, Self::Microphone | Self::MicVolume | Self::MicMute)
    }
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Control {
    pub id: String,
    pub kind: Kind,
    pub target: String,
    pub label: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub version: u32,
    pub open: bool,
    pub position: Option<[i32; 2]>,
    pub size: [i32; 2],
    pub design: String,
    pub palette: String,
    pub frosted: bool,
    pub tint: f32,
    pub opacity: f32,
    pub blur: f32,
    pub frost_strength: f32,
    pub floating: bool,
    pub controls: Vec<Control>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            open: true,
            position: None,
            size: [160, 446],
            design: "compact".into(),
            palette: "wallpaper-auto".into(),
            frosted: true,
            tint: 0.,
            opacity: 100.,
            blur: 10.,
            frost_strength: 100.,
            floating: false,
            controls: [Kind::Volume, Kind::Audio]
                .into_iter()
                .enumerate()
                .map(|(i, kind)| Control {
                    id: format!("control-{i}"),
                    kind,
                    target: String::new(),
                    label: kind.name().into(),
                })
                .collect(),
        }
    }
}
impl Config {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("config/native-controls.json");
        let mut c: Self = if path.is_file() {
            serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?).map_err(
                |e| format!("Cannot read Control Center settings: {e}. File left untouched."),
            )?
        } else {
            Self::import(root)
        };
        if c.version != 1
            || ![c.tint, c.opacity, c.blur, c.frost_strength]
                .iter()
                .all(|v| v.is_finite())
        {
            return Err("Unsupported Control Center settings; file left untouched.".into());
        }
        c.size = [c.size[0].clamp(160, 1800), c.size[1].clamp(112, 1100)];
        c.tint = c.tint.clamp(0., 100.);
        c.opacity = c.opacity.clamp(0., 100.);
        c.blur = c.blur.clamp(0., 80.);
        c.frost_strength = c.frost_strength.clamp(0., 100.);
        if !matches!(
            c.design.as_str(),
            "lumen" | "cloud" | "soft-violet" | "liquid-pastel" | "compact"
        ) {
            c.design = "lumen".into();
        }
        c.palette =
            crate::surface::normalize(&c.palette).unwrap_or_else(|| "wallpaper-auto".into());
        let mut seen = HashSet::new();
        c.controls.retain(|v| {
            v.kind != Kind::Source
                && !v.id.is_empty()
                && v.id.len() <= 64
                && v.id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                && seen.insert(v.id.clone())
                && v.target.len() <= 1024
                && v.label.len() <= 256
        });
        c.normalize_groups();
        c.controls.truncate(40);
        Ok(c)
    }
    fn import(root: &Path) -> Self {
        let mut c = Self::default();
        if let Ok(raw) = std::fs::read(root.join("config/state.json")) {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&raw) {
                let s = &v["settings"];
                let p = &s["controlCenter"];
                if let Some(value) = p.get("controls") {
                    if let Ok(controls) = serde_json::from_value(value.clone()) {
                        c.controls = controls;
                    }
                }
                if let (Some(w), Some(h)) =
                    (p["size"]["width"].as_i64(), p["size"]["height"].as_i64())
                {
                    c.size = [w as i32, h as i32];
                }
                if let (Some(x), Some(y)) =
                    (p["position"]["x"].as_i64(), p["position"]["y"].as_i64())
                {
                    c.position = Some([x as i32, y as i32]);
                }
                if let Some(d) = s["widgetDesign"]["controls"].as_str() {
                    c.design = d.into();
                }
                if let Some(t) = s["widgetTheme"]["controls"]
                    .as_str()
                    .or(s["theme"].as_str())
                {
                    c.palette =
                        crate::surface::normalize(t).unwrap_or_else(|| "wallpaper-auto".into());
                }
                if let Some(o) = s["widgetOpacity"]["controls"].as_f64() {
                    c.opacity = o as f32;
                }
                if let Some(open) = p["open"].as_bool() {
                    c.open = open;
                }
            }
        }
        c
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let dir = root.join("config");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let temp = dir.join("native-controls.json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        unsafe {
            use windows_sys::Win32::Storage::FileSystem::*;
            if MoveFileExW(
                crate::model::wide(&temp).as_ptr(),
                crate::model::wide(dir.join("native-controls.json")).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
    pub fn normalize_groups(&mut self) {
        let mut seen = HashSet::new();
        self.controls.retain_mut(|c| {
            if c.kind == Kind::Mute {
                c.kind = Kind::Volume;
                c.label = "Volume".into();
            }
            if c.kind == Kind::MicMute {
                c.kind = Kind::MicVolume;
                c.label = "Microphone volume".into();
            }
            if c.kind.is_volume() {
                return seen.insert(c.kind);
            }
            if c.kind.is_group() && c.kind != Kind::Source {
                c.target.clear();
                c.label = if c.kind == Kind::Display {
                    "Displays"
                } else {
                    c.kind.name()
                }
                .into();
                seen.insert(c.kind)
            } else {
                true
            }
        });
    }
    pub fn add(&mut self, kind: Kind, target: String, label: String) {
        let kind = match kind {
            Kind::Mute => Kind::Volume,
            Kind::MicMute => Kind::MicVolume,
            _ => kind,
        };
        if kind.is_volume() && self.controls.iter().any(|c| c.kind == kind) {
            return;
        }
        let target = if kind.is_group() && kind != Kind::Source {
            String::new()
        } else {
            target
        };
        if kind.is_group() && kind != Kind::Source && self.controls.iter().any(|c| c.kind == kind) {
            return;
        }

        if self.controls.len() >= 40 {
            return;
        }
        let id = format!(
            "c-{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        self.controls.push(Control {
            id,
            kind,
            target,
            label,
        });
    }
}
#[derive(Clone, Debug)]
pub struct Tile {
    pub index: usize,
    pub rect: [f32; 4],
}
pub const GROUP_NAMES: [&str; 6] = [
    "Audio controls",
    "Audio outputs",
    "Microphone controls",
    "Microphone inputs",
    "Displays",
    "HDR",
];
pub fn manager_layout(
    controls: &[Control],
    w: f32,
    h: f32,
    scroll: f32,
) -> (Vec<Tile>, Vec<(&'static str, [f32; 4])>, f32) {
    let cols = (((w - 36.) + 14.) / 260.).floor().max(1.) as usize;
    let cw = (w - 36. - (cols - 1) as f32 * 14.) / cols as f32;
    let (mut tiles, mut headers) = (vec![], vec![]);
    let mut y = 142.;
    for (group, title) in GROUP_NAMES.iter().enumerate() {
        let items: Vec<_> = controls
            .iter()
            .enumerate()
            .filter(|(_, c)| c.kind.group(&c.target) == group)
            .collect();
        if items.is_empty() {
            continue;
        }
        headers.push((*title, [18., y - scroll, w - 18., y + 24. - scroll]));
        y += 28.;
        for (i, (index, _)) in items.iter().enumerate() {
            let x = 18. + (i % cols) as f32 * (cw + 14.);
            let top = y + (i / cols) as f32 * 152. - scroll;
            tiles.push(Tile {
                index: *index,
                rect: [x, top, x + cw, top + 140.],
            });
        }
        y += ((items.len() + cols - 1) / cols) as f32 * 152. + 12.;
    }
    (tiles, headers, (y + 62. - h).max(0.))
}
#[cfg(test)]
pub fn layout(controls: &[Control], w: f32, h: f32, scroll: f32) -> (Vec<Tile>, f32, bool) {
    layout_devices(controls, w, h, scroll, [2, 2, 2])
}
pub fn layout_devices(
    controls: &[Control],
    w: f32,
    h: f32,
    scroll: f32,
    counts: [usize; 3],
) -> (Vec<Tile>, f32, bool) {
    layout_devices_for_design(controls, w, h, scroll, counts, "lumen")
}
pub fn layout_devices_for_design(
    controls: &[Control],
    w: f32,
    h: f32,
    scroll: f32,
    counts: [usize; 3],
    design: &str,
) -> (Vec<Tile>, f32, bool) {
    let (top, pad, gap) = (60., 18., 10.);
    let compact = h <= 240.;
    let android_compact = design == "compact";
    let mut tiles = vec![];
    let count = |c: &Control| {
        if c.kind == Kind::Source {
            return 3;
        }
        counts[match c.kind {
            Kind::Audio => 0,
            Kind::Microphone => 1,
            _ => 2,
        }]
        .max(1)
    };
    if compact {
        let mut x = pad;
        let height = if android_compact {
            (h - top - pad).clamp(32., 56.)
        } else {
            (h - top - pad).clamp(32., 72.)
        };
        for (index, c) in controls.iter().enumerate() {
            let width = if android_compact {
                if c.kind.is_volume() {
                    160.
                } else {
                    height
                }
            } else if c.kind.is_group() {
                24. + 176. * count(c) as f32
            } else if c.kind.is_volume() {
                184.
            } else {
                132.
            };
            tiles.push(Tile {
                index,
                rect: [x - scroll, top, x + width - scroll, top + height],
            });
            x += width + gap;
        }
        return (tiles, (x - gap + pad - w).max(0.), true);
    }
    let cols = if android_compact {
        ((w - 2. * pad + gap) / 88.).floor().max(1.) as usize
    } else {
        ((w - 2. * pad + gap) / 108.).floor().max(1.) as usize
    };
    let mut row: Vec<(usize, usize, f32)> = vec![];
    let mut y = top;
    let flush_row = |row: &mut Vec<(usize, usize, f32)>, y: &mut f32, tiles: &mut Vec<Tile>| {
        if row.is_empty() {
            return;
        }
        let k = row.len();
        let total_span: usize = row.iter().map(|(_, s, _)| *s).sum();
        let avail = (w - 2. * pad - gap * (k - 1) as f32).max(1.);
        let row_height = row.iter().map(|(_, _, rh)| *rh).fold(0f32, f32::max);
        let mut x = pad;
        for (idx, span, _) in row.drain(..) {
            let width = avail * (span as f32 / total_span as f32);
            tiles.push(Tile {
                index: idx,
                rect: [x, *y - scroll, x + width, *y + row_height - scroll],
            });
            x += width + gap;
        }
        *y += row_height + gap;
    };
    let mut col = 0;
    for (index, c) in controls.iter().enumerate() {
        let span = if android_compact {
            if c.kind.is_volume() { cols } else { 1 }
        } else if c.kind.is_group() || c.kind.is_volume() {
            cols
        } else {
            1
        };
        if col + span > cols {
            flush_row(&mut row, &mut y, &mut tiles);
            col = 0;
        }
        let height = if android_compact {
            if c.kind.is_volume() { 64. } else { 86. }
        } else if c.kind.is_group() {
            36. + 34. * count(c) as f32
        } else {
            72.
        };
        row.push((index, span, height));
        col += span;
        if col == cols {
            flush_row(&mut row, &mut y, &mut tiles);
            col = 0;
        }
    }
    flush_row(&mut row, &mut y, &mut tiles);
    let end = if tiles.is_empty() { top } else { y - gap };
    (tiles, (end + pad - h).max(0.), false)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_device_buttons_merge_into_one_group_at_the_first_position() {
        let mut c = Config::default();
        c.controls.push(Control {
            id: "pinned".into(),
            kind: Kind::Audio,
            target: "speaker".into(),
            label: "Speaker".into(),
        });
        c.normalize_groups();
        assert_eq!(
            c.controls.iter().filter(|c| c.kind == Kind::Audio).count(),
            1
        );
        assert_eq!(c.controls[1].id, "control-1");
        assert!(c.controls[1].target.is_empty());
        let (tiles, _, _) = layout_devices(&c.controls, 520., 420., 0., [4, 2, 2]);
        assert_eq!(tiles[1].rect[3] - tiles[1].rect[1], 172.);
    }
    #[test]
    fn layout_keeps_all_controls_inside_padded_scroll_content() {
        let mut c = Config::default();
        for k in KINDS {
            c.add(k, String::new(), k.name().into());
        }
        for w in [160., 180., 220., 320., 420., 760., 1200.] {
            for h in [112., 120., 180., 240., 400., 700.] {
                let (t, _, row) = layout(&c.controls, w, h, 0.);
                for a in &t {
                    assert!(a.rect[0] >= 18.);
                    assert!(a.rect[1] >= 54.);
                    assert!(a.rect[3] <= h - 18. + 0.01 || !row);
                    if !row {
                        assert!(a.rect[2] <= w - 18. + 0.01);
                    }
                }
                for (i, a) in t.iter().enumerate() {
                    for b in &t[i + 1..] {
                        assert!(
                            a.rect[2] <= b.rect[0]
                                || b.rect[2] <= a.rect[0]
                                || a.rect[3] <= b.rect[1]
                                || b.rect[3] <= a.rect[1]
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn imports_legacy_controls_and_round_trips_without_touching_old_data() {
        let root =
            std::env::temp_dir().join(format!("lumen-controls-model-{}", std::process::id()));
        std::fs::create_dir_all(root.join("config")).unwrap();
        let old = r#"{"settings":{"controlCenter":{"open":true,"size":{"width":760,"height":120},"controls":[{"id":"mic","kind":"mic-volume","target":"","label":"Microphone"}]},"widgetDesign":{"controls":"soft-violet"}}}"#;
        std::fs::write(root.join("config/state.json"), old).unwrap();
        let mut c = Config::load(&root).unwrap();
        assert_eq!(c.controls[0].kind, Kind::MicVolume);
        assert_eq!(c.size, [760, 120]);
        assert_eq!(c.design, "soft-violet");
        c.palette = "gradient:dawn".into();
        c.save(&root).unwrap();
        assert_eq!(Config::load(&root).unwrap().palette, c.palette);
        assert_eq!(
            std::fs::read_to_string(root.join("config/state.json")).unwrap(),
            old
        );
        std::fs::write(root.join("config/native-controls.json"), "{bad").unwrap();
        assert!(Config::load(&root).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
