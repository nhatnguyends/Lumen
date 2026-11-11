//! Source package discovery and lazy, UI-thread-owned widget lifecycles.
use crate::glass::Glass;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};
use windows_sys::Win32::Foundation::HWND;

pub trait WidgetHost {
    unsafe fn show(&mut self);
    unsafe fn hide(&mut self);
    unsafe fn settings(&mut self);
    unsafe fn visible(&self) -> bool;
    fn hwnd(&self) -> HWND;
    unsafe fn apply_common(&mut self, opacity: Option<f32>, floating: Option<bool>);
    unsafe fn refresh(&mut self) {}
    unsafe fn tick(&mut self) {}
    fn error(&self) -> Option<String> {
        None
    }
}
pub struct NativeDefinition {
    pub id: &'static str,
    pub factory: fn(&Path, Rc<RefCell<Glass>>, bool, bool) -> Result<Box<dyn WidgetHost>, String>,
    pub should_open: fn(&Path) -> bool,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub description: String,
    pub runtime: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default)]
    pub has_settings: bool,
}
impl Manifest {
    pub fn read(path: &Path) -> Result<Self, String> {
        let m: Self = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if m.schema_version != 1
            || m.id.is_empty()
            || m.id.len() > 64
            || !m
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || m.name.trim().is_empty()
            || m.name.len() > 100
            || m.description.len() > 500
            || m.runtime != "native"
        {
            return Err(format!("Invalid widget package: {}", path.display()));
        }
        if path
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            != Some(m.id.as_str())
        {
            return Err("Package folder must match id".into());
        }
        Ok(m)
    }
}
pub struct Registry {
    root: PathBuf,
    manifests: Vec<Manifest>,
    definitions: HashMap<&'static str, NativeDefinition>,
    hosts: HashMap<String, Box<dyn WidgetHost>>,
    enabled: HashMap<String, bool>,
    visibility_error: Option<String>,
    show_messages: HashMap<u32, String>,
}
impl Registry {
    pub fn discover(root: &Path) -> Self {
        let definitions: Vec<NativeDefinition> =
            include!(concat!(env!("OUT_DIR"), "/widget_definitions.rs"));
        let definitions: HashMap<_, _> = definitions.into_iter().map(|d| (d.id, d)).collect();
        let mut manifests = vec![];
        if let Ok(dirs) = std::fs::read_dir(root.join("widgets")) {
            for dir in dirs.flatten() {
                let path = dir.path().join("widget.json");
                if !path.is_file() {
                    continue;
                }
                match Manifest::read(&path) {
                    Ok(m)
                        if m.runtime == "native"
                            && (m.id == "launcher" || definitions.contains_key(m.id.as_str())) =>
                    {
                        manifests.push(m)
                    }
                    Ok(m) => eprintln!("widget-package {} needs a source rebuild", m.id),
                    Err(e) => eprintln!("widget-package: {e}"),
                }
            }
        }
        manifests.sort_by(|a, b| a.order.cmp(&b.order).then(a.id.cmp(&b.id)));
        let (enabled, visibility_error) =
            match std::fs::read(root.join("config/native-packages.json")) {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(map) => (map, None),
                    Err(e) => (
                        HashMap::new(),
                        Some(format!(
                            "Invalid package visibility preferences: {e}; file left untouched."
                        )),
                    ),
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (HashMap::new(), None),
                Err(e) => (HashMap::new(), Some(e.to_string())),
            };
        if let Some(error) = &visibility_error {
            eprintln!("widget-packages: {error}");
        }
        let show_messages = manifests
            .iter()
            .map(|m| (show_message(&m.id), m.id.clone()))
            .collect();
        Self {
            root: root.into(),
            manifests,
            definitions,
            hosts: HashMap::new(),
            enabled,
            visibility_error,
            show_messages,
        }
    }
    pub fn manifests(&self) -> &[Manifest] {
        &self.manifests
    }
    pub fn message_package(&self, message: u32) -> Option<&str> {
        self.show_messages.get(&message).map(String::as_str)
    }
    pub fn enabled(&self, id: &str, fallback: bool) -> bool {
        self.enabled.get(id).copied().unwrap_or(fallback)
    }
    pub fn shutdown(&mut self) {
        self.hosts.clear();
    }
    pub fn ids(&self) -> Vec<String> {
        self.manifests.iter().map(|m| m.id.clone()).collect()
    }
    pub fn startup_ids(&self) -> Vec<String> {
        self.manifests
            .iter()
            .filter(|m| {
                m.id != "launcher"
                    && self.enabled.get(&m.id).copied().unwrap_or_else(|| {
                        self.definitions
                            .get(m.id.as_str())
                            .is_some_and(|d| (d.should_open)(&self.root))
                    })
            })
            .map(|m| m.id.clone())
            .collect()
    }
    pub fn remember(&mut self, id: &str, enabled: bool, smoke: bool) {
        if smoke || self.enabled.get(id) == Some(&enabled) {
            return;
        }
        self.enabled.insert(id.into(), enabled);
        if let Err(error) = self.save_enabled() {
            eprintln!("widget-packages: {error}");
        }
    }
    fn save_enabled(&self) -> Result<(), String> {
        if let Some(error) = &self.visibility_error {
            return Err(error.clone());
        }
        let dir = self.root.join("config");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let temp = dir.join("native-packages.json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(&self.enabled).unwrap())
            .map_err(|e| e.to_string())?;
        unsafe {
            use windows_sys::Win32::Storage::FileSystem::*;
            if MoveFileExW(
                crate::model::wide(&temp).as_ptr(),
                crate::model::wide(dir.join("native-packages.json")).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
    pub fn ensure(
        &mut self,
        id: &str,
        glass: Rc<RefCell<Glass>>,
        inspect: bool,
        smoke: bool,
    ) -> Result<(), String> {
        if id == "launcher" || self.hosts.contains_key(id) {
            return Ok(());
        }
        let m = self
            .manifests
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| format!("Unknown widget package: {id}"))?;
        let _ = m;
        let host = (self
            .definitions
            .get(id)
            .ok_or("Native package requires a source rebuild")?
            .factory)(&self.root, glass, inspect, smoke)?;
        unsafe {
            crate::fusion::register_widget(host.hwnd());
        }
        self.hosts.insert(id.into(), host);
        Ok(())
    }
    pub fn host(&self, id: &str) -> Option<&dyn WidgetHost> {
        self.hosts.get(id).map(|h| h.as_ref())
    }
    pub fn host_mut(&mut self, id: &str) -> Option<&mut (dyn WidgetHost + '_)> {
        match self.hosts.get_mut(id) {
            Some(h) => Some(h.as_mut()),
            None => None,
        }
    }
    pub unsafe fn tick(&mut self) {
        for h in self.hosts.values_mut() {
            h.tick();
        }
    }
    pub unsafe fn refresh(&mut self) {
        for h in self.hosts.values_mut() {
            h.refresh();
        }
    }
}
pub fn show_message(id: &str) -> u32 {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(
            crate::model::wide(format!("Lumen.SourcePackage.Show.{id}")).as_ptr(),
        )
    }
}
pub fn manager_message() -> u32 {
    static MESSAGE: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *MESSAGE.get_or_init(|| unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(
            crate::model::wide("Lumen.SourcePackage.Manage").as_ptr(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_all_source_packages_in_order() {
        let registry = Registry::discover(&crate::model::root());
        let ids = registry.ids();
        for id in [
            "launcher", "music", "notes", "calendar", "clock", "controls",
        ] {
            assert!(ids.iter().any(|v| v == id));
        }
        assert!(registry.manifests().iter().all(|m| m.runtime == "native"));
        assert!(registry
            .manifests()
            .windows(2)
            .all(|pair| pair[0].order <= pair[1].order));
        assert!(registry.definitions.len() >= 5);
    }
    #[test]
    fn package_identity_and_runtime_are_validated() {
        let root = std::env::temp_dir()
            .join(format!("lumen-manifest-{}", std::process::id()))
            .join("sample");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("widget.json");
        for id in ["../escape", "wrong-folder", "SAMPLE"] {
            std::fs::write(&path,format!(r#"{{"schemaVersion":1,"id":"{id}","name":"Test","description":"","runtime":"native"}}"#)).unwrap();
            assert!(Manifest::read(&path).is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corrupt_visibility_preferences_are_never_overwritten() {
        let root = std::env::temp_dir().join(format!("lumen-package-prefs-{}", std::process::id()));
        std::fs::create_dir_all(root.join("config")).unwrap();
        let file = root.join("config/native-packages.json");
        std::fs::write(&file, "{broken").unwrap();
        let mut registry = Registry::discover(&root);
        registry.remember("clock", true, false);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{broken");
        std::fs::remove_dir_all(root).unwrap();
    }
}
