use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub name: String,
    pub target: String,
    #[serde(default = "app_kind")]
    pub kind: String,
    #[serde(default)]
    pub args: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub icon_source: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(skip)]
    pub catalog: bool,
}
fn pinned_items<S: serde::Serializer>(items: &[Item], serializer: S) -> Result<S::Ok, S::Error> {
    items
        .iter()
        .filter(|i| !i.catalog)
        .collect::<Vec<_>>()
        .serialize(serializer)
}
fn app_kind() -> String {
    "app".into()
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Dock {
    pub version: u32,
    #[serde(serialize_with = "pinned_items")]
    pub items: Vec<Item>,
    pub columns: usize,
    pub light: bool,
    pub floating: bool,
    pub position: Option<[i32; 2]>,
    pub size: Option<[i32; 2]>,
    pub blur: f32,
    pub frosted: bool,
    pub tint: f32,
    pub frost_strength: f32,
    pub opacity: f32,
    pub icon_size: i32,
    pub show_labels: bool,
    pub show_search: bool,
    pub theme: String,
}
impl Default for Dock {
    fn default() -> Self {
        Self {
            version: 1,
            items: vec![],
            columns: 8,
            light: false,
            floating: false,
            position: None,
            size: None,
            blur: 13.0,
            frosted: true,
            tint: 0.0,
            frost_strength: 100.0,
            opacity: 100.0,
            icon_size: 48,
            show_labels: true,
            show_search: true,
            theme: crate::accent::THEME.into(),
        }
    }
}
impl Dock {
    pub fn ungroup(&mut self, id: &str) {
        if let Some(item) = self.items.iter_mut().find(|item| item.id == id) {
            item.group = None;
        }
        self.clean_groups();
    }
    fn clean_groups(&mut self) {
        let mut counts = std::collections::HashMap::new();
        for item in &self.items {
            if let Some(group) = &item.group {
                *counts.entry(group.clone()).or_insert(0) += 1;
            }
        }
        for item in &mut self.items {
            if item
                .group
                .as_ref()
                .is_some_and(|g| counts.get(g).copied().unwrap_or(0) < 2)
            {
                item.group = None;
            }
        }
    }
    pub fn group_selection(&mut self, ids: &std::collections::HashSet<String>) -> bool {
        if ids.len() < 2
            || ids
                .iter()
                .any(|id| !self.items.iter().any(|i| i.id == *id && !i.catalog))
        {
            return false;
        }
        let first = &self.items.iter().find(|i| ids.contains(&i.id)).unwrap().id;
        let mut suffix = 0u32;
        let group = loop {
            let name = format!("native-selection-{first}-{suffix}");
            if !self
                .items
                .iter()
                .any(|i| i.group.as_deref() == Some(name.as_str()))
            {
                break name;
            }
            suffix += 1;
        };
        for item in &mut self.items {
            if ids.contains(&item.id) {
                item.group = Some(group.clone());
            }
        }
        self.clean_groups();
        true
    }
    #[cfg(test)]
    pub fn group_items(&mut self, id: &str, other: &str) {
        if id == other || !self.items.iter().any(|i| i.id == id) {
            return;
        }
        let Some(target) = self.items.iter().find(|i| i.id == other) else {
            return;
        };
        let group = target
            .group
            .clone()
            .unwrap_or_else(|| format!("native-group-{}", target.id));
        for item in &mut self.items {
            if item.id == id || item.id == other {
                item.group = Some(group.clone());
            }
        }
        self.clean_groups();
    }
    pub fn load(root: &Path) -> Result<Self, String> {
        let own = root.join("config/native-dock.json");
        let mut dock: Self = if own.exists() {
            serde_json::from_slice(&fs::read(&own).map_err(|e| e.to_string())?).map_err(|e| {
                format!("Cannot read native dock config: {e}. The file was left untouched.")
            })?
        } else {
            let source = root.join("config/state.json");
            if source.exists() {
                let value: serde_json::Value =
                    serde_json::from_slice(&fs::read(source).map_err(|e| e.to_string())?)
                        .map_err(|e| format!("Cannot import Electron dock: {e}"))?;
                let items: Vec<Item> = serde_json::from_value(
                    value
                        .get("items")
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!([])),
                )
                .map_err(|e| format!("Cannot import dock items: {e}"))?;
                Self {
                    items,
                    columns: value
                        .pointer("/settings/columns")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(8) as usize,
                    size: Some([746, 370]),
                    position: match (
                        value
                            .pointer("/settings/position/x")
                            .and_then(|v| v.as_i64()),
                        value
                            .pointer("/settings/position/y")
                            .and_then(|v| v.as_i64()),
                    ) {
                        (Some(x), Some(y)) => {
                            let dpi = value
                                .pointer("/settings/anchor/scaleFactor")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(1.0);
                            Some([
                                ((x as f64 + 44.0) * dpi) as i32,
                                ((y as f64 + 44.0) * dpi) as i32,
                            ])
                        }
                        _ => None,
                    },
                    blur: value
                        .pointer("/settings/widgetAppearance/launcher/blurSize")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(13.0) as f32,
                    frosted: value
                        .pointer("/settings/widgetFrosted/launcher")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    tint: value
                        .pointer("/settings/widgetAppearance/launcher/veil")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.) as f32,
                    frost_strength: value
                        .pointer("/settings/widgetAppearance/launcher/frostStrength")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(100.) as f32,
                    opacity: value
                        .pointer("/settings/widgetOpacity/launcher")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(100.) as f32,
                    icon_size: value
                        .pointer("/settings/iconSize")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(48) as i32,
                    show_labels: value
                        .pointer("/settings/showLabels")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    show_search: value
                        .pointer("/settings/showSearch")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    theme: value
                        .pointer("/settings/widgetTheme/launcher")
                        .or_else(|| value.pointer("/settings/theme"))
                        .and_then(|v| v.as_str())
                        .unwrap_or(crate::accent::THEME)
                        .into(),
                    ..Self::default()
                }
            } else {
                Self::default()
            }
        };
        if dock.version != 1 {
            return Err("Unsupported native dock config version; file left untouched.".into());
        }
        dock.columns = dock.columns.clamp(3, 12);
        dock.blur = dock.blur.clamp(0.0, 80.0);
        dock.tint = dock.tint.clamp(0.0, 100.0);
        dock.frost_strength = dock.frost_strength.clamp(0.0, 100.0);
        dock.opacity = dock.opacity.clamp(0.0, 100.0);
        dock.icon_size = dock.icon_size.clamp(36, 60);
        if dock.theme == "windows-accent" {
            dock.theme = crate::accent::THEME.into();
        }
        dock.theme =
            crate::surface::normalize(&dock.theme).unwrap_or_else(|| crate::accent::THEME.into());
        if let Some(size) = &mut dock.size {
            size[0] = size[0].clamp(crate::geometry::DOCK_MIN[0], 1800);
            size[1] = size[1].clamp(crate::geometry::DOCK_MIN[1], 1000);
        }
        dock.items.retain(|i| !i.target.trim().is_empty());
        Ok(dock)
    }
    pub fn add(&mut self, target: &str) -> bool {
        if let Some(item) = self.items.iter_mut().find(|i| {
            i.target.eq_ignore_ascii_case(target) && i.args.as_deref().unwrap_or("").is_empty()
        }) {
            if item.catalog {
                item.catalog = false;
                return true;
            }
            return false;
        }
        let path = Path::new(target);
        let name = if path.is_dir() {
            path.file_name()
        } else {
            path.file_stem()
        }
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
        self.items.push(Item {
            id: format!(
                "native-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            name,
            target: target.into(),
            kind: if path.is_dir() { "folder" } else { "file" }.into(),
            args: None,
            cwd: None,
            icon_source: Some(target.into()),
            icon: None,
            group: None,
            catalog: false,
        });
        true
    }
    pub fn matches(&self, query: &str) -> Vec<usize> {
        if query.trim().is_empty() {
            return self
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| !i.catalog)
                .map(|(n, _)| n)
                .collect();
        }
        let mut matches: Vec<_> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(n, item)| {
                let score = crate::catalog::score(&item.name, query);
                (score > 0).then_some((n, score))
            })
            .collect();
        matches.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| self.items[a.0].catalog.cmp(&self.items[b.0].catalog))
                .then_with(|| self.items[a.0].name.cmp(&self.items[b.0].name))
        });
        matches.into_iter().take(60).map(|(n, _)| n).collect()
    }
    pub fn layout_matches(&self, query: &str, columns: usize) -> Vec<usize> {
        if !query.trim().is_empty() {
            return self.matches(query);
        }
        let columns = columns.max(1);
        let mut groups: Vec<(&str, Vec<usize>)> = vec![];
        let mut singles = std::collections::VecDeque::new();
        for (i, item) in self.items.iter().enumerate() {
            if item.catalog {
                continue;
            }
            if let Some(group) = item.group.as_deref().filter(|s| !s.is_empty()) {
                if let Some((_, members)) = groups.iter_mut().find(|(g, _)| *g == group) {
                    members.push(i);
                } else {
                    groups.push((group, vec![i]));
                }
            } else {
                singles.push_back(i);
            }
        }
        let mut result = vec![];
        for (_, members) in groups {
            if members.len() < 2 {
                singles.extend(members);
                continue;
            }
            let remaining = columns - result.len() % columns;
            if members.len() > remaining {
                while result.len() % columns != 0 {
                    let Some(i) = singles.pop_front() else {
                        break;
                    };
                    result.push(i);
                }
            }
            result.extend(members);
        }
        result.extend(singles);
        result
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let dir = root.join("config");
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let destination = dir.join("native-dock.json");
        let temporary = dir.join("native-dock.json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        {
            use std::io::Write;
            let mut file = fs::File::create(&temporary).map_err(|e| e.to_string())?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| e.to_string())?;
        }
        // Both paths are owned UTF-16 buffers, valid for the duration of this call.
        unsafe {
            use windows_sys::Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            };
            if MoveFileExW(
                wide(temporary.as_os_str()).as_ptr(),
                wide(destination.as_os_str()).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
    pub fn commit_drag(&mut self, order: &[String], moved: &str, group: Option<&str>) -> bool {
        let (pinned, catalog): (Vec<_>, Vec<_>) = std::mem::take(&mut self.items)
            .into_iter()
            .partition(|i| !i.catalog);
        self.items = pinned;
        let success = self.commit_pinned_drag(order, moved, group);
        self.items.extend(catalog);
        success
    }
    fn commit_pinned_drag(&mut self, order: &[String], moved: &str, group: Option<&str>) -> bool {
        let unique: std::collections::HashSet<_> = order.iter().map(String::as_str).collect();
        if order.len() != self.items.len()
            || unique.len() != order.len()
            || self.items.iter().any(|i| !unique.contains(i.id.as_str()))
            || !unique.contains(moved)
        {
            return false;
        }
        if group.is_some_and(|g| {
            !self
                .items
                .iter()
                .any(|i| i.id != moved && i.group.as_deref() == Some(g))
        }) {
            return false;
        }
        let mut items: std::collections::HashMap<_, _> = std::mem::take(&mut self.items)
            .into_iter()
            .map(|i| (i.id.clone(), i))
            .collect();
        self.items = order.iter().map(|id| items.remove(id).unwrap()).collect();
        if let Some(item) = self.items.iter_mut().find(|i| i.id == moved) {
            item.group = group.map(str::to_owned);
        }
        let mut counts = std::collections::HashMap::new();
        for item in &self.items {
            if let Some(g) = &item.group {
                *counts.entry(g.clone()).or_insert(0) += 1;
            }
        }
        for item in &mut self.items {
            if item
                .group
                .as_ref()
                .is_some_and(|g| counts.get(g).copied().unwrap_or(0) < 2)
            {
                item.group = None;
            }
        }
        true
    }
}
pub fn wide(text: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    text.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn root() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            if parent.join("lumen-portable.json").is_file() { return parent.into(); }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "lumen-dock-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(path.join("config")).unwrap();
        path
    }
    #[test]
    fn imports_without_writing_electron_state_and_saves_independently() {
        let root = temp();
        let source = br#"{"items":[{"id":"a","name":"Editor","target":"C:\\Editor.exe","kind":"app","args":"--new-window","unknown":true}],"settings":{"columns":12}}"#;
        fs::write(root.join("config/state.json"), source).unwrap();
        let mut dock = Dock::load(&root).unwrap();
        assert_eq!(dock.items[0].args.as_deref(), Some("--new-window"));
        dock.items[0].name = "Changed".into();
        dock.save(&root).unwrap();
        dock.save(&root).unwrap();
        assert_eq!(Dock::load(&root).unwrap().items[0].name, "Changed");
        assert_eq!(fs::read(root.join("config/state.json")).unwrap(), source);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn checkbox_group_uses_only_checked_pinned_items_and_validates_before_commit() {
        let mut dock = Dock::default();
        for name in ["A", "B", "C", "D", "E"] {
            dock.add(&format!("C:/Apps/{name}.exe"));
        }
        let ids: Vec<_> = dock.items.iter().map(|i| i.id.clone()).collect();
        dock.items[0].group = Some("old".into());
        dock.items[1].group = Some("old".into());
        dock.items[2].group = Some("old".into());
        let checked = [ids[0].clone(), ids[3].clone(), ids[4].clone()]
            .into_iter()
            .collect();
        assert!(dock.group_selection(&checked));
        let group = dock.items[0].group.clone();
        assert!(group.is_some());
        assert_eq!(dock.items[3].group, group);
        assert_eq!(dock.items[4].group, group);
        assert_eq!(dock.items[1].group.as_deref(), Some("old"));
        assert_eq!(dock.items[2].group.as_deref(), Some("old"));
        let before = serde_json::to_vec(&dock).unwrap();
        assert!(!dock.group_selection(&[ids[1].clone()].into_iter().collect()));
        assert!(!dock.group_selection(&[ids[1].clone(), "unknown".into()].into_iter().collect()));
        assert_eq!(serde_json::to_vec(&dock).unwrap(), before);
        dock.items[4].catalog = true;
        assert!(!dock.group_selection(&[ids[1].clone(), ids[4].clone()].into_iter().collect()));
    }
    #[test]
    fn compact_bounds_survive_restart_and_column_changes() {
        let root = temp();
        let mut dock = Dock::default();
        dock.size = Some(crate::geometry::DOCK_MIN);
        dock.save(&root).unwrap();
        let mut restored = Dock::load(&root).unwrap();
        assert_eq!(restored.size, Some(crate::geometry::DOCK_MIN));
        restored.columns = 12;
        restored.save(&root).unwrap();
        assert_eq!(Dock::load(&root).unwrap().size, Some(crate::geometry::DOCK_MIN));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn wallpaper_auto_palette_survives_restart() {
        let root = temp();
        let mut dock = Dock::default();
        dock.theme = crate::accent::THEME.into();
        dock.save(&root).unwrap();
        assert_eq!(Dock::load(&root).unwrap().theme, crate::accent::THEME);
        dock.theme = "windows-accent".into();
        dock.save(&root).unwrap();
        assert_eq!(Dock::load(&root).unwrap().theme, crate::accent::THEME);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn gradient_custom_and_legacy_surface_selections_roundtrip_without_bounds_changes() {
        let root = temp();
        for key in [
            "gradient:afterglow",
            "gradient:dawn",
            "custom:#1A8BE5",
            "porcelain",
        ] {
            let mut dock = Dock::default();
            dock.theme = key.into();
            dock.size = Some([420, 360]);
            dock.position = Some([-1200, 80]);
            dock.save(&root).unwrap();
            let restored = Dock::load(&root).unwrap();
            assert_eq!(restored.theme, crate::surface::normalize(key).unwrap());
            assert_eq!(restored.size, dock.size);
            assert_eq!(restored.position, dock.position);
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corrupt_config_is_not_silently_replaced() {
        let root = temp();
        let path = root.join("config/native-dock.json");
        fs::write(&path, "broken").unwrap();
        assert!(Dock::load(&root).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "broken");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn search_and_duplicate_addition() {
        let mut dock = Dock::default();
        assert!(dock.add("C:\\Apps\\Editor.exe"));
        assert!(!dock.add("c:\\apps\\EDITOR.exe"));
        assert_eq!(dock.matches(" EDIT "), vec![0]);
        assert!(dock.matches("missing").is_empty());
    }
    #[test]
    fn add_accepts_shortcuts_apps_documents_and_folders() {
        let root = temp();
        let folder = root.join("Folder.with.dots");
        fs::create_dir(&folder).unwrap();
        let mut dock = Dock::default();
        for name in ["Example.lnk", "Example.exe", "Notes.txt", "NoExtension"] {
            let path = root.join(name);
            fs::write(&path, b"test fixture").unwrap();
            assert!(dock.add(&path.to_string_lossy()));
            assert_eq!(dock.items.last().unwrap().target, path.to_string_lossy());
        }
        assert!(dock.add(&folder.to_string_lossy()));
        let item = dock.items.last().unwrap();
        assert_eq!(item.kind, "folder");
        assert_eq!(item.name, "Folder.with.dots");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn catalog_results_only_persist_after_explicit_pinning_and_do_not_break_drag() {
        let root = temp();
        let mut dock = Dock::default();
        dock.add("C:\\Pinned.exe");
        dock.add("C:\\Installed.exe");
        dock.items[1].catalog = true;
        assert_eq!(dock.layout_matches("", 6), vec![0]);
        assert_eq!(dock.matches("Installed"), vec![1]);
        let order = vec![dock.items[0].id.clone()];
        assert!(dock.commit_drag(&order, &order[0], None));
        dock.save(&root).unwrap();
        assert_eq!(Dock::load(&root).unwrap().items.len(), 1);
        assert!(dock.add("C:\\Installed.exe"));
        dock.save(&root).unwrap();
        assert_eq!(Dock::load(&root).unwrap().items.len(), 2);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn regroup_and_ungroup_preserve_targets_and_remove_singletons() {
        let mut dock = Dock::default();
        for name in ["A", "B", "C"] {
            dock.add(&format!("C:\\{name}.exe"));
        }
        let ids: Vec<_> = dock.items.iter().map(|i| i.id.clone()).collect();
        dock.group_items(&ids[0], &ids[1]);
        assert_eq!(dock.items[0].group, dock.items[1].group);
        dock.group_items(&ids[0], &ids[2]);
        assert!(dock.items[1].group.is_none());
        dock.ungroup(&ids[0]);
        assert!(dock.items.iter().all(|i| i.group.is_none()));
        assert_eq!(dock.items[0].target, "C:\\A.exe");
    }
    #[test]
    fn responsive_groups_fill_the_row_before_starting_the_next_group() {
        let mut dock = Dock::default();
        for n in 0..7 {
            dock.add(&format!("C:\\App{n}.exe"));
        }
        for i in 0..2 {
            dock.items[i].group = Some("a".into());
        }
        for i in 2..5 {
            dock.items[i].group = Some("b".into());
        }
        assert_eq!(dock.layout_matches("", 3), vec![0, 1, 5, 2, 3, 4, 6]);
        assert_eq!(dock.layout_matches("App2", 3), vec![2]);
    }
    #[test]
    fn drag_commit_is_a_validated_permutation_and_preserves_targets() {
        let mut d = Dock::default();
        for i in 0..4 {
            d.add(&format!("C:\\Apps\\{i}.exe"));
        }
        d.items[0].group = Some("g".into());
        d.items[1].group = Some("g".into());
        let ids: Vec<_> = d.items.iter().map(|i| i.id.clone()).collect();
        let targets: Vec<_> = d.items.iter().map(|i| i.target.clone()).collect();
        assert!(!d.commit_drag(&[ids[0].clone(); 0], &ids[0], None));
        assert_eq!(d.items[0].target, targets[0]);
        let order = vec![
            ids[1].clone(),
            ids[2].clone(),
            ids[0].clone(),
            ids[3].clone(),
        ];
        assert!(d.commit_drag(&order, &ids[0], None));
        assert_eq!(d.items[2].target, targets[0]);
        assert!(d.items.iter().all(|i| i.group.is_none()));
        d.items[0].group = Some("h".into());
        d.items[1].group = Some("h".into());
        assert!(d.commit_drag(&order, &ids[0], Some("h")));
        assert_eq!(d.items[2].group.as_deref(), Some("h"));
        let duplicate = vec![ids[0].clone(); 4];
        assert!(!d.commit_drag(&duplicate, &ids[0], None));
    }
}
