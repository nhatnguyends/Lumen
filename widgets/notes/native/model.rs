//! Native notes own their file. Electron HTML is imported, never written back.
use crate::model::wide;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

// Keep the complete Electron palette, including legacy/current Slate notes.
pub const COLORS: [&str; 7] = ["amber", "rose", "sky", "mint", "violet", "peach", "slate"];
pub const MIN_SIZE: i32 = 160;
pub const DESIGNS: [&str; 4] = ["lumen", "notebook", "index", "honey"];
pub fn images_dir(root: &Path) -> std::path::PathBuf {
    root.join("config/notes-images")
}
pub fn extract_rtf_pictures(note: &mut Note, root: &Path) -> bool {
    let Some(rtf) = note.rtf.as_deref() else {
        return false;
    };
    if !rtf.contains("{\\pict") {
        return false;
    }
    let bytes = rtf.as_bytes();
    let mut out = String::with_capacity(rtf.len());
    let mut pos = 0;
    let mut changed = false;
    while let Some(rel) = rtf[pos..].find("{\\pict") {
        let start = pos + rel;
        let next = bytes.get(start + 6).copied();
        if next.is_some_and(|b| b.is_ascii_alphabetic()) {
            out.push_str(&rtf[pos..start + 6]);
            pos = start + 6;
            continue;
        }
        out.push_str(&rtf[pos..start]);
        let mut i = start + 1;
        let mut depth = 1usize;
        while i < bytes.len() && depth > 0 {
            match bytes[i] {
                b'\\' if i + 1 < bytes.len() => i += 2,
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    i += 1;
                }
                _ => i += 1,
            }
        }
        let group = &rtf[start..i];
        let mut saved = false;
        if group.contains("\\pngblip") || group.contains("\\jpegblip") {
            let mut hex = Vec::new();
            let gb = group.as_bytes();
            let mut j = 1;
            let mut gdepth = 1usize;
            while j < gb.len() && gdepth > 0 {
                if gdepth > 1 {
                    match gb[j] {
                        b'\\' if j + 1 < gb.len() => j += 2,
                        b'{' => {
                            gdepth += 1;
                            j += 1;
                        }
                        b'}' => {
                            gdepth -= 1;
                            j += 1;
                        }
                        _ => j += 1,
                    }
                    continue;
                }
                match gb[j] {
                    b'{' => {
                        gdepth += 1;
                        j += 1;
                    }
                    b'}' => {
                        gdepth -= 1;
                        j += 1;
                    }
                    b'\\' => {
                        j += 1;
                        if j < gb.len() && gb[j].is_ascii_alphabetic() {
                            while j < gb.len() && gb[j].is_ascii_alphabetic() {
                                j += 1;
                            }
                            if j < gb.len() && gb[j] == b'-' {
                                j += 1;
                            }
                            while j < gb.len() && gb[j].is_ascii_digit() {
                                j += 1;
                            }
                            if j < gb.len() && gb[j] == b' ' {
                                j += 1;
                            }
                        } else if j < gb.len() {
                            j += 1;
                        }
                    }
                    b if b.is_ascii_hexdigit() => {
                        hex.push(b);
                        j += 1;
                    }
                    _ => j += 1,
                }
            }
            if hex.len() >= 16 {
                let decoded: Vec<u8> = hex
                    .chunks_exact(2)
                    .filter_map(|pair| {
                        let s = std::str::from_utf8(pair).ok()?;
                        u8::from_str_radix(s, 16).ok()
                    })
                    .collect();
                if let Ok(img) = image::load_from_memory(&decoded) {
                    let dir = images_dir(root);
                    if fs::create_dir_all(&dir).is_ok() {
                        let name = format!("img-{}-{}.png", now(), note.images.len());
                        if img.into_rgba8().save(dir.join(&name)).is_ok() {
                            note.images.push(name);
                            saved = true;
                            changed = true;
                        }
                    }
                }
            }
        }
        if !saved {
            out.push_str(group);
        }
        pos = i;
    }
    if changed {
        out.push_str(&rtf[pos..]);
        note.rtf = Some(out);
    }
    changed
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub rtf: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    pub color: String,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: u64,
    pub updated_at: u64,
}
impl Note {
    pub fn plain(&self) -> String {
        if self.rtf.is_some() {
            self.body.clone()
        } else {
            html(&self.body, false)
        }
    }
    pub fn rich_text(&self) -> String {
        self.rtf.clone().unwrap_or_else(|| html(&self.body, true))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Board {
    pub version: u32,
    pub notes: Vec<Note>,
    pub position: Option<[i32; 2]>,
    pub size: [i32; 2],
    pub open: bool,
    pub floating: bool,
    pub design: String,
    pub opacity: f32,
    pub tint: f32,
    pub frosted: bool,
    pub frost_strength: f32,
    /// None reuses the shared source; a custom blur owns its own Glass cache.
    pub blur: Option<f32>,
    pub palette: String,
}
impl Default for Board {
    fn default() -> Self {
        Self {
            version: 1,
            notes: vec![],
            position: None,
            size: [371, 656],
            open: true,
            floating: false,
            design: "lumen".into(),
            opacity: 100.,
            tint: 8.,
            frosted: true,
            frost_strength: 100.,
            blur: Some(8.),
            palette: "custom:#5B848B".into(),
        }
    }
}
impl Board {
    pub fn load(root: &Path) -> Result<Self, String> {
        let own = root.join("config/native-notes.json");
        let mut board: Self = if own.exists() {
            read(&own)?
        } else {
            let mut b = Self::default();
            let mut sources = vec![
                root.join("config/notes.json"),
                root.join("sticky-notes.json"),
                root.join("notes.json"),
            ];
            if let Some(appdata) = std::env::var_os("APPDATA") {
                sources.push(Path::new(&appdata).join("Lumen/notes.json"));
                sources.push(Path::new(&appdata).join("Lumen/sticky-notes.json"));
            }
            if let Some(source) = sources.iter().find(|p| p.exists()) {
                let v: serde_json::Value = read(source)?;
                b.notes = serde_json::from_value(
                    v.get("notes")
                        .cloned()
                        .ok_or("Notes source has no notes array; file left untouched")?,
                )
                .map_err(|e| e.to_string())?;
                if let Some(w) = v.get("window") {
                    b.position = w
                        .get("x")
                        .and_then(|x| x.as_i64())
                        .zip(w.get("y").and_then(|y| y.as_i64()))
                        .map(|(x, y)| [x as i32, y as i32]);
                    if let (Some(w), Some(h)) = (
                        w.get("width").and_then(|v| v.as_i64()),
                        w.get("height").and_then(|v| v.as_i64()),
                    ) {
                        b.size = [w as i32, h as i32];
                    }
                    b.open = w.get("open").and_then(|v| v.as_bool()).unwrap_or(false);
                }
            }
            let state = root.join("config/state.json");
            if state.exists() {
                let v: serde_json::Value = read(&state)?;
                if let Some(d) = v
                    .pointer("/settings/widgetDesign/notes")
                    .and_then(|v| v.as_str())
                {
                    b.design = d.into();
                }
                b.opacity = v
                    .pointer("/settings/widgetOpacity/notes")
                    .or_else(|| v.pointer("/settings/backgroundOpacity"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.) as f32;
                b.frosted = v
                    .pointer("/settings/widgetFrosted/notes")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
            }
            b
        };
        board.validate()?;
        let mut extracted = false;
        for note in &mut board.notes {
            extracted |= extract_rtf_pictures(note, root);
        }
        if extracted {
            let _ = board.save(root);
        }
        Ok(board)
    }
    pub fn validate(&mut self) -> Result<(), String> {
        if self.version != 1 || !DESIGNS.contains(&self.design.as_str()) || self.notes.len() > 400 {
            return Err("Unsupported native notes configuration; file left untouched".into());
        }
        let mut ids = std::collections::HashSet::new();
        for n in &self.notes {
            if n.id.is_empty()
                || !ids.insert(&n.id)
                || !COLORS.contains(&n.color.as_str())
                || n.body.len() > 800_000
                || n.rtf
                    .as_ref()
                    .is_some_and(|r| r.len() > 8_000_000 || !r.starts_with("{\\rtf"))
            {
                return Err("Invalid note data; source left untouched".into());
            }
        }
        self.size = [
            self.size[0].clamp(MIN_SIZE, 1600),
            self.size[1].clamp(MIN_SIZE, 1200),
        ];
        if !self.opacity.is_finite() || !self.tint.is_finite() {
            return Err("Invalid notes appearance".into());
        }
        self.opacity = self.opacity.clamp(0., 100.);
        self.tint = self.tint.clamp(0., 100.);
        if !self.frost_strength.is_finite() || self.blur.is_some_and(|v| !v.is_finite()) {
            return Err("Invalid notes frost parameters".into());
        }
        self.frost_strength = self.frost_strength.clamp(0., 100.);
        self.blur = self.blur.map(|v| v.clamp(0., 64.));
        if let Some(key) = crate::surface::normalize(&self.palette) {
            self.palette = key;
        }
        if !crate::surface::valid(&self.palette) {
            return Err("Unknown native notes surface palette; file left untouched".into());
        }
        Ok(())
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let folder = root.join("config");
        fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let dest = folder.join("native-notes.json");
        let tmp = folder.join("native-notes.json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let mut f = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        use std::io::Write;
        f.write_all(&bytes)
            .and_then(|_| f.sync_all())
            .map_err(|e| e.to_string())?;
        drop(f);
        unsafe {
            use windows_sys::Win32::Storage::FileSystem::*;
            if MoveFileExW(
                wide(&tmp).as_ptr(),
                wide(&dest).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
    pub fn create(&mut self) -> Result<String, String> {
        if self.notes.len() >= 400 {
            return Err("The board already contains 400 notes.".into());
        }
        let stamp = now();
        let mut id = format!("native-{stamp}");
        while self.notes.iter().any(|n| n.id == id) {
            id.push('n');
        }
        self.notes.push(Note {
            id: id.clone(),
            body: String::new(),
            rtf: None,
            images: vec![],
            color: COLORS[self.notes.len() % COLORS.len()].into(),
            pinned: false,
            created_at: stamp,
            updated_at: stamp,
        });
        Ok(id)
    }
    pub fn visible(&self, query: &str) -> Vec<usize> {
        let query = query.to_lowercase();
        let mut list: Vec<_> = self
            .notes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.plain().to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
        list.sort_by(|&a, &b| {
            self.notes[b]
                .pinned
                .cmp(&self.notes[a].pinned)
                .then(self.notes[b].updated_at.cmp(&self.notes[a].updated_at))
                .then(self.notes[a].id.cmp(&self.notes[b].id))
        });
        list
    }
}
fn read<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T, String> {
    let bytes = fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| format!("Cannot read {}: {e}. File left untouched.", p.display()))
}
fn escape_rtf(text: &str, out: &mut String) {
    for c in text.encode_utf16() {
        match c {
            92 | 123 | 125 => {
                out.push('\\');
                out.push(char::from_u32(c as u32).unwrap());
            }
            10 => out.push_str("\\line "),
            13 => {}
            0..=127 => out.push(char::from_u32(c as u32).unwrap()),
            _ => out.push_str(&format!("\\u{}?", c as i16)),
        }
    }
}
/// Translate only the old editor's semantic tags; arbitrary HTML is never executed.
pub fn html(input: &str, rich: bool) -> String {
    let mut out = if rich {
        "{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Segoe UI Variable Text;}}\\uc1\\fs21 ".into()
    } else {
        String::new()
    };
    let mut rest = input;
    let mut ordered: Vec<Option<usize>> = vec![];
    while !rest.is_empty() {
        if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                let tag = rest[1..end].trim().to_lowercase();
                let tag = tag.split_whitespace().next().unwrap_or("");
                let ctl = match tag {
                    "b" | "strong" => "\\b ",
                    "/b" | "/strong" => "\\b0 ",
                    "i" | "em" => "\\i ",
                    "/i" | "/em" => "\\i0 ",
                    "u" => "\\ul ",
                    "/u" => "\\ul0 ",
                    "s" | "strike" => "\\strike ",
                    "/s" | "/strike" => "\\strike0 ",
                    "br" | "br/" | "/div" | "/p" | "/li" => "\\par ",
                    _ => "",
                };
                if rich {
                    out.push_str(ctl);
                } else if ctl == "\\par " {
                    out.push('\n');
                }
                match tag {
                    "ul" => ordered.push(None),
                    "ol" => ordered.push(Some(0)),
                    "/ul" | "/ol" => {
                        ordered.pop();
                    }
                    "li" => {
                        let prefix = if let Some(Some(i)) = ordered.last_mut() {
                            *i += 1;
                            format!("{i}. ")
                        } else {
                            "• ".into()
                        };
                        if rich {
                            escape_rtf(&prefix, &mut out);
                        } else {
                            out.push_str(&prefix);
                        }
                    }
                    _ => {}
                }
                rest = &rest[end + 1..];
                continue;
            }
        }
        if rest.starts_with('&') {
            if let Some(end) = rest.find(';').filter(|e| *e < 16) {
                let e = &rest[1..end];
                let decoded = match e {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" | "#39" => Some('\''),
                    "nbsp" => Some(' '),
                    _ => e
                        .strip_prefix("#x")
                        .and_then(|v| u32::from_str_radix(v, 16).ok())
                        .or_else(|| e.strip_prefix('#').and_then(|v| v.parse().ok()))
                        .and_then(char::from_u32),
                };
                if let Some(c) = decoded {
                    if rich {
                        escape_rtf(&c.to_string(), &mut out);
                    } else {
                        out.push(c);
                    }
                    rest = &rest[end + 1..];
                    continue;
                }
            }
        }
        let c = rest.chars().next().unwrap();
        if rich {
            escape_rtf(&c.to_string(), &mut out);
        } else {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    if rich {
        out.push('}');
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_bounds_round_trip_without_changing_larger_saved_bounds() {
        for size in [[160, 160], [180, 210], [240, 240], [720, 620]] {
            let mut b = Board::default();
            b.size = size;
            b.position = Some([-560, 120]);
            b.create().unwrap();
            b.notes[0].body = "Keep this note".into();
            let mut restored: Board =
                serde_json::from_slice(&serde_json::to_vec(&b).unwrap()).unwrap();
            restored.validate().unwrap();
            assert_eq!(restored.size, size);
            assert_eq!(restored.position, b.position);
            assert_eq!(restored.notes[0].body, b.notes[0].body);
        }
        let mut b = Board::default();
        b.size = [1, 120];
        b.validate().unwrap();
        assert_eq!(b.size, [MIN_SIZE, MIN_SIZE]);
    }
    #[test]
    fn imports_actual_electron_shape_with_slate_and_all_seven_colors_losslessly() {
        let mut fresh = Board::default();
        for color in COLORS {
            fresh.create().unwrap();
            assert_eq!(fresh.notes.last().unwrap().color, color);
        }
        let root = std::env::temp_dir().join(format!(
            "lumen-legacy-colors-{}-{}",
            std::process::id(),
            now()
        ));
        fs::create_dir_all(root.join("config")).unwrap();
        // Same field types and structure as the live source; synthetic content.
        let colors = ["sky", "slate", "peach", "amber", "rose", "mint", "violet"];
        let notes:Vec<_>=colors.iter().enumerate().map(|(i,color)|serde_json::json!({
            "id":format!("legacy-{i}"),"body":"<b>Ghi chú mẫu</b><br>Keep formatting &amp; text",
            "color":color,"pinned":i==1,"createdAt":1730000000000u64+i as u64,
            "updatedAt":1730000001000u64+i as u64
        })).collect();
        let source = serde_json::json!({"version":1,"notes":notes,"window":{"x":12,"y":34,"width":700,"height":600,"open":true}});
        let bytes = serde_json::to_vec_pretty(&source).unwrap();
        let path = root.join("config/notes.json");
        fs::write(&path, &bytes).unwrap();
        let imported = Board::load(&root).unwrap();
        assert_eq!(imported.notes.len(), 7);
        for (i, n) in imported.notes.iter().enumerate() {
            assert_eq!(n.color, colors[i]);
            assert_eq!(n.id, format!("legacy-{i}"));
            assert_eq!(n.body, source["notes"][i]["body"].as_str().unwrap());
            assert_eq!(n.created_at, 1730000000000 + i as u64);
            assert_eq!(n.updated_at, 1730000001000 + i as u64);
            assert_eq!(n.pinned, i == 1);
        }
        assert_eq!(imported.position, Some([12, 34]));
        assert_eq!(imported.size, [700, 600]);
        assert!(imported.open);
        assert!(
            !root.join("config/native-notes.json").exists(),
            "Import must not itself write files"
        );
        imported.save(&root).unwrap();
        let saved = Board::load(&root).unwrap();
        assert_eq!(saved.notes.len(), 7);
        assert_eq!(saved.notes[1].color, "slate");
        assert_eq!(
            fs::read(&path).unwrap(),
            bytes,
            "Electron source must remain byte-for-byte unchanged"
        );
        let mut invalid = saved;
        invalid.notes[1].color = "unknown-color".into();
        assert!(
            invalid.validate().is_err(),
            "Unrecognized native data is still rejected"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "Read-only diagnostic of current local Electron notes; prints no content"]
    fn local_legacy_notes_load_without_writes() {
        let root = crate::model::root();
        let path = root.join("config/notes.json");
        let before = fs::read(&path).unwrap();
        let source: serde_json::Value = serde_json::from_slice(&before).unwrap();
        let native = root.join("config/native-notes.json");
        assert!(!native.exists(), "Run only before native migration");
        let loaded = Board::load(&root).unwrap();
        let expected = source["notes"].as_array().unwrap();
        assert_eq!(loaded.notes.len(), expected.len());
        for (n, old) in loaded.notes.iter().zip(expected) {
            assert!(
                n.id == old["id"].as_str().unwrap(),
                "Imported ID mismatch (content withheld)"
            );
            assert!(
                n.body == old["body"].as_str().unwrap(),
                "Imported body mismatch (content withheld)"
            );
            assert_eq!(n.color, old["color"].as_str().unwrap());
        }
        assert!(
            fs::read(&path).unwrap() == before,
            "Source changed (content withheld)"
        );
        assert!(!native.exists());
    }
    #[test]
    fn appearance_and_bounds_round_trip_without_rewriting_electron_data() {
        let root = std::env::temp_dir().join(format!(
            "lumen-notes-settings-{}-{}",
            std::process::id(),
            now()
        ));
        fs::create_dir_all(root.join("config")).unwrap();
        let mut original = Board::default();
        original.create().unwrap();
        original.notes[0].body = "<u>Keep original</u>".into();
        let old = serde_json::to_vec(&original).unwrap();
        fs::write(root.join("config/notes.json"), &old).unwrap();
        let state =
            br#"{"settings":{"widgetDesign":{"notes":"notebook"},"widgetOpacity":{"notes":35}}}"#;
        fs::write(root.join("config/state.json"), state).unwrap();
        let mut board = Board::load(&root).unwrap();
        assert_eq!(board.design, "notebook");
        assert_eq!(board.opacity, 35.);
        let id = board.notes[0].id.clone();
        board.design = "index".into();
        board.position = Some([-720, 120]);
        board.size = [880, 540];
        board.floating = true;
        board.notes[0].rtf = Some(html(&board.notes[0].body, true));
        board.notes[0].body = "Keep original".into();
        board.save(&root).unwrap();
        let saved = Board::load(&root).unwrap();
        assert_eq!(saved.notes[0].id, id);
        assert!(saved.notes[0].rtf.as_ref().unwrap().contains("\\ul "));
        assert_eq!(saved.size, [880, 540]);
        assert_eq!(saved.position, Some([-720, 120]));
        assert!(saved.floating);
        assert_eq!(saved.design, "index");
        assert_eq!(saved.opacity, 35.);
        let expected_note = serde_json::to_value(&board.notes[0]).unwrap();
        for (palette, expected) in [
            ("gradient:afterglow", "gradient:afterglow"),
            ("custom:#7A4D28", "custom:#7A4D28"),
            ("ocean", "custom:#1C2E3A"),
        ] {
            board.design = "honey".into();
            board.palette = palette.into();
            board.save(&root).unwrap();
            let restored = Board::load(&root).unwrap();
            assert_eq!(restored.design, "honey");
            assert_eq!(restored.palette, expected);
            assert_eq!(restored.position, Some([-720, 120]));
            assert_eq!(restored.size, [880, 540]);
            assert_eq!(
                serde_json::to_value(&restored.notes[0]).unwrap(),
                expected_note
            );
        }
        assert_eq!(fs::read(root.join("config/notes.json")).unwrap(), old);
        assert_eq!(fs::read(root.join("config/state.json")).unwrap(), state);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn html_preserves_unicode_formatting_and_escapes_rtf() {
        let s = "<b>Tiếng Việt 😀</b><br><ul><li>A &amp; B</li></ul>{\\}";
        assert_eq!(html(s, false), "Tiếng Việt 😀\n• A & B\n{\\}");
        let r = html(s, true);
        assert!(r.contains("\\b "));
        assert!(r.contains("\\b0 "));
        assert!(r.contains("\\u"));
        assert!(r.ends_with("\\{\\\\\\}}"));
        assert_eq!(html("<ol><li>A</li><li>B</li></ol>", false), "1. A\n2. B\n");
    }
    #[test]
    fn pin_search_and_limit_never_discard_notes() {
        let mut b = Board::default();
        b.create().unwrap();
        b.create().unwrap();
        b.notes[0].pinned = true;
        b.notes[0].body = "<b>Việt</b>".into();
        assert_eq!(b.visible("")[0], 0);
        assert_eq!(b.visible("VIỆT"), vec![0]);
        for _ in 2..400 {
            b.create().unwrap();
        }
        assert!(b.create().is_err());
        assert_eq!(b.notes.len(), 400);
    }
    #[test]
    fn corrupt_native_file_does_not_fall_back_or_overwrite_source() {
        let root =
            std::env::temp_dir().join(format!("lumen-notes-{}-{}", std::process::id(), now()));
        fs::create_dir_all(root.join("config")).unwrap();
        let path = root.join("config/native-notes.json");
        fs::write(&path, b"broken").unwrap();
        assert!(Board::load(&root).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"broken");
        fs::remove_file(path).unwrap();
        let mut b = Board::default();
        b.create().unwrap();
        b.notes[0].body = "<b>Keep me</b>".into();
        let old = serde_json::to_vec(&b).unwrap();
        fs::write(root.join("config/notes.json"), &old).unwrap();
        let imported = Board::load(&root).unwrap();
        imported.save(&root).unwrap();
        assert_eq!(fs::read(root.join("config/notes.json")).unwrap(), old);
        assert_eq!(Board::load(&root).unwrap().notes[0].body, "<b>Keep me</b>");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn extract_rtf_pictures_migrates_embedded_png_to_images_and_strips_pict() {
        let root = std::env::temp_dir()
            .join(format!("lumen-notes-pict-{}-{}", std::process::id(), now()));
        fs::create_dir_all(root.join("config")).unwrap();
        let img = image::RgbaImage::from_pixel(4, 3, image::Rgba([255, 0, 0, 255]));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let hex: String = png.iter().map(|b| format!("{b:02x}")).collect();
        let mut b = Board::default();
        b.create().unwrap();
        b.notes[0].body = "Hello".into();
        b.notes[0].rtf = Some(format!(
            "{{\\rtf1\\ansi Hello{{\\pict{{\\*\\picprop}}\\pngblip\\picw100\\pich75 \r\n{hex}\r\n}}\\par}}"
        ));
        assert!(extract_rtf_pictures(&mut b.notes[0], &root));
        assert_eq!(b.notes[0].images.len(), 1);
        assert!(images_dir(&root).join(&b.notes[0].images[0]).exists());
        assert!(!b.notes[0].rtf.as_deref().unwrap().contains("\\pict"));
        assert!(!extract_rtf_pictures(&mut b.notes[0], &root));
        fs::remove_dir_all(root).unwrap();
    }
}
