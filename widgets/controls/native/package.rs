use crate::widget_packages::{NativeDefinition, WidgetHost};
impl WidgetHost for super::Host {
    unsafe fn show(&mut self) {
        self.show();
    }
    unsafe fn hide(&mut self) {
        self.hide();
    }
    unsafe fn settings(&mut self) {
        self.show_settings();
    }
    unsafe fn visible(&self) -> bool {
        self.is_visible()
    }
    fn hwnd(&self) -> windows_sys::Win32::Foundation::HWND {
        self.hwnd()
    }
    unsafe fn apply_common(&mut self, o: Option<f32>, f: Option<bool>) {
        self.apply_common(o, f);
    }
    unsafe fn refresh(&mut self) {
        self.refresh();
    }
}
pub(crate) fn definition() -> NativeDefinition {
    NativeDefinition {
        id: "controls",
        factory: |r, g, i, s| unsafe {
            super::Host::create(r, g, i, s).map(|h| Box::new(h) as Box<dyn WidgetHost>)
        },
        should_open: super::should_open,
    }
}
