# General Settings integration contract

**2026-11-10 update:** General now delegates its shell/controls to settings_ui.
Public Host methods, Action enum and ACTION_MESSAGE stay compatible with root
wiring. See HANDOFF-settings-ui.md for current details and the native test crash
report/test freeze. The former custom General renderer is removed.

`src/general.rs` is ready for root wiring. I have not changed main/window/tray for
this module, per coordinator instruction. It has a native glass UI with Widgets
and General tabs. All widget actions are typed and queued to the owning dock;
the module never opens a placeholder or writes another widget's private config.

## Root wiring required

1. Add `mod general;` in main.rs. Add `general: Option<crate::general::Host>` in
   App, initialize None, and take/drop it before the App/graphics runtime dies.
2. Add General Settings / Manage widgets to tray (and status-bar command if one
   exists). Root's show_general should lazily construct:

   `general::Host::create(dock_hwnd, &root, glass.clone(), smoke)`.

   After `fusion::init`, call `general::load_preferences(&root)` once to restore
   Snap & fuse preference even when the General window is created lazily. Errors
   should be displayed/logged without replacing the invalid configuration.

   The normal root class already owns process DPI awareness. General is an owned
   foreground tool window, not a desktop widget; do not register it for fusion.
3. Before `host.show()`, call
   `host.set_visibility([dock_visible, music_visible, notes_visible, calendar_visible, clock_visible])`.
   Query actual HWND visibility, not persisted open flags.
4. In wndproc, handle `crate::general::ACTION_MESSAGE` (WM_APP + 81) by draining
   `host.take_actions()`. Process actions after releasing any borrowed General
   state (take_actions returns owned Vec). Root dispatch:

   - Show(widget): existing root show/lazy-create action for the named widget.
   - Hide(widget): native Host hide method; for Dock use SW_HIDE without quitting.
   - Settings(widget): create/show the widget, then open its actual Settings.
   - ShowAll / HideAll: apply to the five widget surfaces; keep General open so
     Hide all can be reversed immediately.
   - Opacity(percent): apply only background opacity to all widgets, including
     persisted configs of lazily-created ones. Do not fade foreground text.
   - Floating(bool): apply actual TOPMOST/non-TOPMOST to all widget windows and
     persist their flags. Keep settings dialogs usable above their owner.
5. Refresh the visibility array after each applied lifecycle action. Do not
   optimistically toggle the array before success. Call report_error(&str) on
   action failure so the dialog displays it.
6. `host.refresh()` invalidates glass after a shared wallpaper change. There is
   no polling/animation timer in General. Hidden window surfaces are released.

General's persisted config `config/native-general.json` records the last explicit
all-widget opacity/floating choice and dialog position. **Loading it does not
automatically overwrite per-widget settings**. Root decides whether and how a
newly-created widget should inherit a previous all-widget choice. No autostart
registration or system settings changes are introduced.

## Public API

- `Host::create(owner: HWND, root: &Path, glass: Rc<RefCell<Glass>>, smoke: bool)`
- unsafe `show()`, `hide()`, `refresh()`; `hwnd()`
- `set_visibility([bool;5])`, `take_actions() -> Vec<Action>`, `report_error(&str)`
- `load_preferences(&Path) -> Result<(), String>` restores fusion preference.
- `Widget::{Dock, Music, Notes, Calendar, Clock}` and matching `WIDGETS` order
- `Action::{Show(Widget), Hide(Widget), Settings(Widget), ShowAll, HideAll,
  Opacity(f32), Floating(bool)}`; `ACTION_MESSAGE = WM_APP + 81`

Calendar now exposes `hwnd()`, unsafe `is_visible()`, `hide()`, `show_settings()`,
and `apply_common(opacity: Option<f32>, floating: Option<bool>)`. Missing fields
are preserved. Showing calendar settings does not resize the user's widget.
Hide kills date/hover timers and releases frame/backdrop/shadow; show recreates
them and refreshes the current date.

## Validation performed

`python .scratch/check_calendar_general.py` compiled the actual general.rs plus
existing renderer/glass/compositor in an isolated temporary Cargo harness, with
all build output under native-dock/target/debug. It does not edit shared main.rs.
Three tests passed: config persistence/corrupt-file preservation, widget order/
hit boundaries, and hidden native render/action dispatch/coalescing/hide cleanup.
The action test also initializes the real fusion manager and verifies the toggle
changes enabled state both ways. General tabs were rendered from hidden owned
buffers and visually inspected at `.scratch/general-native-settings.png` using
`python .scratch/check_calendar_general.py -- --ignored`.
No desktop input, visible preview, or runtime restart was used for those tests.

The harness deliberately excludes other widgets while fusion is being wired;
root should run normal cargo check/test after adding mod general. Root must
complete end-to-end tray dispatch and settings UI verification after wiring.

## Known limits

- Widget visibility/status is root-supplied and must be refreshed to stay live.
- General uses a fixed 560 × 570 DIP layout with per-monitor DPI handling; it
  moves by the title and does not expose a resizable edge.
- Native custom-painted controls support pointer input, Tab/Shift-Tab focus,
  Enter/Space activation, Left/Right on opacity and Escape. Per-control UI
  Automation remains an accessibility improvement.
- General stays opaque enough to remain usable when all widget backing is set
  to 0%; that slider controls widgets, not its own dialog.
- All-widget mutation must be wired by root; the UI alone does not apply it.

Snap & fuse is a working direct toggle via `fusion::enabled/set_enabled`, default
on. It persists `snapFuse` in the General config and applies when loaded. It does
not emit a queued Action; root only needs the initial load after fusion::init.

Last isolated run passed 3 tests with the preview ignored. Compiler also reported
a separate fusion test warning around `fusion/mod.rs`'s temporary
`wide(class).as_ptr()` inside a WNDCLASS initializer (line 175 at that snapshot).
Bind the UTF-16 Vec before RegisterClassW; this chat did not modify fusion code.

Reentrancy audit after root wiring: all default WndProc dispatches release the
General state guard first; explicit WM_WINDOWPOSCHANGED follows the shadow and
drops its guard before DefWindowProcW can synchronously deliver WM_SIZE. Drag,
DPI and Escape SetWindowPos calls also run without borrowed state. WM_SIZE clears
the backdrop key and invalidates. The hidden General regression issues native
SetWindowPos and asserts the nested size handler ran; it passed in the normal
shared crate (`cargo test ... general::tests::hidden_host_emits_real_widget_actions_and_releases_surfaces`).
Calendar's corresponding native resize regression also passed; source guard passed.
