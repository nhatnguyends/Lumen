#![windows_subsystem = "windows"]

mod accent;
mod catalog;
mod compositor;
mod color_picker;
mod contrast;
mod fusion;
mod general;
mod geometry;
mod glass;
mod model;
mod motion;
include!(concat!(env!("OUT_DIR"), "/widget_modules.rs"));
mod widget_packages;
mod text_prompt;
mod render;
mod sampling;
mod settings_ui;
mod symbols;
mod surface;
mod tray;
mod update;
mod startup;
mod widget_header;

// Win32 z-order/activation are process/desktop-wide, whereas production uses
// one UI thread. Keep native HWND tests from changing that state concurrently;
// pure geometry/model/pixel tests can still run in parallel.
#[cfg(test)]
pub(crate) fn native_ui_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn main() {
    if update::dispatch() { return; }
    if let Err(error) = window::run() {
        eprintln!("{error}");
        if std::env::args().any(|arg|arg=="--smoke-test") {std::process::exit(1);}
        // Null owner is valid; the two UTF-16 buffers live until the dialog closes.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                std::ptr::null_mut(),
                model::wide(&error).as_ptr(),
                model::wide("Lumen native dock").as_ptr(),
                windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                    | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
