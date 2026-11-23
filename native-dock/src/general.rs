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
    Pin(String, bool),
    RefreshGlass,
    SyncMultiMonitor,
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
    fast_startup: bool,
    low_power: bool,
    liquid_glass: bool,
    frosted_glass: bool,
    blur: f32,
    position: Option<[i32; 2]>,
    multi_monitor: bool,
    duplicate_displays: Vec<String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            opacity: 100.,
            floating: false,
            snap_fuse: true,
            start_with_windows: true,
            fast_startup: false,
            low_power: false,
            liquid_glass: true,
            frosted_glass: true,
            blur: 13.,
            position: None,
            multi_monitor: false,
            duplicate_displays: vec![],
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
        c.blur = if c.blur.is_finite() {
            c.blur.clamp(0., 80.)
        } else {
            13.
        };
        crate::fusion::set_enabled(c.snap_fuse);
        crate::motion::set_low_power(c.low_power);
        crate::glass::set_liquid_glass(c.liquid_glass);
        crate::glass::set_frosted_glass(c.frosted_glass);
        crate::glass::set_global_blur(c.blur);
        unsafe {
            crate::multi_monitor::configure(c.multi_monitor, c.duplicate_displays.clone());
        }
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
    crate::displays::configure(root);
    let config = Config::load(root)?;
    if crate::update::portable(root) && !std::env::args().any(|arg| arg == "--smoke-test") {
        crate::startup::set_enabled(root, config.start_with_windows)?;
    }
    Ok(())
}
#[derive(Clone, Debug)]
pub enum UpdateMessage {
    Progress(crate::update::Progress),
    Done(Result<Option<crate::update::Release>, String>),
}
pub struct Host {
    ui: settings_ui::Host,
    root: PathBuf,
    config: Config,
    packages: Vec<crate::widget_packages::Manifest>,
    visibility: Vec<(String, bool)>,
    pinned: Vec<String>,
    displays: Vec<crate::displays::DisplayInfo>,
    error: String,
    smoke: bool,
    update_rx: Option<std::sync::mpsc::Receiver<UpdateMessage>>,
    update_candidate: Option<crate::update::Release>,
    update_status: String,
    update_progress: Option<f32>,
    auto_apply_update: bool,
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
            "Lumen Settings",
            "Your widgets, your desktop.",
            glass,
            vec![],
            ACTION_MESSAGE,
        )?;
        let displays = crate::displays::enumerate();
        let mut host = Self {
            ui,
            root: root.into(),
            config,
            packages: vec![],
            visibility: vec![],
            pinned: vec![],
            displays,
            error: String::new(),
            smoke,
            update_rx: None,
            update_candidate: None,
            update_status: String::new(),
            update_progress: None,
            auto_apply_update: false,
        };
        host.update_rows();
        Ok(host)
    }
    fn rows(&self) -> Vec<Row> {
        let mut rows = vec![Row::section("Widgets")];
        for (i, widget) in self.packages.iter().enumerate() {
            rows.push(Row::Widget {
                id: 1000 + i as u32 * 3,
                settings_id: if widget.has_settings {
                    1001 + i as u32 * 3
                } else {
                    0
                },
                pin_id: if widget.id == "launcher" {
                    0
                } else {
                    1002 + i as u32 * 3
                },
                label: widget.name.clone(),
                hint: widget.description.clone(),
                value: self
                    .visibility
                    .iter()
                    .any(|(id, visible)| id == &widget.id && *visible),
                pinned: self.pinned.iter().any(|p| p == &widget.id),
            });
        }
        rows.extend([
            Row::section("Background"),
            Row::toggle(
                13,
                "Liquid glass",
                "Subtle edge refraction and chromatic shift on glass surfaces",
                self.config.liquid_glass,
            ),
            Row::toggle(
                16,
                "Frosted glass",
                "Blend widget backgrounds with the desktop wallpaper",
                self.config.frosted_glass,
            ),
            Row::slider(17, "Blur size", self.config.blur, 0., 80., "px"),
        ]);
        rows.push(Row::section("Displays"));
        rows.push(Row::toggle(
            18,
            "Duplicate widgets across displays",
            "Mirror active widgets onto selected monitors",
            self.config.multi_monitor,
        ));
        if self.config.multi_monitor {
            for (i, d) in self.displays.iter().enumerate() {
                if d.primary {
                    rows.push(Row::note(format!(
                        "{} · Main Display (source)",
                        crate::displays::label(d)
                    )));
                } else if d.enabled && d.bounds.is_some() {
                    rows.push(Row::toggle(
                        2000 + i as u32,
                        crate::displays::label(d),
                        "Show duplicated widgets on this display",
                        self.config.duplicate_displays.iter().any(|id| id == &d.id),
                    ));
                } else {
                    rows.push(Row::note(format!(
                        "{} · Off / disconnected",
                        crate::displays::label(d)
                    )));
                }
            }
            if self
                .displays
                .iter()
                .filter(|d| d.enabled && d.bounds.is_some())
                .count()
                < 2
            {
                rows.push(Row::note(
                    "Connect or enable another display in Windows to duplicate widgets.",
                ));
            }
        }
        rows.extend([
            Row::section("Misc"),
            Row::toggle(
                6,
                "Start with Windows",
                "Open Lumen automatically when you sign in",
                self.config.start_with_windows,
            ),
            Row::toggle(
                15,
                "Fast startup (no OS delay)",
                "Use Task Scheduler (Admin) to launch immediately at logon; restarts app",
                self.config.fast_startup,
            ),
            Row::toggle(
                3,
                "Snap & fuse widgets",
                "Join nearby widgets; drag apart to separate",
                crate::fusion::enabled(),
            ),
            Row::toggle(
                12,
                "Low-power mode",
                "Reduce animation rate and blur effects for lower-spec PCs",
                self.config.low_power,
            ),
        ]);
        const UPDATE_ACCENT: u32 = 0xe5881e;
        rows.push(Row::section("Updates"));
        if self.update_rx.is_none() {
            rows.push(Row::button(7, "Check for updates"));
            if let Some(release) = &self.update_candidate {
                if !crate::update::pending(&self.root) {
                    rows.push(Row::accent_button(
                        8,
                        format!("Update Now · v{}", release.version),
                        UPDATE_ACCENT,
                    ));
                    rows.push(Row::button(
                        14,
                        format!("Download v{} Package", release.version),
                    ));
                }
            }
            if crate::update::pending(&self.root) {
                rows.push(Row::accent_button(
                    9,
                    "Restart and Apply Update",
                    UPDATE_ACCENT,
                ));
            }
            rows.push(Row::button(10, "Open GitHub Releases"));
        }
        if let Some(progress) = self.update_progress {
            let label = if self.update_status.is_empty() {
                "Updating Lumen…"
            } else {
                self.update_status.as_str()
            };
            rows.push(Row::progress(label, progress, UPDATE_ACCENT));
        } else if !self.update_status.is_empty() {
            if self.update_candidate.is_some()
                || crate::update::pending(&self.root)
                || self.update_rx.is_some()
            {
                rows.push(Row::highlight_note(&self.update_status, UPDATE_ACCENT));
            } else {
                rows.push(Row::note(&self.update_status));
            }
        }
        if !self.error.is_empty() {
            rows.push(Row::note(&self.error));
        }
        rows.extend([
            Row::section("About Lumen"),
            Row::note(format!(
                "Lumen {} · By Nhat Ng · MIT License",
                crate::update::installed_version(&self.root)
            )),
            Row::button(11, "Credits & licenses"),
        ]);
        rows
    }
    fn update_rows(&mut self) {
        self.ui.set_rows(self.rows());
    }
    pub unsafe fn show(&mut self) {
        self.show_near(std::ptr::null_mut());
    }
    pub unsafe fn show_near(&mut self, anchor: HWND) {
        self.displays = crate::displays::enumerate();
        self.update_rows();
        self.ui.show_near(anchor);
    }
    pub unsafe fn refresh_displays(&mut self) {
        self.displays = crate::displays::enumerate();
        self.update_rows();
        self.ui.refit();
    }
    pub unsafe fn scroll_to_updates(&mut self) {
        self.update_rows();
        self.ui.scroll_to_section("Updates");
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
    pub fn set_pinned(&mut self, pinned: Vec<String>) {
        if self.pinned == pinned {
            return;
        }
        self.pinned = pinned;
        self.update_rows();
    }
    fn apply_changes(&mut self, changes: Vec<Change>) -> Vec<Action> {
        let mut actions = vec![];
        let mut dirty = false;
        for change in changes {
            match (change.id, change.value) {
                (id, Value::Bool(open)) if (1000..2000).contains(&id) && (id - 1000) % 3 == 0 => {
                    if let Some(widget) = self.packages.get(((id - 1000) / 3) as usize) {
                        actions.push(if open {
                            Action::Show(widget.id.clone())
                        } else {
                            Action::Hide(widget.id.clone())
                        });
                    }
                }
                (id, Value::Action) if (1001..2000).contains(&id) && (id - 1001) % 3 == 0 => {
                    if let Some(widget) = self.packages.get(((id - 1001) / 3) as usize) {
                        actions.push(Action::Settings(widget.id.clone()));
                    }
                }
                (id, Value::Bool(pin)) if (1002..2000).contains(&id) && (id - 1002) % 3 == 0 => {
                    if let Some(widget) = self.packages.get(((id - 1002) / 3) as usize) {
                        if pin {
                            if !self.pinned.iter().any(|p| p == &widget.id) {
                                self.pinned.push(widget.id.clone());
                            }
                        } else {
                            self.pinned.retain(|p| p != &widget.id);
                        }
                        actions.push(Action::Pin(widget.id.clone(), pin));
                    }
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
                (12, Value::Bool(v)) => {
                    self.config.low_power = v;
                    crate::motion::set_low_power(v);
                    crate::glass::clear_cache();
                    if v {
                        crate::render::clear_caches();
                        unsafe {
                            windows_sys::Win32::System::Memory::SetProcessWorkingSetSizeEx(
                                windows_sys::Win32::System::Threading::GetCurrentProcess(),
                                usize::MAX,
                                usize::MAX,
                                0,
                            );
                        }
                    }
                    actions.push(Action::RefreshGlass);
                    dirty = true;
                }
                (13, Value::Bool(v)) => {
                    self.config.liquid_glass = v;
                    crate::glass::set_liquid_glass(v);
                    actions.push(Action::RefreshGlass);
                    dirty = true;
                }
                (15, Value::Bool(v)) => {
                    match crate::startup::set_fast_startup_elevated(v) {
                        Ok(()) => {
                            self.config.fast_startup = v;
                            dirty = true;
                            self.error.clear();
                        }
                        Err(error) => self.error = error,
                    }
                }
                (16, Value::Bool(v)) => {
                    self.config.frosted_glass = v;
                    crate::glass::set_frosted_glass(v);
                    actions.push(Action::RefreshGlass);
                    dirty = true;
                }
                (17, Value::Number(v)) => {
                    self.config.blur = v.clamp(0., 80.);
                    crate::glass::set_global_blur(self.config.blur);
                    actions.push(Action::RefreshGlass);
                    dirty = true;
                }
                (18, Value::Bool(v)) => {
                    self.config.multi_monitor = v;
                    unsafe {
                        self.displays = crate::displays::enumerate();
                        crate::multi_monitor::configure(v, self.config.duplicate_displays.clone());
                    }
                    actions.push(Action::SyncMultiMonitor);
                    dirty = true;
                }
                (id, Value::Bool(v)) if id >= 2000 => {
                    if let Some(d) = self.displays.get((id - 2000) as usize) {
                        if v {
                            if !self.config.duplicate_displays.iter().any(|x| x == &d.id) {
                                self.config.duplicate_displays.push(d.id.clone());
                            }
                        } else {
                            self.config.duplicate_displays.retain(|x| x != &d.id);
                        }
                        unsafe {
                            crate::multi_monitor::configure(
                                self.config.multi_monitor,
                                self.config.duplicate_displays.clone(),
                            );
                        }
                        actions.push(Action::SyncMultiMonitor);
                        dirty = true;
                    }
                }
                (7, Value::Action) => self.start_update_check(),
                (8, Value::Action) => actions.extend(self.trigger_update_now()),
                (9, Value::Action) if self.update_rx.is_none() && !self.smoke => {
                    actions.push(Action::RestartUpdate)
                }
                (10, Value::Action) if !self.smoke => unsafe {
                    crate::update::open_releases();
                },
                (14, Value::Action) if !self.smoke => unsafe {
                    let url = self
                        .update_candidate
                        .as_ref()
                        .map(|release| release.url.as_str())
                        .unwrap_or("");
                    crate::update::open_download(url);
                },
                (11, Value::Action) if !self.smoke => unsafe {
                    let file = self.root.join("LICENSES/CREDITS.txt");
                    if file.is_file() {
                        let result = windows_sys::Win32::UI::Shell::ShellExecuteW(
                            std::ptr::null_mut(),
                            wide("open").as_ptr(),
                            wide(&file).as_ptr(),
                            std::ptr::null(),
                            std::ptr::null(),
                            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
                        );
                        if result as isize <= 32 {
                            self.error = "Windows could not open the credits text file.".into();
                        }
                    } else {
                        self.error="Credits collection is missing. Rebuild the package or run scripts/third_party.py.".into();
                    }
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
    pub fn is_updating(&self) -> bool {
        self.update_rx.is_some()
    }
    pub fn announce_applied_update(&mut self, version: &str) {
        self.update_progress = None;
        self.update_status = format!("Updated to Lumen v{version} successfully!");
        self.update_rows();
    }
    pub fn has_update(&self) -> bool {
        self.update_candidate.is_some() || crate::update::pending(&self.root)
    }
    pub fn start_update_check(&mut self) {
        if self.update_rx.is_some() || self.smoke {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let root = self.root.clone();
        self.update_rx = Some(rx);
        self.update_candidate = None;
        self.update_progress = None;
        self.auto_apply_update = false;
        self.update_status = "Checking GitHub…".into();
        std::thread::spawn(move || {
            let _ = tx.send(UpdateMessage::Done(crate::update::check(&root)));
        });
        self.update_rows();
    }
    pub fn trigger_update_now(&mut self) -> Vec<Action> {
        if self.update_rx.is_some() || self.smoke {
            return Vec::new();
        }
        if crate::update::pending(&self.root) {
            return vec![Action::RestartUpdate];
        }
        if let Some(candidate) = self.update_candidate.clone() {
            let (tx, rx) = std::sync::mpsc::channel();
            let root = self.root.clone();
            self.update_rx = Some(rx);
            self.auto_apply_update = true;
            self.update_progress = Some(0.01);
            self.update_status = format!("Downloading Lumen v{}…", candidate.version);
            std::thread::spawn(move || {
                let mut last_pct = 0u32;
                let res = crate::update::stage_with_progress(&root, &candidate, &mut |p| {
                    let pct = (p.fraction * 100.).round() as u32;
                    if pct != last_pct || p.fraction >= 1.0 || p.fraction <= 0.03 {
                        last_pct = pct;
                        let _ = tx.send(UpdateMessage::Progress(p));
                    }
                })
                .map(|_| None);
                let _ = tx.send(UpdateMessage::Done(res));
            });
            self.update_rows();
        }
        Vec::new()
    }
    pub fn poll_update(&mut self) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut changed = false;
        loop {
            let result = self.update_rx.as_ref().map(|rx| rx.try_recv());
            match result {
                Some(Ok(UpdateMessage::Progress(p))) => {
                    self.update_status = p.label;
                    self.update_progress = Some(p.fraction);
                    changed = true;
                }
                Some(Ok(UpdateMessage::Done(value))) => {
                    self.update_rx = None;
                    self.update_progress = None;
                    match value {
                        Ok(Some(release)) => {
                            self.auto_apply_update = false;
                            self.update_status =
                                format!("New version Lumen {} is available!", release.version);
                            self.update_candidate = Some(release);
                        }
                        Ok(None) => {
                            self.update_candidate = None;
                            if crate::update::pending(&self.root) {
                                if self.auto_apply_update {
                                    self.auto_apply_update = false;
                                    self.update_status =
                                        "Update extracted. Restarting Lumen to apply…".into();
                                    actions.push(Action::RestartUpdate);
                                } else {
                                    self.update_status =
                                        "Verified update ready. Restart to apply.".into();
                                }
                            } else {
                                self.auto_apply_update = false;
                                self.update_status =
                                    "No newer compatible stable release is published.".into();
                            }
                        }
                        Err(error) => {
                            self.auto_apply_update = false;
                            self.update_status = error;
                        }
                    }
                    changed = true;
                    break;
                }
                Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => {
                    self.update_rx = None;
                    self.update_progress = None;
                    self.auto_apply_update = false;
                    self.update_status = "Update worker stopped. Try again.".into();
                    changed = true;
                    break;
                }
                _ => break,
            }
        }
        if changed {
            self.update_rows();
        }
        actions
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn update_controls_follow_worker_state_without_starting_network_in_tests() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = crate::render::Runtime::start().unwrap();
        let root = std::env::temp_dir().join(format!("lumen-update-ui-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        unsafe {
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            let mut host = Host::create(std::ptr::null_mut(), &root, glass, true).unwrap();
            let has = |rows: &Vec<Row>, wanted| {
                rows.iter()
                    .any(|row| matches!(row,Row::Button{id,..} if *id==wanted))
            };
            assert!(has(&host.rows(), 7));
            assert!(!has(&host.rows(), 8));
            assert!(has(&host.rows(), 11));
            assert!(!has(&host.rows(), 4) && !has(&host.rows(), 5));
            assert!(!host
                .rows()
                .iter()
                .any(|r| matches!(r, Row::Slider { id: 1, .. })));
            assert!(host
                .rows()
                .iter()
                .any(|row| matches!(row,Row::Note(text) if text.contains("Nhat Ng"))));
            let (tx, rx) = std::sync::mpsc::channel();
            host.update_rx = Some(rx);
            assert!(!has(&host.rows(), 7));
            tx.send(UpdateMessage::Progress(crate::update::Progress {
                label: "Downloading Lumen v0.2.0 (1.0 / 2.0 MB)…".into(),
                fraction: 0.5,
            }))
            .unwrap();
            assert!(host.poll_update().is_empty());
            assert!(host.rows().iter().any(|row| matches!(
                row,
                Row::Progress {
                    label,
                    fraction,
                    ..
                } if label.contains("0.2.0") && (*fraction - 0.5).abs() < 1e-4
            )));
            tx.send(UpdateMessage::Done(Ok(Some(crate::update::Release {
                version: "0.2.0".into(),
                url: String::new(),
                digest: String::new(),
                size: 0,
            }))))
            .unwrap();
            assert!(host.poll_update().is_empty());
            assert!(host.update_status.contains("0.2.0"));
            assert!(host
                .rows()
                .iter()
                .any(|row| matches!(row, Row::HighlightNote { text, .. } if text.contains("0.2.0"))));
            assert!(has(&host.rows(), 7));
            assert!(has(&host.rows(), 8));
            assert!(has(&host.rows(), 14));
            std::fs::write(root.join("lumen-portable.json"), "{}").unwrap();
            assert!(has(&host.rows(), 8));
            assert!(has(&host.rows(), 14));
            let (tx, rx) = std::sync::mpsc::channel();
            host.update_rx = Some(rx);
            tx.send(UpdateMessage::Done(Err("offline".into()))).unwrap();
            assert!(host.poll_update().is_empty());
            assert_eq!(host.update_status, "offline");
            assert!(has(&host.rows(), 7));
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
                    id: 1001 + i as u32 * 3,
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
                    id: 1000 + i as u32 * 3,
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
            let pins = registry
                .ids()
                .into_iter()
                .enumerate()
                .skip(1)
                .map(|(i, _)| Change {
                    id: 1002 + i as u32 * 3,
                    value: Value::Bool(i % 2 == 1),
                })
                .collect();
            let expected_pins = registry
                .ids()
                .into_iter()
                .enumerate()
                .skip(1)
                .map(|(i, id)| Action::Pin(id, i % 2 == 1))
                .collect::<Vec<_>>();
            assert_eq!(host.apply_changes(pins), expected_pins);
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
            assert_eq!(actions, Vec::<Action>::new());
            assert!(!crate::fusion::enabled());
            host.scroll_to_updates();
            let updates_scroll = host.ui.scroll_offset();
            assert!(
                updates_scroll > 0.,
                "expected scroll_to_updates to scroll down to Updates section, got {updates_scroll}"
            );
            host.hide();
            drop(host);
            crate::fusion::shutdown();
        }
        assert!(!root.join("config/native-general.json").exists());
    }
    #[test]
    fn duplicate_display_toggles_across_all_mod3_ids_update_config_and_sync() {
        let _lock = crate::native_ui_test_lock();
        let _runtime = crate::render::Runtime::start().unwrap();
        let root =
            std::env::temp_dir().join(format!("lumen-display-toggles-{}", std::process::id()));
        let make_display = |index: usize, primary: bool| crate::displays::DisplayInfo {
            id: format!("display-{index}"),
            source: format!("\\\\.\\DISPLAY{}", index + 1),
            monitor_path: format!("path-{index}"),
            name: format!("Monitor {index}"),
            enabled: true,
            primary,
            bounds: Some([index as i32 * 1920, 0, index as i32 * 1920 + 1920, 1080]),
            work: Some([index as i32 * 1920, 0, index as i32 * 1920 + 1920, 1040]),
        };
        unsafe {
            let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
            let mut host = Host::create(std::ptr::null_mut(), &root, glass, true).unwrap();
            host.displays = vec![
                make_display(0, false),
                make_display(1, false),
                make_display(2, false),
                make_display(3, false),
            ];
            let enable_changes = (0..4)
                .map(|i| Change {
                    id: 2000 + i as u32,
                    value: Value::Bool(true),
                })
                .collect();
            assert_eq!(
                host.apply_changes(enable_changes),
                vec![Action::SyncMultiMonitor; 4]
            );
            assert_eq!(
                host.config.duplicate_displays,
                vec![
                    "display-0".to_string(),
                    "display-1".to_string(),
                    "display-2".to_string(),
                    "display-3".to_string(),
                ]
            );
            let disable_changes = (0..4)
                .map(|i| Change {
                    id: 2000 + i as u32,
                    value: Value::Bool(false),
                })
                .collect();
            assert_eq!(
                host.apply_changes(disable_changes),
                vec![Action::SyncMultiMonitor; 4]
            );
            assert!(host.config.duplicate_displays.is_empty());
            host.hide();
            drop(host);
            crate::multi_monitor::configure(false, vec![]);
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
