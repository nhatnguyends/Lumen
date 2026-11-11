# Native widgets

See [source package instructions](../widgets/README.md) and the [app guide](../README.md).
All widget windows use Rust/Win32. Control Center implements device groups,
volume/microphone volume with embedded mute, display toggles and per-display HDR.

Edit keeps the live layout. Hover to remove or rename; drag to reorder. Add opens
a separate visual manager with search, categories and a presence switch per
control. Presence switches add/remove the widget control, while device-row
switches operate Windows hardware settings. The primary monitor is marked
Main Display and its toggle is gray and disabled.

Resizing reflows named controls; a short panel becomes a horizontally scrolling
row. Scrollbars can be dragged. Edge fades show remaining content; the first
control stays fully visible at scroll position zero. Device rows track Windows
hotplug notifications and periodic refreshes, without changing saved control order.

Settings: `config/native-controls.json`. Monitor restore identities and geometry:
`config/display-recovery.json`. User config and imported legacy data are preserved.
