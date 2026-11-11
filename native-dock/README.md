# Native Rust Lumen host

The current host manages six [native widget source packages](../widgets/README.md).
Control Center is a Rust UI and native Windows backend in the same process.
Double-click `Launch Lumen.pyw` in the repository root for everyday use; it rebuilds
changed source packages when the app is closed and reuses an existing running app.
The manager uses On/Off switches with independent Settings actions.

A Windows-only dock with a native Win32/GDI+ UI, matching the original Lumen dock's
wide glass panel, pill search bar, shared group backgrounds, 48-DIP icons, labels,
toolbar and Add tile. The native UI uses no Electron or WebView. Music starts
the existing Node/Chrome playback adapter only when playback/Browse is requested;
that browser has its own resource cost.

From the repository root:

```text
python run_native_dock.py
python run_native_dock.py --no-build
python run_native_dock.py --check
python run_native_dock.py --no-build --smoke-test
python run_native_dock.py --no-build --inspect
python run_native_dock.py --widget controls --manage-widgets
```

`--check` runs the repository safety guard, guard regression tests, Rust tests,
and a development build without opening a window. `--smoke-test` opens the dock
for five seconds and prints startup and memory/CPU measurements before closing.
The CPU figure includes startup and is relative to one logical processor;
it is not a steady-state idle benchmark. `--inspect` temporarily exposes a foreground taskbar window so desktop UI tools
can inspect it. Ordinary runs use the desktop layer, hide the taskbar/Alt+Tab
entry, and keep the Lumen icon in the notification area (Show dock, Settings, Quit).

Rust's MSVC toolchain and Visual Studio C++ Build Tools are required. The launcher
finds Cargo on PATH, or uses a workspace-local toolchain under `.scratch/` if one
exists. First-time dependency downloads need network access. Builds go only to
`native-dock/target/debug/`; there is no installer or distributable packaging.

## Use

- On first use, imports items, icons, groups, columns and blur from
  `config/state.json`. Each package writes its own native config; the manager
  stores visibility in `config/native-packages.json`. Legacy data is import-only.
- Search includes pinned items and installed desktop/Store apps from the Windows
  Applications folder, Start Menu and App Paths. The catalog loads on a background
  thread and refreshes every five minutes. Search results stay temporary until
  explicitly pinned. Matching supports Vietnamese accent folding.
  Up/Down and Enter work from the search field;
  Escape or the search clear button clears search. Click a tile to launch it.
- Hover lifts icons by 1.5 DIP and scales them to 1.055 over 300 ms. Pressing
  scales to 0.97; launching shows a brief glow with duplicate-click protection.
- Drag past 4 DIP to move an app. A floating glass tile follows the pointer,
  a dashed placeholder marks insertion, and tiles settle with the old dock's
  260 ms easing. Drop on an existing group to join it; drag out to detach.
  Reordering is disabled during search. Escape or lost capture cancels a drag.
  Only a completed drop writes the new order to the native config.
- Click Add, then choose files/apps/shortcuts (all file types) or a folder.
  File shortcuts stay as shortcuts. Dropping files/folders onto the dock also works.
- Toolbar buttons activate on release inside the same button; dragging off cancels.
- The scrollbar fades in while the pointer is inside the widget, fades out on
  leave, and supports dragging the thumb with pointer capture.
- Glass Add and app menus match the widget. Right-click a pinned tile to Open,
  Rename, Change icon, Group with checked pinned items, Ungroup, or Remove from
  Lumen. Rename changes the dock label, not the original file. Custom PNG/JPEG/BMP/
  ICO images are converted into the local icon cache. Unpinned app results offer
  Open and Pin to dock.
- The gear opens a standalone glass settings page modeled on the old dock:
  Background sliders, frosted glass, nine surface palettes plus Wallpaper · Auto, columns, icon size,
  labels, search, floating mode and Quit. Settings are saved locally.
- Drag the narrow top margin or the gap beneath search to move the panel. Drag
  any edge or corner to resize. The opposite edge stays anchored, and narrower docks reflow
  columns and groups. Position and size are saved independently of Electron.
- Calendar opens a native month widget in the dock's process. See the calendar
  development notes below; the other widgets have their own implementation notes.

The glass reads and blurs the wallpaper image file, supports Windows fill/fit/
stretch/center/tile/span layouts, and caches the result. It never captures the
desktop. PNG/JPEG/BMP wallpapers work; other formats use a solid fallback. Exact
browser shadows, startup registration and other widgets remain future work.
Wallpaper · Auto is the default Surface palette and favors prominent saturated colors from a
small copy of the wallpaper image. Saturation-weighted area selects colorful
regions over pale backgrounds without favoring tiny highlights. When vivid
colors cover at least 5% of eligible pixels, selection uses that vivid pool
and aggregates neighboring bins so gradients are not outvoted by flat pale
backgrounds. Otherwise neutral wallpapers retain their natural palette. It preserves
the selected color's brightness
instead of forcing a dark palette, and includes light wallpaper regions while
ignoring extreme glare. It updates when the wallpaper file changes,
uses the existing decode/cache path, and does not read Windows accent settings.
Legacy Windows-accent palette selections migrate to Wallpaper · Auto. Search
retains a subtle selected-color tint even at 0% panel tint, with a recessed
translucent fill and restrained edge. The enclosing widget carries the upper-left
reflection with a wide, soft falloff at half its previous opacity; search has no raised shadow at rest.
The panel tint, coating and corner reflection draw at the current device size
before the bevel. Rounded highlight edges use an antialiased fill rather than
a binary clip; only the unshaped gradient/noise texture stretches during resize. A separately
blurred backdrop sampled from the widget-owned bitmap gives search its own
frosted layer, with a small optical offset. It is sampled before the decorative
corner glow so that glow does not brighten search. The search material blends
at 50% over its undecorated backdrop; coating and rim opacity are halved too.
Its luminance-dependent shading
deepens dark wallpaper regions while preserving bright transmission, rather
than covering the whole bar in a uniform dark film. A 24% luminance-matched
accent mix with 20% extra saturation gives search stronger palette color
without flattening wallpaper highlights. Grain uses two-pixel cells
at 36% of its original strengthened opacity.
The placeholder is about 58% opaque (65% focused); toolbar icons idle at 62%
and brighten toward 90% on hover. This layer is cached across typing
and hover, reused while resizing, and refreshed when the underlying background
changes. Click/launch and group
selection accents derive from the same palette.
Wallpaper decoding applies EXIF rotation/mirroring before sizing, blurring and
color selection. Fill uses a centered crop of that oriented image; other fit
modes retain their layout.
The final move position and display changes invalidate the wallpaper crop cache.

The toolbar uses the original SVG paths and a common 1.7-unit stroke, rasterized
at twice the device resolution. GDI+ supplies vector paths and cached text;
sprites blend directly into the owned premultiplied buffer.
App icons are prefiltered once with Lanczos and sampled with continuous
premultiplied bilinear coordinates, avoiding GDI+ scale/translation wobble.
Per-pixel premultiplied alpha replaces the binary Windows region, smoothing the
actual window contour on fractional display scales. A simulated refractive
bevel with a single edge, fading corner reflections and a soft shadow give the glass more depth. The native
renderer approximates the old surface rather than promising pixel-identical
Chromium rendering. Design reference:
[Apple materials](https://developer.apple.com/design/human-interface-guidelines/materials).
The compositor uses
[UpdateLayeredWindow](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-updatelayeredwindow).

The search uses white glass in the light theme and darker glass in dark themes,
with an animated focus shadow; groups use a
subtle light glaze. Cached opalescent gradients remain visible at full tint.
Hover surfaces have a directional reflection and shadow. Light themes use a
white hover material and white pointer glint instead of a dark wash. Toolbar symbols use
the old dock's round hover background and 60-degree / 600 ms rotation.
The scrollbar stays outside tile hover surfaces and inset from the panel edge
and rounded corners. Group/hover shadows are compact and cast down/right.
The hover rim has a thicker top reflection. Shadow masks are continuously
blurred and cached, rather than stacked opacity rings. Icons and labels have
cached shadows; icon shadows use a broad circular Gaussian with transparent
outer padding, and cached tiles expand to contain it. Dark labels omit their
text shadow. Labels use 11-DIP
Segoe UI Variable Text Bold. Labels are grid-fitted at the final device DPI
and composed 1:1; settled tiles snap to device pixels to avoid a second filtering
pass. Scrolling keeps continuous translation until it settles. Text and toolbar foregrounds choose contrast from the
actual composited glass pixels, including tint/frost/opacity and wallpaper;
one shared foreground palette changes across the entire widget together,
with hysteresis to prevent rapid switching. Smoked and mid-gray glass prefers
white text; dark text is reserved for clearly bright glass. Viewport fades cover fractional
device-pixel rows to avoid an exposed bottom line. Scrolling
uses continuous pixel offsets with a critically damped spring; repeated wheel
input preserves both position and velocity. Search retains icon positions
across filters; groups move with their apps,
and partial rows fade at the top and bottom of the viewport.

Animation timers stop after motion settles. The animation clock requests precise
timer pacing only during motion. A 10 ms timer wake-up and DWM presentation
pacing avoid Windows rounding a 16 ms timer to approximately 31 ms. Input is
coalesced into animation frames; paint never schedules another immediate paint.
A static monochrome grain layer is cached with the glass so it does not shimmer.
Wallpaper is rendered directly into the widget viewport and the finished
widget background is cached. Moving freezes that cache until release; there
is no extra full-monitor bitmap. Partial frost blending uses only a viewport
scratch buffer. Wallpaper sources, fonts,
offscreen buffers, vector icons, prefiltered app icons and labels are cached.
Static app contents and group materials are composed into cached sprites.
Resize input is coalesced per frame and reuses the existing sprites while held;
final text layout is regenerated in small batches after release. The wider outer widget shadow
uses a small cached nine-slice alpha template on a non-interactive owned window.
The temporary full shadow DIB is released after Windows copies it. Its position
follows native window-position messages before repaint rather than waiting
for wallpaper rendering. Gradient/noise
textures are cached, and auxiliary buffers do not rebuild contour masks. A list
background is allocated only when content overflows, so viewport fades blend
icons, labels, and group surfaces together. Closed Settings releases its paint
buffer; soft-shadow cache masks store one alpha byte per pixel. Search retains a native Edit
control for input, selection and IME; only its own client is composited.

On this machine, a five-second smoke check with 29 pinned items reported
84.89 MiB working set, 46.25 MiB private memory and 521 ms to the first window
after the memory pass (previous same-session sample: 119.10 / 83.83 MiB).
These are dock-only development-build measurements, not a full widget-suite
comparison. The short CPU average includes wallpaper decoding. Profiling found
GDI+ sprite composition and repeated wallpaper scaling dominated frame time;
see [PROFILING.md](PROFILING.md) for before/after samples and remaining limits.

Logs: `config/logs/native-dock.log`. Close the native dock before rebuilding it
on Windows. Quit the original app if you want to measure this dock in isolation.

This is an unsigned local development build. Passing the source guard does not
guarantee Defender acceptance. See `../SECURITY.md` for the supported behavior.

Group with... enters a staged checkbox mode directly on the app grid. The source
app starts checked; click any other tiles to toggle them, then use Done (at least
two apps) to create one group from exactly those checked apps. Cancel or Escape
discards the staged selection. Arrow keys and Space support keyboard selection;
Enter commits. Scrolling and edge resize remain available, while launching and
reordering are suspended during selection.

Background opacity controls the outer widget through premultiplied per-pixel
alpha, so actual desktop windows can show through. Search and grouped cards
retain their full glass backing independently; icons and labels retain their
own opacity. The normal fully opaque path stays unchanged. The translucent
path uses a GDI+ context over the existing DIB, preserves alpha in viewport
fades, and caches native Edit glyphs recovered from two app-owned mattes.
The outer shadow follows background opacity. Settings omits manual Refresh
icons; icon caches are rebuilt on app startup and explicit icon changes.

Floating window uses Windows topmost z-order, including on startup and when
switched from Settings or a context menu. Disabling it removes topmost status
and returns the normal dock to the desktop layer. Transparent chrome retains
a 1/255-alpha input floor inside the rounded shape so move/resize targets still
receive mouse input at zero background opacity.
App labels stay at 11-DIP Bold and their native pixel dimensions during icon
size changes. Stale labels are clipped rather than stretched, and tile cache
keys include the current label layout. Popup actions use pixel-aligned Segoe
UI Semibold at the current DPI.

During a native window move, Windows moves the existing layered surface and
the frosted background/search backing stays frozen. Any incidental repaint
reuses the same background when dimensions/DPI match. Release invalidates
the crop once at the final position. Column and icon-size changes reflow the
app grid inside the existing bounds; they never auto-resize the widget.

## Native calendar

Open Calendar from the dock toolbar, the tray's Show calendar command, or launch
with `python run_native_dock.py --calendar`. It shares the dock's Win32 UI thread,
GDI+ renderer, wallpaper-file source and layered-window compositor. It includes
the 42-cell month grid, previous/next month, Today, date selection, Sunday/Monday
week starts, all-edge resize, window dragging, floating mode and native glass
settings. Page Up/Down navigate months, arrows select dates, Home returns to Today,
and Escape leaves settings or cancels a window drag. No clock is embedded.

Lumen uses wallpaper glass. Almanac, Blocks, Pop, Prism and Grove retain distinct
date artwork and palettes; Blocks/Pop/Prism/Grove include month progress. These are
native renditions, not pixel-identical browser layouts. Settings expose theme,
week start, frost, floating mode, background opacity and tint. Geometry stays put
when changing themes. The native calendar saves only `config/native-calendar.json`;
its initial import reads the old calendar geometry/week start and app appearance
without modifying `config/calendar.json` or `config/state.json`. Invalid native
configuration is reported and left untouched.

The calendar matches the current date-only Electron widget. It does not display
events or connect Graph/ICS. Wallpaper is cached while dragging, refreshed at the
final position and checked once per minute. Hover timers stop when settled.
The same-process wallpaper decode cache is shared; each calendar owns only its
viewport surfaces. `--smoke-test --calendar` exercises startup without saving.

Calendar-specific tests: `cargo test --locked --manifest-path native-dock/Cargo.toml calendar::`.
The ignored `calendar::tests::render_calendar_preview` test writes a six-theme
contact sheet under `.scratch/` from the widget-owned buffers in hidden windows;
it does not capture the desktop.

## Native clock

Open the Rust clock using `python run_native_clock.py`, or
`python run_native_dock.py --clock`. In an existing updated native host, the
launcher forwards the request to that host. The notification tray also has
**Show clock**. Electron's primary entry point remains `python run_lumen.py`.

The clock is a second Win32/GDI+ layered window in the dock process. Both share
one wallpaper decoder/source and the native canvas, contrast, rounded-alpha
compositor and shadow implementation. It keeps nine distinct content layouts:
Lumen, Afterglow, Paper, Nightfall, Colour block, Split flap, Daybook, Timeline
and Terminal. These are native interpretations of the existing layouts.

Use the gear or right-click for clock settings. The Clock face tab selects the
layout, time zone, 12/24-hour format and seconds. Glass & window controls frost,
floating mode, background opacity and tint. The zone button cycles the same ten
choices as the original settings. Imported additional IANA zones are resolved
through Windows ICU. Windows supplies timezone/DST rules; there are no fixed
UTC offsets. Left/Right on the clock cycles faces; Escape closes settings.

Drag the face to move; resize from any edge/corner (280–1100 DIP). Theme changes
preserve bounds. Frost freezes while moving and updates at release. Background
opacity changes chrome alpha while content stays readable, and rounded hit
regions remain usable at zero opacity. Close hides only the clock; restore it
from the tray. Hidden clocks stop their clock timer and release frame buffers;
visible clocks tick at second boundaries, or minute boundaries with seconds off.

First launch imports `/settings/clock` and clock opacity from `config/state.json`
without editing it. Native settings, position, size and open state are saved
atomically in `config/native-clock.json`. Invalid/future configs produce an error
and are left intact. `--smoke-test` does not save clock changes.

`python run_native_clock.py --check` runs the shared safety checks, Rust tests
and debug build. Clock tests cover midnight/noon, day progress, DST transitions,
IANA mapping, import/round-trip, corrupt config preservation, all nine renderers,
settings pages, zero opacity and show/hide/restore. For optional render previews,
set `LUMEN_CLOCK_PREVIEW_DIR` and run the `clock::` Rust tests; they export pixels
from the app-owned surfaces, without capturing the desktop.

## Native Music development

`python run_native_dock.py --music` opens the Rust Music surface from source.
The Music toolbar and tray item open the same widget. Playback uses the existing
Node/Chrome adapter; the UI has no Electron renderer or WebView. See
[YouTube Music](../docs/youtube-music.md) and [Music handoff](HANDOFF-music.md)
for implemented behavior, isolated profile paths and pending coordinated QA.

## Widget management and snap/fuse

Open **General Settings / Manage widgets** from the tray, or the Apps/grid button
on Dock. Show/hide each widget, open its settings, or Show all / Hide all. General
Settings stays usable with every widget hidden. Shared background opacity and
floating controls apply to all five native widgets without changing their themes
or geometry. Selecting a widget's Settings dismisses the manager to reveal it.

Snap & fuse is on by default and can be disabled in General Settings. Drag the
glass edges close together (about 12 DIP) with at least 48 DIP of overlap. Both
widgets must be on the same layer (both floating or both desktop). The shared
outline makes a soft concave junction, removes internal rims and uses one outer
shadow. The nearby materials blend to the same wallpaper sample; themes/content
away from the seam stay independent. Chains and T-shaped multi-widget groups work.

Drag any member away to detach only that widget. Resize any member independently;
connections are reassessed on release. Per-widget saved bounds reconstruct the
group after restart. Settings/editor/browser windows are excluded. Whole-group
dragging and animated liquid morphing are not implemented.

Bridge HWNDs and component caches are reused when unchanged. Translucent overlap
pixels have one owner, avoiding a dark seam. Failed uploads or components above
the temporary 16-million-pixel bridge budget fall back to standalone chrome and
shadows rather than leaving a half-fused surface. See [integration QA](INTEGRATION-QA.md)
for validation and remaining limits.
