use crate::widget_packages::{NativeDefinition, WidgetHost};
impl WidgetHost for super::Host {
    unsafe fn show(&mut self) { super::Host::show(self); }
    unsafe fn hide(&mut self) { super::Host::hide(self); }
    unsafe fn settings(&mut self) { self.show_settings(); }
    unsafe fn visible(&self) -> bool { self.is_visible() }
    fn hwnd(&self) -> windows_sys::Win32::Foundation::HWND { self.hwnd() }
    unsafe fn apply_common(&mut self, opacity: Option<f32>, floating: Option<bool>) { self.apply_common(opacity, floating); }
    unsafe fn refresh(&mut self) { windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW(self.hwnd(), windows_sys::Win32::UI::WindowsAndMessaging::WM_SETTINGCHANGE, 0, 0); }
    unsafe fn tick(&mut self) {  }
}
pub(crate) fn definition() -> NativeDefinition {
    NativeDefinition {
        id: "calendar",
        factory: |root, glass, inspect, smoke| unsafe {
            super::Host::create(root, glass, inspect, smoke).map(|h| Box::new(h) as Box<dyn WidgetHost>)
        },
        should_open: super::should_open,
    }
}
