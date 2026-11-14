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

## Monitor input source

Add a per-monitor **Monitor input** control through Edit → Add. DDC/CI input
choices use VCP 0x60 and advertised capability values, never the default Windows
screen number. Windows physical monitor handles are released through RAII.

Input choices come from the monitor's DDC/CI VCP 0x60 capability list. HDMI,
DisplayPort, DVI and VGA labels describe ports, never the connected computer.
Unknown/vendor codes remain `Input 0xNN`; USB-C is not guessed from a vendor code.

Monitors with missing or incorrect capabilities can use a local, calibrated
`config/monitor-inputs.json`. Keys are the monitor device paths stored in
`config/display-recovery.json`, not DISPLAY1/2 numbers or model-name matches.
Each profile has `inputs: [{code, name}]` and `trust_readback: true/false`.
For unreliable readback, selection highlights only the last command in the
current session; on startup no port is falsely marked active. This installation's
measured calibration remains in local config, outside app source/distribution.
Profiles are device-instance-specific; another computer detects its own paths
and must calibrate a monitor that does not report its ports. No source switching
or brute-force probing happens during automatic detection.
