#![windows_subsystem = "windows"]

mod accent;
mod catalog;
mod color_picker;
mod compositor;
mod contrast;
mod desktop_layer;
mod desktop_sync;
mod fusion;
mod general;
mod geometry;
mod glass;
mod model;
mod motion;
include!(concat!(env!("OUT_DIR"), "/widget_modules.rs"));
mod render;
mod sampling;
mod settings_ui;
mod startup;
mod surface;
mod symbols;
mod text_prompt;
mod tray;
mod update;
mod widget_header;
mod widget_packages;

// Win32 z-order/activation are process/desktop-wide, whereas production uses
// one UI thread. Keep native HWND tests from changing that state concurrently;
// pure geometry/model/pixel tests can still run in parallel.
#[cfg(test)]
pub(crate) fn native_ui_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn main() {
    if update::dispatch() {
        return;
    }
    if let Err(error) = window::run() {
        eprintln!("{error}");
        if std::env::args().any(|arg| arg == "--smoke-test") {
            std::process::exit(1);
        }
        // Null owner is valid; the two UTF-16 buffers live until the dialog closes.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                model::wide(&error).as_ptr(),
                model::wide("Lumen").as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
