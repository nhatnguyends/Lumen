# Shared App Dock settings UI — stable integration API (2026-11-10)

Owner: Calendar/settings kit chat. `src/settings_ui.rs` reproduces the existing
`src/settings.rs` shell/control metrics and drawing; do not copy it per widget.
Root adds `mod settings_ui;` once. No Dock settings/shared host edits by this chat.

```rust
use crate::settings_ui::{Host, Row, Choice, Value, Change};
// IDs are widget-local u32 values. Choose a widget-local WM_APP message.
let rows = vec![
    Row::section("Background"),
    Row::toggle(1, "Frosted glass", "Blend this widget with the wallpaper", true),
    Row::slider(2, "Glass tint", 18.0, 0.0, 100.0, "%"),
    Row::choices(3, "Theme", "lumen", vec![
        Choice::new("lumen", "Lumen", Some(0x554336)),
        Choice::new("paper", "Paper", Some(0xe8e4df)),
    ]),
];
let host = Host::create(owner_hwnd, "Calendar settings",
    "Changes here apply only to Calendar.", glass_rc, rows, WM_APP + 84)?;
```

- `unsafe Host::create(owner: HWND, title: &str, subtitle: &str,
  glass: Rc<RefCell<Glass>>, rows: Vec<Row>, notify_message: u32) -> Result<Host,String>`
- unsafe `show(&mut self)`, `hide(&mut self)`; `hwnd(&self) -> HWND`;
  `is_visible(&self) -> bool`; `set_rows(&mut self, Vec<Row>)`;
  `take_changes(&mut self) -> Vec<Change>`; `set_surface(&mut self, color:u32, light:bool)`.
- `Row::section(label)`; `toggle(id,label,hint,bool)`;
  `slider(id,label,value,min,max,suffix)`;
  `choices(id,label,selected_key,Vec<Choice>)`; `button(id,label)`; `note(text)`.
- `Choice::new(key,label,Option<COLORREF>)`; public owned strings.
- `Change { pub id:u32, pub value:Value }`;
  `Value::{Bool(bool),Number(f32),Choice(String),Action}`.
- On `notify_message` in the widget's wndproc, drain `take_changes()`, validate and
  apply to that widget config, save once, refresh its rendering and set_rows with
  current values if needed. The kit posts asynchronously; no borrowed widget
  state is called synchronously. Slider updates are coalesced per ID until drain.
- Keep one `Option<Host>` per widget (inside UI-thread state). Gear, tray and
  manager `show_settings()` open that same separate window. Never replace widget
  contents. Owner teardown drops settings first. Hide/Escape releases surfaces;
  reopen reuses HWND; opening does not change owner's bounds or config.
- The kit uses `LumenNativeWidgetSettings` class, not a fusion member class. It
  provides header drag, edge resize (420×360 DIP minimum), scroll, Tab/Shift-Tab,
  Enter/Space, arrow adjustment of sliders, close/Escape, DPI and owned z-order.
- Shared Glass is read-only in the kit: it never calls set_blur. Independent
  widget blur requires that widget's local Glass instance/variant, not mutation
  of the shared dock Glass. Settings schema/value state remains widget-local.
- UI constants mirror Dock: 568×668 DIP initial window, 30-DIP shell radius,
  13-DIP bold title at x54/y14, back/close at y14, 16-DIP section cards/rims,
  12-DIP row headings, 34×20 toggle/14 thumb, 4-DIP sliders/13-DIP thumb,
  3-column palette cards with 9-DIP radius, shared renderer/fonts/shadow.

API above is implemented and stable. Peers may integrate without editing this
file. The kit and General adapter passed their targeted native regressions;
root subsequently verified the shared suite serially (117 passed/4 ignored).
Native test launches are now paused in peer chats; root owns final desktop QA.

## Shared widget chrome header (user requirement 2026-11-10)

Also exported by `settings_ui` (no extra root module):

- `HeaderAction::{Settings,Hide}` (Clone/Copy/PartialEq/Eq).
- `widget_header_hits(width:f32) -> [(HeaderAction,[f32;4]);2]` uses 32×32 DIP
  hit areas at [w-84,12,w-52,44] and [w-48,12,w-16,44].
- unsafe `draw_widget_header(dc:HDC,dpi:f32,width:f32,title:&str,ink:u32,
  hover:Option<HeaderAction>,pressed:Option<HeaderAction>)`.
- Shared Segoe UI Variable Text Semibold (Canvas::text_medium) 12 DIP, title-case
  caller title, x20 inset, row y8..48 (40 DIP high). Settings uses dock's outline
  SVG gear **5.svg** at20 DIP; hide is a matching outline X. Shared hover/press material.
  Constants WIDGET_HEADER_TOP=8 and WIDGET_HEADER_HEIGHT=40 define those bounds;
  the 32-DIP buttons sit within that same row (y12..44).
- Callers use the returned stable hit rectangles; activate only on mouse release
  in the same button. Helper only draws; it does not own input or window state.
- Use for "Music", "Sticky Notes", "Calendar", "Clock" chrome only; keep theme
  content typography independent. Dock keeps its search header with no new title.
- A widget may reveal this row on hover if its legacy layout needs that behavior.
  Calendar keeps it visible to preserve settings access at minimum size.

Implementation details for lifecycle: cached header data contains plain pixels,
not render::Image/GDI+ handles. Each native Image is scoped to a draw so renderer
shutdown and test-thread TLS destruction cannot leave stale native handles.
Additive API: `Host::refresh()` invalidates backing after wallpaper changes;
`#[cfg(test)] unsafe render_for_test()` renders hidden owned buffers for tests.


## Header reveal update — 2026-11-10

`draw_widget_header` now takes the owning `HWND` immediately after `dc`.
All four widget adapters pass `self.hwnd`. The shared `widget_header` module
attaches an app-local subclass lazily to that widget, animates presence180/240ms
and Dock hover/rotation, blocks invisible-header clicks, and cleans up on hide/
destruction. Native child transitions preserve widget presence. No settings,
editor or Dock toolbar HWND is registered here. Title remains visible. Icons
are now16 DIP (matching Dock), within the original32-DIP hit rectangles. Pixel
cache still owns no persistent GDI+ image handles.


## Responsive title typography — latest 2026-11-10

The shared header now draws its title in Segoe UI Light16 DIP, with the same
40-DIP row/inset. It hides the title at width<320 DIP or height<260 DIP (logical
size, independent of device scaling), without changing or scaling the controls.
`draw_widget_header` obtains real height from the supplied owner HWND; no adapter
signature change. `widget_title_visible(width,height)` is the shared breakpoint.
Settings/close reveal remains independent; only widget chrome titles are changed.


## Latest compact chrome decision — supersedes Light16 title

Shared title is12 DIP Bold; visible only at width>=420 and height>=360 DIP.
`widget_content_top(width,height)` returns54 DIP for large panels,14 DIP for
compact ones. Hide the row's empty space, not just its text. Music chrome label
is `YouTube Music`. Keep the original hover controls and their32-DIP hit boxes;
compact content should reserve only that top-right area where needed. Root
updated Notes, Calendar layout, Music/coverflow and Clock compact/emblem geometry.
