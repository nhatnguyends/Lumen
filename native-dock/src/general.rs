//! General Settings UI. The dock owns widget lifecycles and consumes actions.
//! Wire ACTION_MESSAGE to Host::take_actions; this module never mutates a widget's
//! private config or launches another process.
use crate::{
    glass::Glass,
    model::wide,
    settings_ui::{self, Change, Row, Value},
};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};
use windows_sys::Win32::{Foundation::HWND, UI::WindowsAndMessaging::WM_APP};

pub const ACTION_MESSAGE: u32 = WM_APP + 81;
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Show(String),
    Hide(String),
    Settings(String),
    ShowAll,
    HideAll,
    Opacity(f32),
    Floating(bool),
    RestartUpdate,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct Config {
    version: u32,
    opacity: f32,
    floating: bool,
    snap_fuse: bool,
    start_with_windows: bool,
    position: Option<[i32; 2]>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            opacity: 100.,
            floating: false,
            snap_fuse: true,
            start_with_windows: true,
            position: None,
        }
    }
}
impl Config {
    fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("config/native-general.json");
        let mut c: Self = if path.exists() {
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Cannot read General Settings: {e}. File left untouched."))?
        } else {
            Self::default()
        };
        if c.version != 1 || !c.opacity.is_finite() {
            return Err("Unsupported General Settings. File left untouched.".into());
        }
        c.opacity = c.opacity.clamp(0., 100.);
        crate::fusion::set_enabled(c.snap_fuse);
        Ok(c)
    }
    fn save(&self, root: &Path) -> Result<(), String> {
        let dir = root.join("config");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let temp = dir.join("native-general.json.tmp");
        let dest = dir.join("native-general.json");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        unsafe {
            use windows_sys::Win32::Storage::FileSystem::*;
            if MoveFileExW(
                wide(temp).as_ptr(),
                wide(dest).as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            ) == 0
            {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
        Ok(())
    }
}

pub fn load_preferences(root: &Path) -> Result<(), String> {
    let config = Config::load(root)?;
    if crate::update::portable(root) && !std::env::args().any(|arg| arg == "--smoke-test") {
        crate::startup::set_enabled(root, config.start_with_windows)?;
    }
    Ok(())
}
pub struct Host {
    ui: settings_ui::Host,
    root: PathBuf,
    config: Config,
    packages: Vec<crate::widget_packages::Manifest>,
    visibility: Vec<(String, bool)>,
    error: String,
    smoke: bool,
    update_rx: Option<std::sync::mpsc::Receiver<Result<Option<crate::update::Release>, String>>>,
    update_candidate: Option<crate::update::Release>,
    update_status: String,
}
impl Host {
    pub unsafe fn create(
        owner: HWND,
        root: &Path,
        glass: Rc<RefCell<Glass>>,
        smoke: bool,
    ) -> Result<Self, String> {
        let config = Config::load(root)?;
        let ui = settings_ui::Host::create(
            owner,
            "General Settings",
            "Manage widgets and shared appearance.",
            glass,
            vec![],
            ACTION_MESSAGE,
        )?;
        let mut host = Self {
            ui,
            root: root.into(),
            config,
            packages: vec![],
            visibility: vec![],
            error: String::new(),
            smoke,
            update_rx: None,
            update_candidate: None,
            update_status: String::new(),
        };
        host.update_rows();
        Ok(host)
    }
    fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::section("Widgets")];
        for (i, widget) in self.packages.iter().enumerate() {
            rows.push(Row::Widget {
                id: 1000 + i as u32 * 2,
                settings_id: if widget.has_settings {
                    1001 + i as u32 * 2
                } else {
                    0
                },
                label: widget.name.clone(),
                hint: widget.description.clone(),
                value: self
                    .visibility
                    .iter()
                    .any(|(id, visible)| id == &widget.id && *visible),
            });
        }
        rows.extend([
            Row::button(4, "Show all widgets"),
            Row::button(5, "Hide all widgets"),
            Row::section("General"),
            Row::toggle(
                6,
                "Start with Windows",
                "Open Lumen automatically when you sign in",
                self.config.start_with_windows,
            ),
            Row::slider(1, "Background opacity", self.config.opacity, 0., 100., "%"),
            Row::toggle(
                3,
                "Snap & fuse widgets",
                "Join nearby widgets; drag apart to separate",
                crate::fusion::enabled(),
            ),
            Row::note("Widget themes, content and individual sizes remain independent."),
        ]);
        rows.push(Row::section("Updates"));
        rows.push(Row::note(format!(
            "Lumen {} · Windows x64",
            crate::update::installed_version(&self.root)
        )));
        if self.update_rx.is_none() {
            rows.push(Row::button(7, "Check for updates"));
            if self.update_candidate.is_some()
                && crate::update::portable(&self.root)
                && !crate::update::pending(&self.root)
            {
                rows.push(Row::button(8, "Download update"));
            }
            if crate::update::pending(&self.root) {
                rows.push(Row::button(9, "Restart and apply update"));
            }
            rows.push(Row::button(10, "Open GitHub Releases"));
        }
        if !self.update_status.is_empty() {
            rows.push(Row::note(&self.update_status));
        }
        if !crate::update::portable(&self.root) {
            rows.push(Row::note("Source edition: install portable updates separately. Widget code updates together with Lumen."));
        }
        if !self.error.is_empty() {
            rows.push(Row::note(&self.error));
        }
        rows
    }
    fn update_rows(&mut self) {
        self.ui.set_rows(self.rows());
    }
    pub unsafe fn show(&mut self) {
        self.update_rows();
        self.ui.show();
    }
    pub unsafe fn hide(&mut self) {
        self.ui.hide();
    }
    pub fn hwnd(&self) -> HWND {
        self.ui.hwnd()
    }
    pub fn set_packages(&mut self, packages: Vec<crate::widget_packages::Manifest>) {
        self.packages = packages;
        self.update_rows();
    }
    pub fn set_visibility(&mut self, visibility: Vec<(String, bool)>) {
        if self.visibility == visibility {
            return;
        }
        self.visibility = visibility;
        self.update_rows();
    }
    fn apply_changes(&mut self, changes: Vec<Change>) -> Vec<Action> {
        let mut actions = vec![];
        let mut dirty = false;
        for change in changes {
            match (change.id, change.value) {
                (id, Value::Bool(open)) if id >= 1000 && (id - 1000) % 2 == 0 => {
                    if let Some(widget) = self.packages.get(((id - 1000) / 2) as usize) {
                        actions.push(if open {
                            Action::Show(widget.id.clone())
                        } else {
                            Action::Hide(widget.id.clone())
                        });
                    }
                }
                (id, Value::Action) if id >= 1001 && (id - 1001) % 2 == 0 => {
                    if let Some(widget) = self.packages.get(((id - 1001) / 2) as usize) {
                        actions.push(Action::Settings(widget.id.clone()));
                    }
                }
                (1, Value::Number(v)) if v.is_finite() => {
                    self.config.opacity = v.clamp(0., 100.);
                    actions.push(Action::Opacity(self.config.opacity));
                    dirty = true;
                }
                (6, Value::Bool(v)) => {
                    if crate::update::portable(&self.root) {
                        match crate::startup::set_enabled(&self.root, v) {
                            Ok(()) => {
                                self.config.start_with_windows = v;
                                dirty = true;
                                self.error.clear();
                            }
                            Err(error) => self.error = error,
                        }
                        continue;
                    }
                    use std::os::windows::process::CommandExt;
                    let python =
                        std::env::var_os("LUMEN_PYTHON").unwrap_or_else(|| "python".into());
                    match std::process::Command::new(python)
                        .arg(self.root.join("startup_windows.py"))
                        .arg(if v { "--enable" } else { "--disable" })
                        .creation_flags(0x08000000)
                        .output()
                    {
                        Ok(out) if out.status.success() => {
                            self.config.start_with_windows = v;
                            dirty = true;
                            self.error.clear();
                        }
                        Ok(out) => {
                            self.error = String::from_utf8_lossy(&out.stderr).trim().to_string()
                        }
                        Err(e) => self.error = e.to_string(),
                    }
                }
                (3, Value::Bool(v)) => {
                    self.config.snap_fuse = v;
                    crate::fusion::set_enabled(v);
                    dirty = true;
                }
                (4, Value::Action) => actions.push(Action::ShowAll),
                (5, Value::Action) => actions.push(Action::HideAll),
                (7, Value::Action) if self.update_rx.is_none() && !self.smoke => {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let root = self.root.clone();
                    self.update_rx = Some(rx);
                    self.update_status = "Checking GitHub…".into();
                    std::thread::spawn(move || {
                        let _ = tx.send(crate::update::check(&root));
                    });
                }
                (8, Value::Action) if self.update_rx.is_none() && !self.smoke => {
                    if let Some(candidate) = self.update_candidate.clone() {
                        let (tx, rx) = std::sync::mpsc::channel();
                        let root = self.root.clone();
                        self.update_rx = Some(rx);
                        self.update_status = "Downloading and verifying update…".into();
                        std::thread::spawn(move || {
                            let _ = tx.send(crate::update::stage(&root, &candidate).map(|_| None));
                        });
                    }
                }
                (9, Value::Action) if self.update_rx.is_none() && !self.smoke => {
                    actions.push(Action::RestartUpdate)
                }
                (10, Value::Action) if !self.smoke => unsafe {
                    crate::update::open_releases();
                },
                _ => {}
            }
        }
        if dirty && !self.smoke {
            if let Err(e) = self.config.save(&self.root) {
                self.error = e;
            }
        }
        self.update_rows();
        actions
    }
    pub fn take_actions(&mut self) -> Vec<Action> {
        let changes = self.ui.take_changes();
        self.apply_changes(changes)
    }
    pub fn report_error(&mut self, error: &str) {
        if self.error == error {
            return;
        }
        self.error = error.into();
        self.update_rows();
    }
    pub unsafe fn refresh(&mut self) {
        self.ui.refresh();
    }
    pub fn poll_update(&mut self) {
        let result = self.update_rx.as_ref().map(|rx| rx.try_recv());
        match result {
            Some(Ok(value)) => {
                self.update_rx = None;
                match value {
                    Ok(Some(release)) => {
                        self.update_status = format!("Lumen {} is available.", release.version);
                        self.update_candidate = Some(release);
                    }
                    Ok(None) => {
                        self.update_candidate = None;
                        self.update_status = if crate::update::pending(&self.root) {
                            "Verified update ready. Restart to apply.".into()
                        } else {
                            "No newer compatible stable release is published.".into()
                        };
                    }
                    Err(error) => self.update_status = error,
                }
                self.update_rows();
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                self.update_rx = None;
                self.update_status = "Update worker stopped. Try again.".into();
                self.update_rows();
            }
            _ => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_controls_follow_worker_state_without_starting_network_in_tests() {
        let _lock=crate::native_ui_test_lock();let _runtime=crate::render::Runtime::start().unwrap();
        let root=std::env::temp_dir().join(format!("lumen-update-ui-{}",std::process::id()));std::fs::create_dir_all(&root).unwrap();
        unsafe {
            let glass=Rc::new(RefCell::new(Glass::new(13.,1.)));
            let mut host=Host::create(std::ptr::null_mut(),&root,glass,true).unwrap();
            let has=|rows:&Vec<Row>,wanted| rows.iter().any(|row|matches!(row,Row::Button{id,..} if *id==wanted));
            assert!(has(&host.rows(),7));assert!(!has(&host.rows(),8));
            let (tx,rx)=std::sync::mpsc::channel();host.update_rx=Some(rx);assert!(!has(&host.rows(),7));
            tx.send(Ok(Some(crate::update::Release{version:"0.2.0".into(),url:String::new(),digest:String::new()}))).unwrap();host.poll_update();
            assert!(host.update_status.contains("0.2.0"));assert!(has(&host.rows(),7));assert!(!has(&host.rows(),8),"source builds must not install portable updates over source");
            std::fs::write(root.join("lumen-portable.json"),"{}").unwrap();assert!(has(&host.rows(),8));
            let (tx,rx)=std::sync::mpsc::channel();host.update_rx=Some(rx);tx.send(Err("offline".into())).unwrap();host.poll_update();assert_eq!(host.update_status,"offline");assert!(has(&host.rows(),7));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn general_config_round_trips_and_rejects_corruption() {
        assert!(Config::default().start_with_windows);
        let root = std::env::temp_dir().join(format!("lumen-general-model-{}", std::process::id()));
        let c = Config {
            opacity: 42.,
            floating: true,
            start_with_windows: false,
            ..Config::default()
        };
        c.save(&root).unwrap();
        let c = Config::load(&root).unwrap();
        assert_eq!(c.opacity, 42.);
        assert!(c.floating);
        assert!(!c.start_with_windows);
        let file = root.join("config/native-general.json");
        std::fs::write(&file, "{broken").unwrap();
        assert!(Config::load(&root).is_err());
        assert_eq!(std::fs::read_to_string(file).unwrap(), "{broken");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn hidden_host_emits_real_widget_actions_and_releases_surfaces() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = crate::render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-general-adapter-{}", std::process::id()));
        unsafe {
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            crate::fusion::init(glass.clone());
            let mut host = Host::create(std::ptr::null_mut(), &root, glass, true).unwrap();
            let registry = crate::widget_packages::Registry::discover(&crate::model::root());
            host.set_packages(registry.manifests().to_vec());
            host.set_visibility(registry.ids().into_iter().map(|id| (id, true)).collect());
            let (width, height, pixels) = host.ui.render_for_test();
            assert!(pixels.chunks_exact(4).any(|p| p[3] > 200));
            if let Some(path) = std::env::var_os("LUMEN_MANAGER_PREVIEW") {
                let mut rgba = pixels.clone();
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                    if pixel[3] > 0 {
                        for c in 0..3 {
                            pixel[c] = (pixel[c] as u32 * 255 / pixel[3] as u32).min(255) as u8;
                        }
                    }
                }
                image::save_buffer(
                    path,
                    &rgba,
                    width as u32,
                    height as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
            let changes = registry
                .manifests()
                .iter()
                .enumerate()
                .map(|(i, _)| Change {
                    id: 1001 + i as u32 * 2,
                    value: Value::Action,
                })
                .collect();
            assert_eq!(
                host.apply_changes(changes),
                registry
                    .ids()
                    .into_iter()
                    .map(Action::Settings)
                    .collect::<Vec<_>>()
            );
            let toggles = registry
                .ids()
                .into_iter()
                .enumerate()
                .map(|(i, _)| Change {
                    id: 1000 + i as u32 * 2,
                    value: Value::Bool(i % 2 == 0),
                })
                .collect();
            let expected = registry
                .ids()
                .into_iter()
                .enumerate()
                .map(|(i, id)| {
                    if i % 2 == 0 {
                        Action::Show(id)
                    } else {
                        Action::Hide(id)
                    }
                })
                .collect::<Vec<_>>();
            assert_eq!(host.apply_changes(toggles), expected);
            let actions = host.apply_changes(vec![
                Change {
                    id: 1,
                    value: Value::Number(35.),
                },
                Change {
                    id: 2,
                    value: Value::Bool(true),
                },
                Change {
                    id: 3,
                    value: Value::Bool(false),
                },
                Change {
                    id: 4,
                    value: Value::Action,
                },
                Change {
                    id: 5,
                    value: Value::Action,
                },
            ]);
            assert_eq!(
                actions,
                vec![Action::Opacity(35.), Action::ShowAll, Action::HideAll]
            );
            assert!(!crate::fusion::enabled());
            host.hide();
            drop(host);
            crate::fusion::shutdown();
        }
        assert!(!root.join("config/native-general.json").exists());
    }
}
