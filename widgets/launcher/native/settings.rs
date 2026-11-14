//! App Dock settings, using the same sections and control styling as Lumen.
use super::*;
#[derive(Clone, Copy)]
pub(super) enum Action {
    Frosted,
    Tint,
    Strength,
    Blur,
    Opacity,
    Palette,
    Theme(usize),
    Less,
    More,
    Icon,
    Labels,
    Search,
    Desktop,
    PinWidget(usize),
    Quit,
    Close,
}
#[derive(Clone, Copy)]
pub(super) struct Hit {
    pub rect: [i32; 4],
    pub action: Action,
}
pub(super) struct State {
    pub frame: Option<crate::compositor::Buffer>,
    pub hwnd: HWND,
    pub dpi: f32,
    pub scroll: i32,
    pub smooth_scroll: crate::motion::SmoothScroll,
    pub scroll_clock: Option<crate::motion::FrameClock>,
    pub frame_queued: bool,
    pub expanded: bool,
    pub hits: Vec<Hit>,
    pub dragging: Option<Action>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            frame: None,
            hwnd: null_mut(),
            dpi: 1.0,
            scroll: 0,
            smooth_scroll: Default::default(),
            scroll_clock: None,
            frame_queued: false,
            expanded: false,
            hits: vec![],
            dragging: None,
        }
    }
}
pub(super) const THEMES: [(&str, &str, u32); 12] = crate::surface::PRESETS;
const PICK_SURFACE_COLOR: u32 = WM_APP + 220;
impl App {
    pub(super) fn selection_color(&self) -> u32 {
        if self.dock.theme == crate::accent::THEME {
            self.accent_color
        } else {
            crate::accent::vivid(self.theme_color())
        }
    }
    pub(super) fn theme_color(&self) -> u32 {
        crate::surface::color(&self.dock.theme, self.accent_color)
    }
    pub(super) unsafe fn set_accent(&mut self, color: u32) {
        if self.accent_color == color {
            return;
        }
        self.accent_color = color;
        if self.dock.theme == crate::accent::THEME {
            self.dock.light = crate::accent::light(color);
            self.background_key.clear();
            self.overlay = None;
            self.rebuild_style(self.hwnd);
        }
        if !self.settings.hwnd.is_null() {
            InvalidateRect(self.settings.hwnd, null(), 0);
        }
    }
    pub(super) unsafe fn open_settings(&mut self, owner: HWND) {
        let accent = self.glass.borrow().wallpaper_accent();
        self.set_accent(accent);
        if !self.settings.hwnd.is_null() {
            ShowWindow(self.settings.hwnd, SW_SHOW);
            SetForegroundWindow(self.settings.hwnd);
            return;
        }
        let class = wide("LumenNativeDockSettings");
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(settings_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..zeroed()
        };
        RegisterClassW(&wc);
        let mut r: RECT = zeroed();
        GetWindowRect(owner, &mut r);
        let scale = GetDpiForWindow(owner) as f32 / 96.;
        let w = (568. * scale) as i32;
        let h = (668. * scale) as i32;
        let mut monitor: MONITORINFO = zeroed();
        monitor.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(
            MonitorFromWindow(owner, MONITOR_DEFAULTTONEAREST),
            &mut monitor,
        );
        let x = (r.left + (r.right - r.left - w) / 2).clamp(
            monitor.rcWork.left,
            (monitor.rcWork.right - w).max(monitor.rcWork.left),
        );
        let y = (r.top - 80).clamp(
            monitor.rcWork.top,
            (monitor.rcWork.bottom - h).max(monitor.rcWork.top),
        );
        let pointer = GetWindowLongPtrW(owner, GWLP_USERDATA);
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("App Dock settings").as_ptr(),
            WS_POPUP,
            x,
            y,
            w,
            h,
            owner,
            null_mut(),
            instance,
            pointer as _,
        );
        if hwnd.is_null() {
            self.status = "Could not open App Dock settings".into();
            return;
        }
        self.settings.hwnd = hwnd;
        self.settings.dpi = GetDpiForWindow(hwnd) as f32 / 96.;
        round_window(hwnd, self.settings.dpi, 30);
        let none = 0xfffffffeu32;
        DwmSetWindowAttribute(hwnd, 34, &none as *const _ as _, 4);
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        InvalidateRect(hwnd, null(), 0);
        self.paint_settings(hwnd);
    }
    pub(super) unsafe fn settings_changed(&mut self, layout: bool, blur: bool) {
        if blur {
            self.glass.borrow_mut().set_blur(self.dock.blur, self.dpi);
        }
        if layout {
            // Columns/icon size reflow content inside the user's existing
            // bounds. Only an explicit edge drag changes the window size.
            self.layout_edit(self.hwnd);
            let (w, h) = self.dimensions(self.hwnd);
            self.dock.size = Some([w, h]);
        }
        ShowWindow(
            self.edit,
            if self.search_visible() && self.group_selection.is_none() {
                SW_SHOW
            } else {
                SW_HIDE
            },
        );
        self.rebuild_style(self.hwnd);
        InvalidateRect(self.hwnd, null(), 0);
        InvalidateRect(self.settings.hwnd, null(), 0);
    }
    unsafe fn paint_settings(&mut self, hwnd: HWND) {
        let frame_started = std::time::Instant::now();
        let mut ps: PAINTSTRUCT = zeroed();
        BeginPaint(hwnd, &mut ps);
        let mut rect: RECT = zeroed();
        GetClientRect(hwnd, &mut rect);
        let mut frame = self
            .settings
            .frame
            .take()
            .filter(|f| {
                f.width == rect.right && f.height == rect.bottom && f.dpi == self.settings.dpi
            })
            .or_else(|| {
                crate::compositor::Buffer::new(rect.right, rect.bottom, self.settings.dpi, 30.)
            })
            .expect("settings paint buffer");
        let dc = frame.dc;
        let bg = rgb(30, 32, 36);
        let b = CreateSolidBrush(bg);
        FillRect(dc, &rect, b);
        DeleteObject(b);
        let scale = self.settings.dpi;
        let w = (rect.right as f32 / scale) as i32;
        let h = (rect.bottom as f32 / scale) as i32;
        let max_scroll = (804 + if self.settings.expanded { 162 } else { 0 } - h + 18).max(0);
        let now = self.now();
        if self.settings.scroll_clock.is_some() {
            self.settings.smooth_scroll.bounds(max_scroll as f32, now);
            self.settings.scroll = self.settings.smooth_scroll.value(now).round() as i32;
            if !self.settings.smooth_scroll.moving(now) {
                KillTimer(hwnd, 1);
                self.settings.scroll_clock = None;
            }
        } else {
            self.settings.scroll = self.settings.scroll.clamp(0, max_scroll);
            self.settings.smooth_scroll.max = max_scroll as f32;
            self.settings
                .smooth_scroll
                .snap(self.settings.scroll as f32);
        }
        let c = Canvas::new(dc, scale);
        let white = rgb(241, 242, 248);
        let dim = rgb(172, 188, 199);
        c.round(
            [0., 0., w as f32, h as f32],
            30.,
            rgb(30, 32, 36),
            255,
            false,
            false,
        );
        c.round(
            [0.5, 0.5, w as f32 - 0.5, h as f32 - 0.5],
            30.,
            0xffffff,
            22,
            true,
            false,
        );
        drop(c);
        let c = Canvas::new(dc, scale);
        c.round([17., 16., 45., 44.], 14., white, 14, false, false);
        c.round([17., 16., 45., 44.], 14., white, 24, true, false);
        c.line(&[(33., 23.), (27., 30.), (33., 37.)], white, 200, 1.3);
        c.text(
            "App Dock settings",
            [54., 14., w as f32 - 58., 46.],
            13.,
            true,
            white,
            0,
        );
        c.line(
            &[(w as f32 - 37., 25.), (w as f32 - 29., 33.)],
            dim,
            210,
            1.2,
        );
        c.line(
            &[(w as f32 - 29., 25.), (w as f32 - 37., 33.)],
            dim,
            210,
            1.2,
        );
        drop(c);
        let saved = SaveDC(dc);
        IntersectClipRect(
            dc,
            0,
            (58. * scale) as i32,
            rect.right,
            rect.bottom - (12. * scale) as i32,
        );
        let c = Canvas::new(dc, scale);
        let scroll = self.settings.scroll;
        let mut hits = vec![
            Hit {
                rect: [16, 14, 46, 46],
                action: Action::Close,
            },
            Hit {
                rect: [w - 46, 14, w - 16, 46],
                action: Action::Close,
            },
        ];
        let tx = |text: &str, r: [i32; 4], size: f32, bold: bool, color: u32, align: i32| {
            c.text(
                text,
                [
                    r[0] as f32,
                    (r[1] - scroll) as f32,
                    r[2] as f32,
                    (r[3] - scroll) as f32,
                ],
                size,
                bold,
                color,
                align,
            )
        };
        let round = |r: [i32; 4], radius: f32, alpha: u8, outline: bool| {
            c.round(
                [
                    r[0] as f32,
                    (r[1] - scroll) as f32,
                    r[2] as f32,
                    (r[3] - scroll) as f32,
                ],
                radius,
                white,
                alpha,
                outline,
                false,
            )
        };
        let line = |y: i32| {
            c.line(
                &[
                    (16., (y - scroll) as f32),
                    ((w - 16) as f32, (y - scroll) as f32),
                ],
                white,
                20,
                0.5,
            )
        };
        let mut hit = |r: [i32; 4], action| {
            let r = [r[0], r[1] - scroll, r[2], r[3] - scroll];
            if r[3] > 58 && r[1] < h - 12 {
                hits.push(Hit { rect: r, action });
            }
        };
        let toggle = |y: i32, on: bool| {
            let r = [w - 61, y + 10, w - 27, y + 30];
            round(r, 10., if on { 58 } else { 20 }, false);
            round(r, 10., 60, true);
            let x = if on { w - 37 } else { w - 51 };
            c.round(
                [
                    x as f32 - 7.,
                    (y + 13 - scroll) as f32,
                    x as f32 + 7.,
                    (y + 27 - scroll) as f32,
                ],
                7.,
                white,
                255,
                false,
                false,
            );
        };
        tx(
            "Changes here apply only to App Dock.",
            [24, 62, w - 24, 87],
            11.,
            false,
            dim,
            0,
        );
        tx(
            "B A C K G R O U N D",
            [25, 94, w - 25, 109],
            9.,
            true,
            dim,
            0,
        );
        let extra = if self.settings.expanded { 162 } else { 0 };
        round([16, 112, w - 16, 462 + extra], 16., 10, false);
        round([16, 112, w - 16, 462 + extra], 16., 35, true);
        tx("Frosted glass", [28, 116, w - 80, 137], 12., true, white, 0);
        tx(
            "Blend this widget’s design with the wallpaper",
            [28, 136, w - 80, 151],
            9.5,
            false,
            dim,
            0,
        );
        toggle(115, self.dock.frosted);
        hit([w - 76, 114, w - 20, 154], Action::Frosted);
        line(154);
        for (label, value, max, y, action, suffix) in [
            ("Glass tint", self.dock.tint, 100., 154, Action::Tint, "%"),
            (
                "Frost strength",
                self.dock.frost_strength,
                100.,
                209,
                Action::Strength,
                "%",
            ),
            ("Blur size", self.dock.blur, 80., 264, Action::Blur, "px"),
            (
                "Background opacity",
                self.dock.opacity,
                100.,
                319,
                Action::Opacity,
                "%",
            ),
        ] {
            tx(label, [28, y + 6, w - 80, y + 28], 11.5, false, white, 0);
            tx(
                &format!("{}{suffix}", value.round()),
                [w - 82, y + 6, w - 28, y + 28],
                10.,
                false,
                dim,
                2,
            );
            let end = w - 64;
            let fill = 28. + (end - 28) as f32 * value / max;
            c.round(
                [
                    28.,
                    (y + 35 - scroll) as f32,
                    end as f32,
                    (y + 39 - scroll) as f32,
                ],
                2.,
                dim,
                65,
                false,
                false,
            );
            c.round(
                [
                    28.,
                    (y + 35 - scroll) as f32,
                    fill.max(28.1),
                    (y + 39 - scroll) as f32,
                ],
                2.,
                white,
                190,
                false,
                false,
            );
            c.round(
                [
                    fill - 6.5,
                    y as f32 + 30.5 - scroll as f32,
                    fill + 6.5,
                    (y as f32 + 43.5 - scroll as f32),
                ],
                6.5,
                white,
                255,
                false,
                false,
            );
            tx(
                &format!("{}", value.round()),
                [w - 59, y + 24, w - 26, y + 49],
                12.,
                false,
                white,
                2,
            );
            hit([22, y + 25, w - 60, y + 52], action);
        }
        tx(
            "Background opacity lets desktop windows show through.",
            [32, 382, w - 30, 399],
            9.,
            false,
            dim,
            0,
        );
        tx(
            "Search, groups, icons and labels keep their own opacity.",
            [32, 397, w - 30, 414],
            9.,
            false,
            dim,
            0,
        );
        line(425);
        tx(
            if self.settings.expanded {
                "⌄ Surface palette"
            } else {
                "› Surface palette"
            },
            [30, 431, w - 30, 458],
            12.,
            false,
            dim,
            0,
        );
        hit([20, 425, w - 20, 462], Action::Palette);
        if self.settings.expanded {
            let cell = (w - 64) / 3;
            for (i, theme) in THEMES.iter().enumerate() {
                let x = 28 + (i % 3) as i32 * cell;
                let width = cell - 8;
                let color = if theme.0 == crate::accent::THEME {
                    self.accent_color
                } else if theme.0 == "custom" {
                    self.theme_color()
                } else {
                    theme.2
                };
                let chip_ink = if crate::contrast::luminance(color) > 0.42 {
                    rgb(24, 30, 39)
                } else {
                    rgb(248, 250, 253)
                };
                let y = 470 + (i / 3) as i32 * 36;
                let chip = [
                    x as f32,
                    (y - scroll) as f32,
                    (x + width) as f32,
                    (y + 29 - scroll) as f32,
                ];
                if crate::surface::is_gradient(theme.0) {
                    crate::surface::swatch(&c, chip, 9., theme.0, scale);
                } else {
                    c.round(chip, 9., color, 255, false, false);
                }
                if theme.0 == self.dock.theme
                    || theme.0 == "custom"
                        && crate::surface::custom_color(&self.dock.theme).is_some()
                {
                    c.round(
                        [
                            x as f32,
                            (y - scroll) as f32,
                            (x + width) as f32,
                            (y + 29 - scroll) as f32,
                        ],
                        9.,
                        chip_ink,
                        180,
                        true,
                        false,
                    );
                }
                tx(
                    theme.1,
                    [x + 8, y, x + width - 8, y + 29],
                    10.,
                    false,
                    chip_ink,
                    0,
                );
                hit([x, y, x + width, y + 29], Action::Theme(i));
            }
        }
        let start = 494 + extra;
        tx(
            "A P P  L A Y O U T",
            [25, start - 20, w - 25, start - 4],
            9.,
            true,
            dim,
            0,
        );
        round([16, start, w - 16, start + 260], 16., 10, false);
        round([16, start, w - 16, start + 260], 16., 35, true);
        tx(
            "Columns",
            [28, start + 3, w - 140, start + 26],
            11.5,
            false,
            white,
            0,
        );
        tx(
            "Used by the Lumen grid",
            [28, start + 23, w - 140, start + 38],
            9.5,
            false,
            dim,
            0,
        );
        round([w - 112, start + 8, w - 28, start + 34], 13., 16, false);
        round([w - 112, start + 8, w - 28, start + 34], 13., 25, true);
        tx(
            "−",
            [w - 110, start + 6, w - 84, start + 36],
            15.,
            false,
            dim,
            1,
        );
        tx(
            &self.dock.columns.to_string(),
            [w - 84, start + 6, w - 57, start + 36],
            11.5,
            true,
            white,
            1,
        );
        tx(
            "+",
            [w - 57, start + 6, w - 28, start + 36],
            15.,
            false,
            dim,
            1,
        );
        hit([w - 114, start, w - 84, start + 40], Action::Less);
        hit([w - 58, start, w - 22, start + 40], Action::More);
        line(start + 42);
        tx(
            "Icon size",
            [28, start + 48, w - 80, start + 70],
            11.5,
            false,
            white,
            0,
        );
        tx(
            &format!("{}px", self.dock.icon_size),
            [w - 82, start + 48, w - 28, start + 70],
            10.,
            false,
            dim,
            2,
        );
        let fill = 28. + (w - 92) as f32 * (self.dock.icon_size - 36) as f32 / 24.;
        c.round(
            [
                28.,
                (start + 80 - scroll) as f32,
                (w - 64) as f32,
                (start + 84 - scroll) as f32,
            ],
            2.,
            dim,
            65,
            false,
            false,
        );
        c.round(
            [
                28.,
                (start + 80 - scroll) as f32,
                fill.max(28.1),
                (start + 84 - scroll) as f32,
            ],
            2.,
            white,
            190,
            false,
            false,
        );
        c.round(
            [
                fill - 6.5,
                (start as f32 + 75.5 - scroll as f32),
                fill + 6.5,
                (start as f32 + 88.5 - scroll as f32),
            ],
            6.5,
            white,
            255,
            false,
            false,
        );
        tx(
            &self.dock.icon_size.to_string(),
            [w - 59, start + 69, w - 26, start + 94],
            12.,
            false,
            white,
            2,
        );
        hit([22, start + 68, w - 60, start + 99], Action::Icon);
        line(start + 100);
        for (label, on, y, action) in [
            ("Sync Desktop items", self.dock.sync_desktop, start + 180, Action::Desktop),
            (
                "Show labels",
                self.dock.show_labels,
                start + 100,
                Action::Labels,
            ),
            (
                "Show search",
                self.dock.show_search,
                start + 140,
                Action::Search,
            ),
        ] {
            tx(label, [28, y, w - 80, y + 40], 11.5, false, white, 0);
            toggle(y, on);
            hit([w - 76, y, w - 20, y + 40], action);
            line(y + 40);
        }
        let mut footer_y = start + 236;
        let manifests: Vec<(String, String)> = self
            .packages
            .manifests()
            .iter()
            .map(|m| (m.id.clone(), m.name.clone()))
            .collect();
        if !manifests.is_empty() {
            tx("TOOLBAR WIDGETS", [26, footer_y, w - 26, footer_y + 18], 9.5, true, dim, 0);
            footer_y += 24;
            for (i, (id, name)) in manifests.iter().enumerate() {
                let y = footer_y + i as i32 * 40;
                let on = self.dock.pinned_widgets.iter().any(|p| p == id);
                self.symbols.draw(
                    dc,
                    self.dpi,
                    crate::symbols::widget_symbol(id),
                    28.,
                    y as f32 + 12.,
                    16.,
                    white,
                );
                tx(name, [52, y, w - 80, y + 40], 11.5, false, white, 0);
                toggle(y, on);
                hit([w - 76, y, w - 20, y + 40], Action::PinWidget(i));
                line(y + 40);
            }
            footer_y += manifests.len() as i32 * 40 + 24;
        }
        tx(
            "Lumen · Native dock",
            [27, footer_y, w - 110, footer_y + 27],
            10.,
            false,
            dim,
            0,
        );
        tx(
            "Quit",
            [w - 75, footer_y, w - 27, footer_y + 27],
            11.,
            false,
            white,
            2,
        );
        hit([w - 82, footer_y - 5, w - 22, footer_y + 32], Action::Quit);
        drop(c);
        RestoreDC(dc, saved);
        self.settings.hits = hits;
        let content = footer_y + 40;
        if content > h - 14 {
            let track = (h - 90).max(30) as f32;
            let thumb = (track * (h - 60) as f32 / content as f32).max(35.);
            let max = (content - h + 18).max(1) as f32;
            let y = 70. + (track - thumb) * scroll as f32 / max;
            frame.fill_round([w as f32 - 8., y, w as f32 - 5., y + thumb], 1.5, white, 70);
        }
        frame.present(hwnd);
        self.settings.frame = Some(frame);
        EndPaint(hwnd, &ps);
        if IsWindowVisible(hwnd) != 0 && self.settings.scroll_clock.is_some() {
            crate::motion::pace_vsync(
                hwnd,
                frame_started,
                &mut self.settings.frame_queued,
                crate::motion::VSYNC_FRAME,
            );
        }
    }
    unsafe fn settings_action(&mut self, action: Action, x: i32) {
        let w = client_dips(self.settings.hwnd, self.settings.dpi).0;
        let t = ((x - 28) as f32 / (w - 92).max(1) as f32).clamp(0., 1.);
        let mut layout = false;
        match action {
            Action::Close => {
                ShowWindow(self.settings.hwnd, SW_HIDE);
                self.settings.frame = None;
                return;
            }
            Action::Frosted => self.dock.frosted = !self.dock.frosted,
            Action::Tint => self.dock.tint = (t * 100.).round(),
            Action::Strength => self.dock.frost_strength = (t * 100.).round(),
            Action::Blur => self.dock.blur = (t * 80.).round(),
            Action::Opacity => self.dock.opacity = (t * 100.).round(),
            Action::Palette => self.settings.expanded = !self.settings.expanded,
            Action::Theme(i) => {
                if THEMES[i].0 == "custom" {
                    PostMessageW(
                        self.settings.hwnd,
                        PICK_SURFACE_COLOR,
                        0,
                        self.theme_color() as isize,
                    );
                    return;
                }
                self.dock.theme = THEMES[i].0.into();
                self.dock.light = crate::accent::light(self.theme_color());
            }
            Action::Less => {
                self.dock.columns = self.dock.columns.saturating_sub(1).max(3);
                layout = true;
            }
            Action::More => {
                self.dock.columns = (self.dock.columns + 1).min(12);
                layout = true;
            }
            Action::Icon => {
                self.dock.icon_size = 36 + (t * 24.).round() as i32;
                layout = true;
            }
            Action::Labels => self.dock.show_labels = !self.dock.show_labels,
            Action::Search => self.dock.show_search = !self.dock.show_search,
            Action::Desktop => {
                self.dock.sync_desktop = !self.dock.sync_desktop;
                self.sync_desktop_items(self.hwnd);
            }
            Action::PinWidget(index) => {
                if let Some(id) = self.packages.manifests().get(index).map(|m| m.id.clone()) {
                    if let Some(pos) = self.dock.pinned_widgets.iter().position(|p| p == &id) {
                        self.dock.pinned_widgets.remove(pos);
                    } else {
                        self.dock.pinned_widgets.push(id);
                    }
                    self.layout_edit(self.hwnd);
                    self.refresh_manager();
                    self.save();
                    layout = true;
                }
            }
            Action::Quit => {
                PostMessageW(self.hwnd, WM_CLOSE, 0, 0);
                return;
            }
        }
        self.settings_changed(layout, matches!(action, Action::Blur));
    }
}
unsafe fn client_dips(hwnd: HWND, scale: f32) -> (i32, i32) {
    let mut r: RECT = zeroed();
    GetClientRect(hwnd, &mut r);
    (
        (r.right as f32 / scale) as i32,
        (r.bottom as f32 / scale) as i32,
    )
}
unsafe extern "system" fn settings_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let cs = &*(lparam as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
    }
    if message == WM_NCCALCSIZE || message == WM_NCPAINT {
        return 0;
    }
    if message == WM_NCACTIVATE || message == WM_ERASEBKGND {
        return 1;
    }
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
    if pointer.is_null() {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    if message == PICK_SURFACE_COLOR {
        if let Some(color) = crate::color_picker::choose(hwnd, lparam as u32) {
            if IsWindow(hwnd) == 0 || GetWindowLongPtrW(hwnd, GWLP_USERDATA) != pointer as isize {
                return 0;
            }
            if let Ok(mut app) = (*pointer).try_borrow_mut() {
                app.dock.theme = crate::surface::custom_key(color);
                app.dock.light = crate::accent::light(color);
                app.background_key.clear();
                app.overlay = None;
                let root = app.hwnd;
                app.rebuild_style(root);
                app.save();
                InvalidateRect(root, null(), 0);
                InvalidateRect(hwnd, null(), 0);
            }
        }
        return 0;
    }
    let Ok(mut app) = (*pointer).try_borrow_mut() else {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    };
    match message {
        WM_PAINT => {
            app.paint_settings(hwnd);
            0
        }
        WM_NCHITTEST => {
            let (x, y) = xy(lparam);
            let mut r: RECT = zeroed();
            GetWindowRect(hwnd, &mut r);
            let scale = app.settings.dpi;
            let edge = crate::geometry::edge(
                x - r.left,
                y - r.top,
                r.right - r.left,
                r.bottom - r.top,
                (10. * scale) as i32,
                (40. * scale) as i32,
            );
            if edge != HTCLIENT {
                return edge as isize;
            }
            if y - r.top < (14. * scale) as i32
                || (y - r.top < (52. * scale) as i32
                    && x - r.left > (48. * scale) as i32
                    && x < r.right - (50. * scale) as i32)
            {
                HTCAPTION as isize
            } else {
                HTCLIENT as isize
            }
        }
        WM_NCLBUTTONDOWN if (HTLEFT..=HTBOTTOMRIGHT).contains(&(wparam as u32)) => {
            app.start_resize(hwnd, wparam as u32, lparam);
            0
        }
        WM_SIZE => {
            round_window(hwnd, app.settings.dpi, 30);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_MOVE => {
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_DPICHANGED => {
            app.settings.dpi = (wparam as u16) as f32 / 96.;
            let r = &*(lparam as *const RECT);
            SetWindowPos(
                hwnd,
                null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            round_window(hwnd, app.settings.dpi, 30);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_LBUTTONDOWN => {
            let (x, y) = xy(lparam);
            let x = (x as f32 / app.settings.dpi) as i32;
            let y = (y as f32 / app.settings.dpi) as i32;
            let action = app
                .settings
                .hits
                .iter()
                .find(|hit| {
                    x >= hit.rect[0] && x < hit.rect[2] && y >= hit.rect[1] && y < hit.rect[3]
                })
                .map(|h| h.action);
            if let Some(a) = action {
                if matches!(
                    a,
                    Action::Tint | Action::Strength | Action::Blur | Action::Opacity | Action::Icon
                ) {
                    app.settings.dragging = Some(a);
                    SetCapture(hwnd);
                }
                app.settings_action(a, x);
                if app.settings.dragging.is_none() {
                    app.save();
                }
            }
            0
        }
        WM_MOUSEMOVE => {
            if app.resize_drag.is_some() {
                app.move_resize(hwnd, lparam);
                return 0;
            }
            if let Some(action) = app.settings.dragging {
                let (x, _) = xy(lparam);
                let x = (x as f32 / app.settings.dpi) as i32;
                app.settings_action(action, x);
            }
            0
        }
        WM_LBUTTONUP => {
            if app.resize_drag.is_some() {
                app.finish_resize(hwnd);
            }
            if app.settings.dragging.take().is_some() {
                ReleaseCapture();
                app.settings_changed(false, true);
                app.save();
            }
            0
        }
        WM_CAPTURECHANGED => {
            app.resize_drag = None;
            0
        }
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) as u16 as i16) as f32 / 120.;
            let (_, h) = client_dips(hwnd, app.settings.dpi);
            let content = 844 + if app.settings.expanded { 162 } else { 0 };
            let max = (content - h + 18).max(0) as f32;
            let now = app.now();
            app.settings.smooth_scroll.bounds(max, now);
            if app.settings.scroll_clock.is_none() {
                let cur = app.settings.scroll as f32;
                app.settings.smooth_scroll.snap(cur);
                app.settings.scroll_clock = Some(crate::motion::FrameClock::start());
            }
            app.settings.smooth_scroll.by(-delta * 72., now);
            app.settings.scroll = app.settings.smooth_scroll.value(now).round() as i32;
            SetTimer(hwnd, 1, 10, None);
            InvalidateRect(hwnd, null(), 0);
            0
        }
        WM_TIMER if wparam == 1 => {
            let now = app.now();
            app.settings.scroll = app.settings.smooth_scroll.value(now).round() as i32;
            if !app.settings.frame_queued {
                InvalidateRect(hwnd, null(), 0);
            }
            if !app.settings.smooth_scroll.moving(now) {
                KillTimer(hwnd, 1);
                app.settings.scroll_clock = None;
            }
            0
        }
        m if m == crate::motion::VSYNC_FRAME => {
            app.settings.frame_queued = false;
            let now = app.now();
            app.settings.scroll = app.settings.smooth_scroll.value(now).round() as i32;
            if app.settings.smooth_scroll.moving(now) {
                InvalidateRect(hwnd, null(), 0);
            } else {
                KillTimer(hwnd, 1);
                app.settings.scroll_clock = None;
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_KEYDOWN if wparam == 0x1b => {
            ShowWindow(hwnd, SW_HIDE);
            app.settings.frame = None;
            0
        }
        WM_CLOSE => {
            ShowWindow(hwnd, SW_HIDE);
            app.settings.frame = None;
            0
        }
        WM_DESTROY => {
            app.settings.hwnd = null_mut();
            0
        }
        _ => {
            drop(app);
            DefWindowProcW(hwnd, message, wparam, lparam)
        }
    }
}
