use super::*;
use crate::settings_ui::{Change, Value};
unsafe fn rect(hwnd: HWND) -> [i32; 4] {
    let mut r = zeroed();
    GetWindowRect(hwnd, &mut r);
    [r.left, r.top, r.right, r.bottom]
}
unsafe fn save_pixels(width: u32, height: u32, mut bytes: Vec<u8>, name: &str) {
    if let Some(folder) = std::env::var_os("LUMEN_CLOCK_PREVIEW_DIR") {
        let folder = PathBuf::from(folder);
        std::fs::create_dir_all(&folder).unwrap();
        for p in bytes.chunks_exact_mut(4) {
            p.swap(0, 2);
            let a = p[3] as u32;
            if a > 0 {
                for v in &mut p[..3] {
                    *v = ((*v as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        image::RgbaImage::from_raw(width, height, bytes)
            .unwrap()
            .save(folder.join(format!("{name}.png")))
            .unwrap();
    }
}
unsafe fn export(s: &State, name: &str) {
    let b = s.frame.as_ref().unwrap();
    save_pixels(b.width as u32, b.height as u32, b.snapshot(), name);
}
unsafe fn fixture(name: &str) -> (Host, Rc<RefCell<Glass>>) {
    let root = std::env::temp_dir().join(format!("lumen-clock-{name}-{}", std::process::id()));
    let glass = Rc::new(RefCell::new(Glass::new(13., 1.)));
    (
        Host::create(&root, glass.clone(), true, true).unwrap(),
        glass,
    )
}

#[test]
fn detached_settings_are_singleton_keep_owner_unchanged_and_close_independently() {
    let _ui = crate::native_ui_test_lock();
    unsafe {
        let _graphics = render::Runtime::start().unwrap();
        let (mut host, glass) = fixture("settings");
        glass.borrow_mut().refresh();
        host.show();
        let hwnd = host.hwnd();
        let bounds = rect(hwnd);
        let original = serde_json::to_value(&host.state.borrow().config).unwrap();
        let pixels = host.state.borrow().frame.as_ref().unwrap().snapshot();
        let revision = glass.borrow().revision();
        let sample = Buffer::plain(180, 180, 1.).unwrap();
        glass.borrow().draw(sample.dc, hwnd, 180, 180);
        let source_pixels = sample.snapshot();
        host.show_settings();
        let settings = host.state.borrow().settings.as_ref().unwrap().hwnd();
        assert_ne!(settings, hwnd);
        assert_ne!(IsWindowVisible(settings), 0);
        assert_eq!(rect(hwnd), bounds);
        assert!(host.is_visible());
        assert_eq!(
            serde_json::to_value(&host.state.borrow().config).unwrap(),
            original
        );
        assert_eq!(
            host.state.borrow().frame.as_ref().unwrap().snapshot(),
            pixels
        );
        {
            let mut s = host.state.borrow_mut();
            let kit = s.settings.as_mut().unwrap();
            let (w, h, p) = kit.render_for_test();
            save_pixels(w, h, p, "settings-detached");
        }
        host.show_settings();
        assert_eq!(
            host.state.borrow().settings.as_ref().unwrap().hwnd(),
            settings
        );
        // The detached shell has its own minimum and scrolling; neither
        // resizes the owner nor replaces its face.
        let dpi = GetDpiForWindow(settings) as f32 / 96.;
        SetWindowPos(
            settings,
            null_mut(),
            0,
            0,
            (420. * dpi) as i32,
            (360. * dpi) as i32,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let mut limits: MINMAXINFO = zeroed();
        SendMessageW(settings, WM_GETMINMAXINFO, 0, &mut limits as *mut _ as _);
        assert_eq!(limits.ptMinTrackSize.x, (420. * dpi) as i32);
        assert_eq!(limits.ptMinTrackSize.y, (360. * dpi) as i32);
        {
            let mut s = host.state.borrow_mut();
            let (w, h, p) = s.settings.as_mut().unwrap().render_for_test();
            save_pixels(w, h, p, "settings-detached-min");
        }
        SendMessageW(settings, WM_KEYDOWN, 0x22, 0);
        SendMessageW(settings, WM_KEYDOWN, 0x22, 0);
        {
            let mut s = host.state.borrow_mut();
            let (w, h, p) = s.settings.as_mut().unwrap().render_for_test();
            save_pixels(w, h, p, "settings-detached-scrolled");
        }
        assert_eq!(rect(hwnd), bounds);
        assert_eq!(
            serde_json::to_value(&host.state.borrow().config).unwrap(),
            original
        );
        let mut class = [0u16; 100];
        let len = GetClassNameW(settings, class.as_mut_ptr(), 100);
        assert_eq!(
            String::from_utf16_lossy(&class[..len as usize]),
            "LumenNativeWidgetSettings"
        );
        SendMessageW(settings, WM_KEYDOWN, 0x1B, 0);
        assert_eq!(IsWindowVisible(settings), 0);
        assert!(host.is_visible());
        assert_eq!(rect(hwnd), bounds);
        host.show_settings();
        assert_eq!(
            host.state.borrow().settings.as_ref().unwrap().hwnd(),
            settings
        );
        assert_ne!(IsWindowVisible(settings), 0);
        SendMessageW(settings, WM_CLOSE, 0, 0);
        assert_eq!(IsWindowVisible(settings), 0);
        assert!(host.is_visible());
        {
            let mut s = host.state.borrow_mut();
            assert!(s.apply_setting(Change {
                id: 4,
                value: Value::Number(29.)
            }));
            assert!(s.apply_setting(Change {
                id: 3,
                value: Value::Number(45.)
            }));
            assert!(s.apply_setting(Change {
                id: 2,
                value: Value::Number(22.)
            }));
            s.key.clear();
            s.paint();
            assert_eq!(s.config.blur, 29.);
            assert_eq!(s.config.frost_strength, 45.);
            assert_eq!(
                s.config.size,
                original["size"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_i64().unwrap() as i32)
                    .collect::<Vec<_>>()
                    .as_slice()
            );
        }
        assert_eq!(glass.borrow().revision(), revision);
        glass.borrow().draw(sample.dc, hwnd, 180, 180);
        assert_eq!(sample.snapshot(), source_pixels);
        assert_eq!(rect(hwnd), bounds);
        let theme = host.state.borrow().config.theme.clone();
        host.apply_common(Some(35.), Some(true));
        assert_eq!(host.state.borrow().config.opacity, 35.);
        assert_eq!(host.state.borrow().config.theme, theme);
        assert_eq!(rect(hwnd), bounds);
        SendMessageW(hwnd, WM_TIMER, TICK, 0);
        UpdateWindow(hwnd);
        assert!(host.is_visible());
        assert!(host.state.borrow().frame.is_some());
        host.hide();
        assert!(!host.is_visible());
        host.show_settings();
        assert!(!host.is_visible());
        assert_eq!(host.state.borrow().config.open, false);
        assert!(host.state.borrow().frame.is_none());
        let (settings_owner, settings_hwnd) = (
            host.hwnd(),
            host.state.borrow().settings.as_ref().unwrap().hwnd(),
        );
        drop(host);
        assert_eq!(IsWindow(settings_owner), 0);
        assert_eq!(IsWindow(settings_hwnd), 0);
    }
}
#[test]
fn all_faces_render_at_minimum_and_mixed_dpi_without_losing_content_or_hit_targets() {
    let _ui = crate::native_ui_test_lock();
    unsafe {
        let _graphics = render::Runtime::start().unwrap();
        let (mut host, _) = fixture("compact");
        let hit_dpi = host.state.borrow().dpi;
        crate::geometry::assert_widget_corner_hits(host.hwnd(), hit_dpi);
        host.show();
        for dpi in [1., 1.25, 1.5, 2.] {
            for (width, height) in [
                (120, 120),
                (128, 128),
                (120, 320),
                (320, 120),
                (160, 160),
                (160, 320),
                (320, 160),
                (200, 200),
                (240, 240),
                (279, 279),
                (280, 280),
                (380, 380),
            ] {
                {
                    let mut s = host.state.borrow_mut();
                    s.dpi = dpi;
                    s.frost = None;
                    s.frost_key.clear();
                }
                let (pw, ph) = (
                    (width as f32 * dpi).round() as i32,
                    (height as f32 * dpi).round() as i32,
                );
                SetWindowPos(
                    host.hwnd(),
                    null_mut(),
                    0,
                    0,
                    pw,
                    ph,
                    SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOZORDER,
                );
                for (id, _) in THEMES {
                    for twelve in [false, true] {
                        let mut s = host.state.borrow_mut();
                        s.config.theme = id.into();
                        s.config.hour12 = twelve;
                        s.key.clear();
                        s.paint();
                        assert_eq!(rect(s.hwnd)[2] - rect(s.hwnd)[0], pw);
                        assert_eq!(rect(s.hwnd)[3] - rect(s.hwnd)[1], ph);
                        assert_eq!(
                            s.frame.as_ref().map(|b| [b.width, b.height]),
                            Some([pw, ph])
                        );
                        let bytes = s.frame.as_ref().unwrap().snapshot();
                        assert!(bytes.chunks_exact(4).any(|p| p[3] == 255));
                        assert!(bytes
                            .chunks_exact(4)
                            .all(|p| p[..3].iter().all(|v| *v <= p[3])));
                        if width < 160 || height < 160 {
                            // Exclude header controls: the tiny primary face itself
                            // must leave visible pixels below the reserved header.
                            let background = s.background.as_ref().unwrap().snapshot();
                            let left = (12. * dpi) as usize;
                            let right = ((width as f32 - 12.) * dpi) as usize;
                            let top = (44. * dpi) as usize;
                            let bottom = ((height as f32 - 12.) * dpi) as usize;
                            let changed = (top..bottom)
                                .flat_map(|y| (left..right).map(move |x| (y * pw as usize + x) * 4))
                                .filter(|i| bytes[*i..*i + 3] != background[*i..*i + 3])
                                .count();
                            assert!(
                                changed > 20,
                                "missing tiny primary face: {id} {width}x{height} dpi={dpi}"
                            );
                        }
                        for b in s.buttons() {
                            assert!(
                                b.r[0] >= 0. && b.r[2] <= width as f32 && b.r[3] <= height as f32
                            );
                            assert!(b.r[2] - b.r[0] >= 32. && b.r[3] - b.r[1] >= 32.);
                            assert_eq!(
                                s.hit((b.r[0] + b.r[2]) / 2., (b.r[1] + b.r[3]) / 2.),
                                Some(b.id)
                            );
                        }
                        if dpi == 1. {
                            export(
                                &s,
                                &format!(
                                    "{id}-{width}x{height}-{}",
                                    if twelve { "12h" } else { "24h" }
                                ),
                            );
                        }
                    }
                }
                let mut info: MINMAXINFO = zeroed();
                SendMessageW(host.hwnd(), WM_GETMINMAXINFO, 0, &mut info as *mut _ as _);
                assert_eq!(info.ptMinTrackSize.x, (MIN_SIZE as f32 * dpi).ceil() as i32);
                assert_eq!(info.ptMinTrackSize.y, (MIN_SIZE as f32 * dpi).ceil() as i32);
            }
        }
        {
            let mut s = host.state.borrow_mut();
            s.config.opacity = 0.;
            s.key.clear();
            s.paint();
            let pixels = s.frame.as_ref().unwrap().snapshot();
            assert!(pixels.chunks_exact(4).any(|p| p[3] == 1));
            assert!(pixels.chunks_exact(4).any(|p| p[3] > 240));
        }
        host.hide();
        {
            let s = host.state.borrow();
            assert!(
                s.frame.is_none()
                    && s.background.is_none()
                    && s.shadow.is_none()
                    && s.frost.is_none()
            );
        }
        host.apply_common(Some(25.), Some(true));
        assert!(!host.is_visible());
        assert!(host.state.borrow().frame.is_none());
        host.show();
        assert!(host.is_visible());
    }
}
#[test]
fn first_corner_mask_matches_a_fresh_repaint_without_resize() {
    let _ui = crate::native_ui_test_lock();
    unsafe {
        let _runtime = render::Runtime::start().unwrap();
        let (host, _) = fixture("first-corner-mask");
        let dpi = host.state.borrow().dpi;
        SetWindowPos(
            host.hwnd(),
            null_mut(),
            0,
            0,
            (324. * dpi) as i32,
            (291. * dpi) as i32,
            SWP_NOMOVE | SWP_NOACTIVATE | SWP_NOZORDER,
        );
        let mut s = host.state.borrow_mut();
        s.config.theme = "lumen".into();
        s.config.palette = "theme".into();
        s.key.clear();
        s.paint();
        let first = s.frame.as_ref().unwrap().snapshot();
        export(&s, "corner-first-lumen");
        s.frame = None;
        s.background = None;
        s.key.clear();
        s.paint();
        let fresh = s.frame.as_ref().unwrap().snapshot();
        export(&s, "corner-fresh-lumen");
        assert!(
            first
                .chunks_exact(4)
                .zip(fresh.chunks_exact(4))
                .all(|(a, b)| a[3] == b[3]),
            "startup alpha cut must equal a fresh render without resizing"
        );
    }
}
#[test]
fn settings_adapter_rejects_invalid_values_and_preserves_geometry_theme_and_shared_source() {
    let _ui = crate::native_ui_test_lock();
    unsafe {
        let _graphics = render::Runtime::start().unwrap();
        let (host, glass) = fixture("schema");
        let mut s = host.state.borrow_mut();
        let geometry = (s.config.position, s.config.size);
        let rev = glass.borrow().revision();
        assert!(!s.apply_setting(Change {
            id: 4,
            value: Value::Number(f32::NAN)
        }));
        assert!(!s.apply_setting(Change {
            id: 10,
            value: Value::Choice("missing".into())
        }));
        assert!(!s.apply_setting(Change {
            id: 11,
            value: Value::Choice("Nowhere/Invalid".into())
        }));
        assert!(s.apply_setting(Change {
            id: 10,
            value: Value::Choice("neon".into())
        }));
        assert!(s.apply_setting(Change {
            id: 11,
            value: Value::Choice("UTC".into())
        }));
        assert!(s.apply_setting(Change {
            id: 4,
            value: Value::Number(999.)
        }));
        assert_eq!(s.config.blur, 80.);
        for palette in ["gradient:dawn", "custom:#345678", "ocean", "theme"] {
            assert!(s.apply_setting(Change {
                id: 6,
                value: Value::Choice(palette.into())
            }));
            assert_eq!(
                s.config.palette,
                if palette == "theme" {
                    "theme".into()
                } else {
                    crate::surface::normalize(palette).unwrap()
                }
            );
        }
        assert!(!s.apply_setting(Change {
            id: 6,
            value: Value::Choice("custom".into())
        }));
        assert_eq!((s.config.position, s.config.size), geometry);
        assert_eq!(glass.borrow().revision(), rev);
        let rows = s.settings_rows();
        for id in [2, 3, 4, 5] {
            assert!(rows
                .iter()
                .any(|r| matches!(r,crate::settings_ui::Row::Slider{id:key,..}if *key==id)));
        }
        assert!(!rows.iter().any(|r|matches!(r,crate::settings_ui::Row::Toggle{id:14,..})));
        for id in [1, 12, 13] {
            assert!(rows
                .iter()
                .any(|r| matches!(r,crate::settings_ui::Row::Toggle{id:key,..}if *key==id)));
        }
    }
}

#[test]
fn selected_surface_rebuilds_cached_backdrop_and_reuses_it_between_ticks() {
    let _ui = crate::native_ui_test_lock();
    unsafe {
        let _graphics = render::Runtime::start().unwrap();
        let (mut host, _) = fixture("surface-cache");
        host.show();
        let mut s = host.state.borrow_mut();
        s.config.frosted_glass = false;
        let mut previous = String::new();
        let mut previous_pixels = Vec::new();
        for palette in ["gradient:dawn", "custom:#345678", "theme"] {
            assert!(s.apply_setting(Change {
                id: 6,
                value: Value::Choice(palette.into())
            }));
            s.paint();
            let key = s.key.clone();
            let pixels = s.background.as_ref().unwrap().snapshot();
            assert_ne!(key, previous);
            if !previous_pixels.is_empty() {
                assert_ne!(pixels, previous_pixels);
            }
            s.paint();
            assert_eq!(s.key, key);
            assert_eq!(s.background.as_ref().unwrap().snapshot(), pixels);
            previous = key;
            previous_pixels = pixels;
        }
    }
}

#[test]
fn afterglow_reference_layout_keeps_location_time_period_date_and_footer_grouped() {
    let _ui = crate::native_ui_test_lock();
    unsafe {
        let _graphics = render::Runtime::start().unwrap();
        let (mut host, _) = fixture("afterglow-reference");
        {
            let mut s = host.state.borrow_mut();
            s.config.theme = "sunset".into();
            s.config.hour12 = true;
            s.config.show_seconds = false;
            s.config.time_zone = "Asia/Ho_Chi_Minh".into();
            s.location_label = model::location_label(&s.config.time_zone);
        }
        host.show();
        let time = SYSTEMTIME {
            wYear: 2026,
            wMonth: 11,
            wDay: 10,
            wDayOfWeek: 2,
            wHour: 10,
            wMinute: 41,
            ..zeroed()
        };
        for (w, h) in [(420, 420), (380, 380), (160, 160), (480, 300), (160, 320)] {
            SetWindowPos(
                host.hwnd(),
                null_mut(),
                0,
                0,
                w,
                h,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let mut s = host.state.borrow_mut();
            s.dpi = 1.;
            s.key.clear();
            s.paint();
            let mut frame = Buffer::new(w, h, 1., 34.).unwrap();
            frame.copy_from(s.background.as_ref().unwrap().dc);
            let batch = render::AlphaBatch::new(frame.dc, 1.);
            frame.background_alpha(255);
            s.afterglow_content = None;
            s.paint_content_at(frame.dc, w as f32, h as f32, &time);
            face::draw_location(
                &Canvas::new(frame.dc, 1.),
                w as f32,
                h as f32,
                &s.location_label,
                s.ink,
            );
            drop(batch);
            frame.present_with_alpha(s.hwnd, true);
            let p = frame.snapshot();
            assert!(p.chunks_exact(4).all(|p| p[..3].iter().all(|v| *v <= p[3])));
            save_pixels(
                w as u32,
                h as u32,
                p,
                &format!("afterglow-reference-{w}x{h}"),
            );
            let old = s.afterglow_content.as_ref().unwrap().1.identity();
            let next = SYSTEMTIME {
                wMilliseconds: 640,
                ..time
            };
            s.paint_content_at(frame.dc, w as f32, h as f32, &next);
            assert_eq!(s.afterglow_content.as_ref().unwrap().1.identity(), old);
        }
    }
}
