# Native widget source packages

For the proposed future GitHub package distribution and separate app/widget
updates, see [PACKAGE-DISTRIBUTION.md](../PACKAGE-DISTRIBUTION.md). That design
is not implemented yet; the source-build discovery described here remains current.

Lumen's native manager discovers `widgets/*/widget.json`. The six current packages
are App Dock (`launcher`), Clock, Calendar, YouTube Music, Notes and Control Center.
They share one Rust process, the compositor, wallpaper cache, settings controls,
window placement and snap/fuse. No Electron process is started for widget UI.
Music retains its existing optional playback/browser adapter.

Each manager row has an On/Off switch and a separate Settings action. Closing a
widget updates the switch; visibility is restored next launch. Switching a
widget off preserves its data, appearance and geometry. App Dock can also be
hidden; the tray remains the entry to the manager.

## Package layout

```text
widgets/<id>/
  widget.json
  native/
    mod.rs          # native window, render, input and package-private modules
    package.rs      # WidgetHost implementation and NativeDefinition factory
    model.rs        # package-owned config, import and validation
```

For example:

```json
{
  "schemaVersion": 1,
  "id": "weather",
  "name": "Weather",
  "description": "Forecast on your desktop.",
  "runtime": "native",
  "order": 60,
  "hasSettings": true,
  "nativeEntry": "native/mod.rs"
}
```

IDs are stable lowercase identifiers (hyphens are supported); the folder and ID
must match. Entry files must stay inside their package. `build.rs` discovers
native modules and factories automatically. `mod.rs` exports `pub(crate) mod
package`; its `definition()` supplies a factory and `should_open` migration hook.
Use an existing package adapter as the example for the `WidgetHost` trait in
`native-dock/src/widget_packages.rs`. No manager enum, central registration list
or shared class-name whitelist needs editing. The registry marks the returned
main HWND for snap/fuse; settings/editor windows stay separate.

App Dock is the bootstrap package: its entry is `native/host.rs`, with
`nativeModule: "window"`. Shared services remain in `native-dock/src`; per-widget
models, themes and windows stay with their packages. Only native UI packages
remain; the optional Chrome playback adapter lives under `music/adapter/`.

## Adding or updating a package

Add its manifest, native source and lifecycle adapter, then quit Lumen from the
tray and double-click **Launch Lumen.pyw**. The launcher detects changed source
or manifests and rebuilds the local debug app. Discovery happens on the next
source build/start, not by loading arbitrary DLLs into a running process. No
installer or distributable is generated. For development, the equivalent is
`python run_native_dock.py`; `--widget <id>` opens a discovered package and
`--manage-widgets` opens the manager. When already running, the double-click
launcher summons the existing app rather than rebuilding a locked binary.

## Persistence and material contract

- `config/native-packages.json` stores visibility by stable ID. Native configs
  remain independent, including `native-controls.json`; legacy data is import-only.
- Use shared settings and header controls. Keep a fixed, readable font at all sizes.
- Theme backgrounds cover the complete chrome rectangle. The compositor owns
  the single rounded outer mask/bevel; avoid a second inset panel/background rim.
- Intentional content cards can be rounded, with comfortable inner/outer spacing.
- Background opacity affects chrome; content/control backing remains readable.
- Freeze cached frost during moves and refresh on release. Stop animations and
  device polling when hidden. Keep OS calls and expensive work off the UI thread.
- Safety checks cover every source package. Tests must not change real display,
  HDR, audio or microphone settings; use read-only diagnostics or pure validation.

Control Center uses native Core Audio and display configuration APIs on a private
COM worker, without recording audio or spawning a helper. It supports editable
control lists, speaker/microphone volume and mute, default device selectors,
pinned-device buttons, display attach/detach and HDR. The primary/last display
cannot be detached. Unsupported device switching/HDR returns an error. Hardware
writes require an explicit user release; adding a control does not execute it.
Edit works directly on the live controls: hover for remove/rename badges and
move by dragging a control onto another. + Add opens a visual catalog rendered
with the same glass material and icon/slider previews. Added items are marked,
and Back returns to the editable widget. Done exits edit mode. The compact grid
reflows with window dimensions; there is no Row button. Native controls reuse
Music's optical lens construction in a package-local rounded-rectangle cache.

## Compact native bounds

| Package | Minimum width × height (DIP) |
| --- | --- |
| App Dock |180×120|
| Clock |120×120|
| Calendar |168×190|
| YouTube Music |168×104|
| Sticky Notes |160×160|
| Control Center |160×112|

Tiny layouts retain core interactions and hide secondary information until enlarged.
Existing saved larger bounds remain intact. Notes uses a36-DIP Add glass lens,
shrinking to28 DIP in tiny layouts, with the same optical shader as Music.
