# Clock legacy visual fidelity + Afterglow motion — latest user request

Direct user asked for dynamic Clock animation (sun becoming moon), preserving
Afterglow's old gradient, then clarified all theme visuals still differ and
reading the old code is acceptable instead of live screenshots.

Implemented against `src/renderer/src/clock.css` and `ClockApp.tsx`:

- New `clock/visuals.rs`: source palettes and layered theme paint for all nine
  themes. Afterglow restores its165deg blue/lilac/peach gradient plus warm orange
  lower-left and pale lower-right radial layers. Lumen/Ocean gradients,
  Daybook/Timeline glows, Paper ivory, Nightfall green-black, Terminal scanline,
  Colour block black base are retained instead of flat approximation colors.
  The artwork is used for Surface palette=Theme material (legacy/default);
  explicitly chosen manual palettes still work. The texture stays cached.
- Colour block full/compact: original blue #386ef5, orange #ff9226 and red
  #ff432c, dark tile text and circular red dial, rather than translucent pastel.
- Split flap: original dark split-card halves, cream digits and Impact type;
  Paper/companion dials: original red second hand and current-color pin;
  Daybook: separate lime/green/cyan progress accents. Native light display
  typography restores the Lumen/Afterglow/Nightfall character.
- Necessary small shared-render addition: `render.rs` cached font family selection
  for Segoe UI Light and Impact (weight namespaces3400/2400) plus
  `Canvas::text_clock_display`. Existing font routes/chrome remain unchanged.
- New `clock/afterglow.rs`: continuous outlined sun→crescent morph, slow rotating
  rays, gentle bob and night-star twinkle; no background-colored eraser. Default
  follows the Clock's chosen zone (artistic dawn05:30–07:00/dusk17:30–19:00).
  Settings add Dynamic animation and Clock time /24-second decorative loop.
  The legacy gradient itself stays intact, not replaced by a new night palette.
- Target40ms frame timer only for visible, non-minimized, stationary Afterglow
  with motion enabled. Stops on hide/move/minimize/theme change/destroy. Existing
  second/minute time timer is retained. No timer for other themes' new motion.
- Afterglow text/date/footer cache updates once per displayed second/minute;
  animated frames draw the vector emblem over that native-DPI sprite. No
  wallpaper decode/blur or theme-gradient raster each animation frame. New
  textures/sprites are owned by Clock state and released on hide; no TLS images.
- Backward-compatible config additions: dynamicAnimation=true,
  animationMode="clock". No bounds/theme/data reset.

Verification: `cargo check --locked --manifest-path native-dock/Cargo.toml`
passes; safety guard passes. **Only pure tests run**:5 afterglow tests plus2
visuals tests pass; no HWND/GDI+ test case, no desktop input, no running app
restart since root's native-test pause. These cover clock/loop phase continuity,
bounded closed morph paths, hidden/moving/disabled timer policy, cache key
invariance across milliseconds, visible option invalidation, legacy config
roundtrip, color anchors/opaque texture generation across nine themes.
Pure gradient PNGs were inspected at `.scratch/clock-legacy-paint/` (these are
paint references, not screenshots or full-widget native-frame verification).

Root remaining QA: serialized full Clock render/native tests for all9 revised
faces, min/wide/low layouts, Impact/Light font metrics, new Afterglow cache alpha,
header integration, settings persistence/timer lifecycle, then debug build and
single coordinated runtime reload. No claim of pixel-identical full-widget QA
or new running binary yet. Other themes' possible animations (smooth dial,
flipping cards, orbit/ruler/meter motion) were discussed as options, not claimed
implemented. Current code changes are complete for the explicit Afterglow and
legacy-visual requests; no shared host/fusion/tray edits.

---

# Clock ready — detached settings / unified header / 160-DIP layouts (2026-11-10)

**Source edits complete. No remaining implementation changes planned. HWND tests
remain paused per root. No desktop actions, runtime restart or packaging.**

## Current files / API

Only Clock sources plus this handoff were changed for this request:
`src/clock.rs`, `src/clock/model.rs`, `src/clock/face.rs`; new
`src/clock/{settings,compact,tests}.rs`. Root/Calendar own shared `settings_ui.rs`,
root module wiring, host/fusion and debug build.

- `Host::show_settings()` now opens/reuses one `settings_ui::Host`, never changes
  Clock's visibility/config/geometry/content, including when Clock is hidden.
  Separate settings class `LumenNativeWidgetSettings`; owns independent minimum,
  scrolling, close/Escape and surfaces. Owner teardown drops it before Clock.
- Existing Host APIs preserved: hwnd/is_visible/show/hide/refresh/apply_common.
  Clock owner drains kit notifications at `WM_APP+185`; no additional root wiring
  needed beyond existing show_settings routing and shared module registration.
- Shared `widget_header_hits` / `draw_widget_header` used directly: Clock title,
  12-DIP semibold, outline20-DIP icons, 32-DIP hit areas, release-in-same-button.
  Old clock zone header/inline settings/cycle sliders removed.
- Settings schema follows Background → Clock face → Window, using shared palette
  data and controls. Own opacity, tint, frost strength, blur, surface palette,
  nine content themes, timezone, hour format, seconds, floating. Default
  `palette="theme"` preserves legacy content material; other options use kit data.
- Added backward-compatible config fields `blur`, `frostStrength`, `palette`.
  Shared Glass is never changed for Clock appearance. A Clock-only cached blurred
  viewport samples its shared sharp wallpaper source; no extra wallpaper decoder.
  Texture reuse while moving/resizing; hidden Clock frees texture/frame/backing/
  shadow and stops its clock timer. Non-blur slider changes do not rebuild frost.
- `MIN_SIZE=160`, `MAX_SIZE=1100` used in validation/min-track; small saved bounds
  survive restart. Settings minimum remains the kit's 420×360, not widget minimum.
- Nine real compact layouts below280 DIP; secondary details intentionally reduced,
  native text sizes retained, no bitmap text compression. AM/PM gets a separate
  row/slot, dial identity / orbit / split cards / bars / ruler / Terminal retained.

## Verification actually run

Before native-test STOP: **8 clock tests passed**, exit0, 3.20s,113 unrelated tests
filtered out. Separate settings singleton/close/Escape/owner invariance, settings
minimum/scroll, opaque content at zero chrome alpha, preset/invalid values,
General API, source wallpaper pixels unchanged by own blur settings, hidden
resource release, import/corrupt config/DST, compact bounds/appearance restart.
The owned-render matrix was 9 themes ×12/24h ×5 sizes ×4 raster DPI settings
(1/1.25/1.5/2). Tests hold shared `native_ui_test_lock`; root now serializes native
processes as well. Exact last command/output is in the urgent report below.

App-owned previews visually reviewed for all nine160×160 12h faces, wide/low,
narrow/tall, full detached settings,420×360 settings and scrolled theme controls.
Folder `.scratch/clock-compact-preview/` (source file wallpaper only for settings,
not desktop capture). Shared-kit rendering matches the Dock shell/control style.

**Post-STOP final source adjustment:** viewing the already-generated320×160 preview
showed the date row could touch the bottom. Compact digital font now respects
both width AND available content height; Daybook meter spacing reserves its last
bar. Added pure regression
`clock::face::compact::tests::compact_digital_date_and_meter_rows_fit_wide_low_and_narrow_tall`.
Pure arithmetic checks for five aspect ratios pass. Final `cargo check` exits0
and `node scripts/check-safe.mjs` passes. No HWND/test EXE launched after STOP.

## Root's remaining verification

Run the final clock tests once in the serialized root process (including the new
pure geometry case and refreshed wide/low previews), then debug-build/reload
user runtime. Live QA: common manager/tray/general settings all route to the same
separate window; slider appearance isolation; crop/blur at real monitor/DPI;
160-DIP resize/restore and fuse/detach; close/settings/visibility lifecycle.
No known remaining Clock source error. Last build warnings were unrelated Notes
Design and kit is_visible dead-code warnings. Root reports shared header TLS
Image lifetime cause fixed via pixel-only cache; no edits to that shared fix here.

---

# URGENT native-test stop report — 2026-11-10

Received coordinator STOP request after the following Clock command had already
completed. No more cargo-test/HWND/native process launches will be made until
root permits them. No desktop action, popup dismissal or app restart.

Last native command (inside the standard `run_native_dock.toolchain()` Python
environment):

`cargo test --locked --manifest-path D:\CustomTools\Widget\native-dock\Cargo.toml clock::`

`LUMEN_CLOCK_PREVIEW_DIR=D:\CustomTools\Widget\.scratch\clock-compact-preview`.
Executable printed by Cargo:
`native-dock/target/debug/deps/lumen_native_dock-0114aacfc302aa34.exe`.
Result: **8 passed; 0 failed; 0 ignored; 113 filtered out; finished in 3.20s**;
command exit **0**. Tool cell then ran `cargo check --locked --manifest-path ...`:
exit **0**, finished in 0.57s (unrelated Notes Design / kit is_visible warnings).
No crash output or application-error dialog was observed by this chat.

Last printed test was
`clock::tests::settings_adapter_rejects_invalid_values_and_preserves_geometry_theme_and_shared_source ... ok`.
The preceding HWND tests were
`all_faces_render_at_minimum_and_mixed_dpi_without_losing_content_or_hit_targets`
and `detached_settings_are_singleton_keep_owner_unchanged_and_close_independently`,
both **ok**. All three Clock HWND tests hold `crate::native_ui_test_lock()`, but
as root notes this does not serialize separate cargo-test processes.

The earlier retry before this success encountered link.exe LNK1104 (test EXE
locked); it did NOT execute a test process. Subsequent successful runs are the
ones reported above.

Source-analysis lead for root (not a demonstrated root cause): GDI+ Runtime is
per-test, while renderer font/image/HEADER_SYMBOLS caches are thread-local.
Inspect whether cached GDI+ objects are disposed/reused after the last
GdiplusShutdown on a test thread, and whether unlocked pixel tests overlap
native tests. UPDATE: root confirmed the shared header TLS Image lifetime was
the cause; kit now caches pixels only and root serialized full suite passed. Also inspect common settings Host Drop / owner teardown and fusion
subclass cleanup. Clock's own teardown drops settings before DestroyWindow of
its main HWND while the boxed State remains alive. No source changes made in
response to this crash report.

---

# Native Clock — handoff

2026-11-09. Clock implementation is complete for coordinator integration. **The final Clock tests and cargo check PASS with the landed fusion APIs and General Settings Host API.** Clock calls `fusion::revision(HWND)` and `Buffer::bevel_for(HWND)` per contract. No compositor/fusion source was edited here. No desktop actions/restarts after the coordination request.

## Files / shared edits

New: `native-dock/src/clock.rs`, `clock/model.rs`, `clock/face.rs`, `run_native_clock.py`.

Earlier shared edits (made before the request to leave shared files to coordinator):

- `main.rs`: module registration; `window.rs`: `App.clock`, show/refresh/drop, shared `Rc<RefCell<Glass>>`, crate-visible window-mode helpers, CLI existing-instance forwarding (`WM_APP + 31`); `settings.rs`: updated shared-source borrows.
- `tray.rs`: Show clock, action 4. `run_native_dock.py`: `--clock`. `Cargo.toml`: Win32 Time/SystemInformation features.
- `render.rs`: `Canvas::text_monospace`, cached Consolas for Terminal; cache weights 1400/1700 encode mono regular/bold, actual font weight uses modulo 1000; existing text paths unchanged.
- Safety guard and fixture cover `run_native_clock.py`; README documents clock usage.
- Fixed an early Calendar import compile error for mouse-tracking types; no intentional behavior changes in another widget.

## Ported behavior

- Dock process/UI thread, shared wallpaper decoder/bitmap, existing GDI+ canvas, alpha compositor, contrast and shadow. No extra clock process/browser.
- Nine distinct old layouts: Lumen, Afterglow, Paper analog, Nightfall orbit, Colour block, Split flap, Daybook meters, Timeline and Terminal/Consolas. Native interpretations, not pixel-identical Chromium artwork.
- 12/24-hour, seconds/date, old ten timezone presets; extra imported IANA zones map through system ICU. Windows handles DST.
- Gear/right-click settings: face, zone, hour format, seconds, frost, tint, opacity and floating mode. Settings backing remains normal opacity; zero-opacity clock chrome retains opaque content and rounded input floor.
- Native move/resize hit testing (280–1100 DIP), uniform content scaling, theme changes preserve geometry, freeze frost while moving, refresh after release, shadow follows position.
- Close hides only Clock, kills its timer, releases frame/background/shadow; Show returns to clock face. Timers align to seconds/minutes according to seconds setting.
- Atomic independent `config/native-clock.json`; import old clock bounds/options/opacity without editing Electron state; reject/preserve corrupt/future configs; smoke mode never persists.
- Fusion key now includes `fusion::revision(self.hwnd)`; rebuilt backdrop calls `bevel_for(self.hwnd)`. Stable revisions retain the cache.

## Entry points

`python run_native_clock.py` / `python run_native_dock.py --clock`; tray Show clock. Existing updated host accepts `WM_APP + 31` on Dock and opens Clock. `Host::create/show/refresh/hwnd/is_visible/hide/show_settings/apply_common` are available; Clock HWND class `LumenNativeClockWindow`, `WM_CLOSE` hides, `WM_CONTEXTMENU` opens settings. General Settings API matches the other widgets: `hwnd() -> HWND`, `unsafe is_visible() -> bool`, `hide()`, `show_settings()`, `apply_common(opacity: Option<f32>, floating: Option<bool>)`. Absent fields preserve choices; geometry/theme are preserved; changes save even while hidden, without allocating hidden frames. No dedicated Clock toolbar icon. Widget manager, General Settings, fusion registry and desktop QA belong to coordinator.

## Checks actually run

- Earlier full `python run_native_dock.py --check` builds passed (last such full build: 75 passed, 2 ignored, before final refinements).
- Earlier broad `cargo test --locked --manifest-path native-dock/Cargo.toml`: **83 passed, 3 ignored**, followed by passing `cargo check`.
- Final clock-only tests after fusion hooks and General Settings API: **5 passed**; cover midnight/noon/day progress, DST, IANA validation, import/round-trip/corrupt preservation, all nine renders, settings, HWND title/show, geometry preservation on theme switch, premultiplied alpha and opaque text at zero chrome, hidden-buffer release, hide/show, no smoke writes. New cases cover minimum/non-square frame dimensions and the General Settings API, including absent fields and applying changes while hidden.
- Guard and all **6 safety regressions** passed. Launcher `--help` passed.
- App-owned previews visually inspected: all nine layouts, settings, 12-hour variants. Fixed AM/PM overlap and clipped centered time found in previews. `.scratch/clock-preview/{theme}.png`, `{theme}-12h.png`, `settings-face.png`, `settings-glass.png`. Unloaded wallpaper source: these validate layout/fallback material, not real-wallpaper crop.
- Earlier startup-inclusive five-second dock+clock smoke: same PID, clean exit, 94.74 MiB working set / 47.49 MiB private. Not an idle/four-widget profile.

## Final verification needed / real limits

- New 280×280 / 520×280 / 280×520 owned-surface render cases were added after the last passing test; rerun when fusion APIs land. Final hook changes are not claimed as compiled until then.
- Final live desktop move/resize, mixed DPI, opacity-zero hit testing, topmost/desktop/tray restore, existing-instance forwarding and fusion need sequential QA. Earlier live inspection was inconclusive during concurrent host manipulation.
- Blur/frost strength are inherited from the shared host source; Clock exposes frost on/off, own tint/opacity, not independent blur/strength sliders. Zone UI cycles presets rather than dropdown. Custom buttons lack Tab/UIA navigation; header hover is basic, without Dock rotation animation.
- Extra IANA zones need system ICU; missing mapping reports an error and preserves config. Preset mapping does not need ICU.
- No runtime restarted for final refinements; rebuild shared debug binary in coordinated QA. No release/installer output. No changes to snap/fuse implementation beyond the two requested Clock hooks.

## 2026-11-10 settings / compact work in progress

Clock is preparing its schema and compact 160×160-DIP layouts while Calendar owns
`settings_ui.rs` and `SETTINGS-UI-CONTRACT.md`. Required kit capabilities: a single
reusable detached HWND (outside fusion), Dock shell/header/close/Escape/scroll,
sections, slider/value controls (opacity/tint/frost strength/blur), switches,
choice controls/cards (9 content themes, zones, surface palette), and a callback
that can update Clock without holding a kit or Clock RefCell borrow across native
show/position/paint. Need create/show/sync/close or owner callback notification,
plus an owned-buffer/render test hook. Clock `Host::show_settings()` will open kit
window without showing/resizing/hiding/replacing the widget. Kit HWND must be
excluded from fusion, use independent min size and normal-opacity backing.
Clock-local blur will sample the shared sharp wallpaper and cache its own viewport
frost, never call `set_blur` on shared Glass. No new full wallpaper decoder.

Clock adapter now consumes the published shared kit and header helpers directly.
No separate settings renderer or duplicated header implementation. Notify message
is `WM_APP+185`. `Host::show_settings` no longer calls `show()` on Clock; it opens
one shared-kit HWND and keeps a hidden owner hidden. All Background controls are
schema sliders/switches/choices, including Clock-local frost strength/blur/palette.
New module `clock/compact.rs` provides real 160-DIP layout variants; native min,
config clamping and restart validation use one MIN_SIZE=160 constant. Shared
header 12-DIP semibold /20-DIP icons/32-DIP hits is reused. Tests queued against
actual shared kit include independent HWND close/Escape/reopen, owner config/
geometry/content preservation, 9 faces×12/24h×5 sizes×4 raster DPI settings,
premultiplied alpha, accessible-sized header targets, hidden buffer release,
common settings API, and compact persistence. Test run currently waits for Music
view's in-progress `coverflow_content` implementation to compile; no peer files
were modified to bypass the build.

## 2026-11-10 Afterglow reference and location

Implemented the user's old-widget reference layout: left-aligned thin time,
measured adjacent AM/PM, tracked eyebrow/weekday/footer, grouped date, semicircle
sun above the horizon and progress percentage at the right. Location replaces
the Afterglow header title and follows the configured time zone (Ho Chi Minh for
Asia/Ho_Chi_Minh; compact minimum uses HCMC). Local time uses the Windows zone
mapping, without geolocation. The legacy gradient and clock-time sun/moon
animation remain enabled. Fixed cached foreground PARGB handling so text edges
remain smooth. Shared render.rs adds GDI+ text measurement methods for Clock.

Verified cargo check, the exact reference render test (five square/compact/wide
sizes, premultiplied alpha and animation foreground-cache reuse), the city-label
unit test and scripts/check-safe.mjs. Inspected generated 420x420, 160x160 and
480x300 images in .scratch/clock-afterglow-reference/. No live app restart;
shared host reload remains coordinated with other widget work.

## Shared chrome coordination status
User-requested restart completed: PID 23872, responsive, Clock sunset created; no ongoing live QA or further restart planned in this chat. Desktop manipulation is paused for root's final reload. Preserve root's shared title/content-top and compact layout changes in subsequent Clock work.


## More prominent Afterglow graphic
Enlarged compact sun/moon from 32 DIP to 42% of content width (54-100 DIP), reserved date width accordingly, enlarged full-face emblem to 96x69 normalized units, and brightened/thickened outline/rays. Compact Afterglow alone reserves 44 DIP top for the location row; shared compact chrome helper remains unchanged. Exact five-size reference render test and safety guard pass. Previews: .scratch/clock-graphic-prominence/. No live app restart performed for this change.


## Shared Surface palette adapter READY (source-only, pending root API)
Clock adapter now uses surface::normalize on validation/load (theme preserved), surface::valid guards, surface::color and common palette_choices; legacy flat palettes normalize to matching custom hex. surface::paint is in cached backdrop after base tint and before coating/contrast/bevel. Content themes/Afterglow animation/compact chrome hooks remain intact.
Files: clock.rs, clock/model.rs, clock/settings.rs, clock/tests.rs. Added model restart test (dawn/custom/ocean migration/theme, invalid picker actions), settings adapter coverage and cached-backdrop rebuild/reuse test. Updated old ocean assertion for migration. Native tests intentionally NOT run under root QA slot. surface.rs not yet present at handoff, so compile awaits shared API publication. No app restart or HWND test process. Root can run serialized full suite when shared module lands.


Shared surface API follow-up applied: tests use gradient:dawn namespaced key. Clock now skips prior flat tint c.round for surface::managed palettes, then paints shared surface once at tint/100 (100% without frost). Auto/theme pipelines preserved. Shared module inspected; no native tests or restarts performed, as root owns serialized QA.

