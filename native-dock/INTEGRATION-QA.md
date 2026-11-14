# Native Lumen — coordinated integration QA

2026-11-09. This report supersedes the pre-integration QA limitations in the
individual widget handoffs where a later result is recorded below. It describes
the native development build, not a packaged release or the Electron entry point.

## Result and scope

The four widget chats were coordinated against one shared workspace. Dock,
Clock, Calendar, Notes and Music now share one native host process and wallpaper
source. General Settings and multi-widget snap/fuse were integrated in that host.
This is evidence for the tested paths, not a claim that every possible scenario
is bug-free or every old Chromium animation has been ported.

### Automated verification

| Check | Result |
| --- | --- |
| `python run_native_dock.py --check` | 104 Rust tests passed, 5 intentionally ignored; development build passed |
| Repository safety guard | Passed; all 6 safety guard regression tests passed |
| `npm run typecheck` | Passed |
| `npm run test:music` | 18 passed |
| Fusion-specific tests | 10 passed, included in the Rust total |

Ignored tests are explicit live-data/catalog/preview/performance diagnostics, not failed
functional tests. Tests using real HWNDs serialize desktop-wide z-order and
activation operations with one test mutex. Pure model/geometry/pixel tests remain
parallel. This matches production's single UI thread and removes an observed
parallel-test TOPMOST race.

### Desktop QA actually performed

The session used the source launcher in inspection mode at 175% DPI. Normal
startup uses tray mode; inspection mode temporarily exposes app windows for QA.

| Area | Observed result |
| --- | --- |
| Tray registration | Registered successfully on the interactive desktop |
| Dock toolbar | Apps opens manager; Music, Notes, Calendar open their existing native hosts; gear opens Dock settings |
| General Settings | Both tabs open; visibility reflects actual HWND state |
| Hide all / Show all | All five widgets hide, General remains usable, all five restore |
| Per-widget settings | Clock, Calendar, Notes and Music settings opened; Dock settings opened from gear |
| Settings focus | Manager dismisses when selecting widget Settings, exposing the selected settings surface |
| Shared opacity | Applied 50% to all five persisted configs while General retained usable backing; restored 100% |
| Floating | Applied on/off to all five widgets; native TOPMOST tests cover the actual style changes |
| Snap toggle | Off restores separate outlines/shadows; on restores automatic joining |
| Music resize | Width 560 → 336 DIP, height stayed 480 through later repaints; saved size was 336 × 480 |
| Notes resize | Height 665 → minimum 420 DIP; settings controls and board remain usable |
| Dock Columns | 8 → 7 → 8; position and size did not change |
| Clock settings | Face and Glass & window pages opened at minimum size; return to face worked |
| Notes migration | All three existing notes loaded after legacy color fix; original source remained untouched |
| Notes editor | Opened existing empty note and closed with Done without entering/deleting content |
| Three-widget fuse | Clock joined Calendar + Dock into one smooth outline |
| Independent detach | Clock moved left alone; Calendar and Dock stayed fixed and connected |
| Independent fused resize | Calendar height 536 → 433 DIP from top edge; bottom connection to Dock remained |
| Music browser | Browse opened real YouTube Music in its owned player frame |
| Music playback | Anonymous quick pick played; native title, artist, artwork, duration and progress updated |
| Music Pause / Seek | Native Pause worked; seek while paused moved approximately 0:49 → 2:45 |
| Music queue | Queue selection routed to another item and its preroll ad; unsupported ad controls dimmed; playback paused after QA |
| Restart bounds | Dock retained 573 × 396 DIP after fixing fractional-DPI rounding |

The early sandbox instance could not expose interactive windows/tray. It was
replaced by the authorized interactive source run; those early discovery results
were not counted as desktop QA.

## Bugs found and fixed during integration

1. **Legacy Notes could not open.** Rust validation lacked Electron's `slate`
   color. Migration now preserves all seven legacy colors and note contents.
2. **Music shrank every paint after resize.** Reentrant `WM_NCCALCSIZE` fell
   through to native thick-frame calculations while state was borrowed. Uploading
   the smaller client bitmap then resized the whole HWND again. Stateless NC
   responses now precede state borrowing; callback-capable Win32 forwarding drops
   the borrow. Music and Notes have repeated-real-paint regressions.
3. **Native resize callbacks were skipped.** Holding `RefMut` through default
   Win32 move/resize loops blocked nested size/minimum/end-move handling. Related
   widget/editor/settings paths were audited and fixed; final bounds persist.
4. **Hide all hid its own recovery UI.** General is now independent of Dock's
   HWND ownership while commands still target Dock. A native regression verifies
   that hiding the command target does not hide the manager.
5. **Floating widgets could cover General controls.** Explicit management actions
   restore manager stacking; selecting widget Settings hides General. The shared
   decorative fuse surface sits below cluster members and topmost controllers.
6. **Fused shadows had rectangular holes.** Transparent rounded corners inside
   HWND bounds now receive the shared shadow instead of cutting a square hole.
7. **Translucent seams could double opacity.** Overlapping HWND margins now have
   one background owner. Real BGRA outputs are composited in a three-window
   straight/T-seam regression at 0%, 25%, 50% and 100% opacity.
8. **Failed bridge rendering could suppress standalone chrome.** Fusion coverage
   and shadow suppression commit only after a successful upload. Simulated surface
   failure falls back to independent rounded chrome and shadows, then recovers.
9. **Unrelated changes rebuilt every fused background.** Stable components reuse
   bridge HWNDs/uploads and component-derived cache revisions. A distant member
   change is checked to preserve the existing bridge, upload count and revision.
10. **Mixed-DPI snap acceptance disagreed with adjacency.** Pairwise overlap
    eligibility and post-snap adjacency validation now agree; material seam
    orientation uses the same pair DPI tolerance.
11. **Dock lost a DIP on restart.** Physical sizing and saved DIP bounds now
    round consistently instead of truncating at fractional DPI.

## Snap/fuse contract

- Default on; toggle in General Settings, persisted in `native-general.json`.
- Magnetic edge range about 12 DIP; require a meaningful shared edge rather than
  touching corners. Only widgets on the same desktop/floating layer join.
- Connected components support chains and T junctions, not only pairs.
- Each widget retains its own HWND, content, theme, data, position and size.
- Shared smooth-union coverage removes internal rims and adds a concave glass
  junction. Nearby backdrop materials blend to a common wallpaper sample.
- Each widget can be dragged away independently and resized independently.
  Saved per-widget bounds reconstruct touching components on restart.
- Settings, editors and the Music browser frame never become snap targets.
- Frost stays cached during movement; shape/backdrop connections settle on release.
- Bridge CPU buffers are temporary; unchanged native surfaces and cache revisions
  are reused. Over-budget components (16 million bridge pixels) and failed uploads
  retain independent chrome rather than leaving a partial fused state.

## Limits / follow-up

- Live mixed-DPI monitor crossing was not exercised; geometry/native callback
  regressions cover DPI cases. Test on hardware with different monitor scales.
- Full old-app theme/animation parity is not claimed. Liquid merge/split morph
  animation, whole-cluster dragging/resizing and arbitrary duplicate widget
  instances are not implemented. Independent member manipulation is implemented.
- Sign-in/account migration was not automated. Native Music uses an isolated
  browser profile; no cookies are copied. Browser/Node memory is additional to
  the native host when Music playback is active.
- Single-track Next/Previous behavior depends on the page queue/ad state. Mock
  adapter tests cover routing; live playback, pause, seek and queue selection were
  exercised, including disabled controls during an ad. Do not promise every
  account/playlist/region combination has been tested.
- Keyboard/UI Automation coverage of custom widget controls is still uneven.
  Notes uses a separate native RichEdit editor. Existing handoffs retain detailed
  per-widget parity/accessibility limitations.
- Active-QA memory/CPU logs include rendering, interaction and browser startup;
  they are not an idle benchmark or a fixed five-widget memory budget.

QA changed appearance/layout preferences only temporarily; original existing
Dock/Clock/Calendar geometry/themes were restored, along with full opacity and
the default native Music layout. All note/app content and original Electron
state files were preserved. No installer or release output was generated.


## 2026-11-10 follow-up — source changes, desktop verification pending

- User reports supersede the earlier visual pass: an aligned fused edge pinched inward; the concave connector appeared jagged; the shared background transition was too narrow.
- Fusion geometry now fills facing aligned rounded corners to their tangent points and bounds the union at its exterior limits. Concave connectors retain the smooth union. Shared member/bridge edge coverage uses 8×8 physical-pixel sampling near the contour.
- Optical background blending expands from 56/24 DIP to 140/80 DIP (normal/tangential), with a quintic smooth falloff and the same screen-coordinate wallpaper at the seam.
- Dock compact minimum is 240×180 DIP. Below 440 DIP the toolbar uses a separate row; search/clear/native Edit retain their own width and label fonts stay unchanged. Config/manual/native minimums share DOCK_MIN.
- Isolated source geometry harness: 18 passes (fusion11 + Dock geometry7), including all four edge rotations at DPI1/1.25/1.75/2, fractional concave coverage, broad optical falloff and compact header hit targets. Safety guard passes.
- Full crate build/reload and live visual QA are pending while the four widget chats finish their settings/header/legacy fidelity edits. Do not interpret the isolated geometry run as a full application or visual pass.


### Follow-up verification and crash fix

- The user-reported Application Error named the Cargo test binary `lumen_native_dock-0114aacfc302aa34.exe`, not the running source app. Music's repeated-layered-paint test completed its paints/Host destruction and crashed with0xC0000005 during renderer/thread teardown.
- Root cause: shared header TLS cached GDI+ Image handles beyond `render::Runtime` shutdown. The shared kit now caches bounded plain pixel data, creating/dropping the native image within the active renderer lifetime; it also uses the proper Dock gear SVG. Added successive-renderer-session regression.
- Root serialized targeted Music regression: pass, process exit0. Full native suite after the fix:117 passed,4 ignored, exit0,12.29s. TypeScript typecheck,6 safety regressions and safety guard pass. Parallel chat-owned HWND test runs were paused; no crash result was treated as a test pass.
- Local debug source build succeeded. Root launched inspect PID19852 and observed common headers across all5 widgets, restored Notes pastel/bead/pin/trash/footer styling, and wider fused contour/background. Music and Notes settings each opened as separate glass windows and closed without replacing/resizing their widget. Music player was not loaded after restart; populated Coverflow artwork/playback fidelity is not claimed visually verified in this follow-up.
- Inspect app exited cleanly (launcher exit0). Reopened the same source debug build in ordinary tray mode, with all widgets; no installer/release output. Did not modify note content, theme selection or widget geometry for these visual checks. Full per-control live checks, compact edge drags and mixed-monitor fusion remain covered by native regressions but still benefit from further desktop QA.


## 2026-11-10 circular fusion outline and resize snapping

- Replaced polynomial smooth-union plus corner patches with a cached orthogonal rectangle-union contour. Collinear edges merge before rounding; convex and concave vertices use exact quarter circles with one component radius, clamped only by adjacent segment length. Near-aligned coordinates normalize within the existing1.5-DPI snap tolerance. Geometry cache owns only plain data.
- All fused member coverage is recalculated from that common outline, including previously opaque edge pixels. Bridge and member bevels now use the same directional lighting/band, avoiding a different highlight on the concave fill. Physical-pixel8×8 edge coverage is retained.
- Resize magnets target neighbor left/right/top/bottom edges within12 DIP. Only grabbed edges move; opposite anchors and widget-local minimums remain fixed. Native WM_SIZING uses the shared adapter; captured Dock/Calendar resizers invoke the same helper. Settings/editor are excluded. General Snap & fuse toggle disables both move and resize snapping.
- Regression coverage: exact concave/convex circle samples across1/1.25/1.75/2 DPI; fully aligned union equals one rounded rectangle; straight local edges remain straight when a third member extends the component bounds; near alignment; resize corners/minimum/layer separation/negative coordinates; actual test-owned HWND WM_SIZING and disabled behavior.
- Full native suite:125 passed,4 ignored, exit0. Safety guard passes. One newly added Music test expected5 appearance sliders although its schema has4; corrected it to verify the four required slider IDs rather than an incorrect row count.
- Source inspect build showed the new circular contour in the existing multi-widget layout and exited cleanly. Final build reopened all widgets in normal tray mode. Resize snapping is verified by pure geometry/native message regressions; exhaustive manual edge dragging on every monitor was not repeated in this follow-up. No user content or theme was edited.


## 2026-11-10 shared settings/close hover reveal

- Clock/Calendar/Music/Notes use the same16-DIP SVG gear/close strokes and32-DIP hit boxes. Shared drawing matches Dock toolbar's round hover material, resting/hover alpha and600ms rotation. Title stays visible.
- Added app-local widget_header controller:180ms reveal,240ms hide, continuous retargeting, release-based existing actions and invisible-button input guard. Precise animation timer stops when settled; native-child leave tracking uses a cheap50ms pointer check only while needed, without idle redraw. Hide/destroy removes timers and per-HWND state.
- Preview exposed missing startup/title enter/leave tracking and the risk of global WindowFromPoint checks overriding delivered mouse events around fused/captured windows. Final controller trusts client/nonclient message coordinates during pointer movement, registers leave tracking when initially under the cursor, and checks global pointer only for startup/leave/child transitions. Temporary pointer diagnostics were removed.
- Added continuous-retarget/settle regression plus a test-owned HWND lifecycle regression, including delivered WM_MOUSEMOVE remaining authoritative across paints and cleanup on hide/destroy. Full final suite:132passed,5ignored, exit0; safety guard passes. Source build succeeded and all widgets reopened in ordinary tray mode (PID14996 at this snapshot).
- Live previews checked hidden header states and informed the pointer-tracking corrections. Final pointer-event behavior is covered by the native HWND regression; this entry does not claim an exhaustive manual hover pass on every theme/monitor.


## Latest compact-title update — 2026-11-10

- Shared widget title now12 DIP Bold (supersedes Light16), visible only at420x360 DIP or larger. Music chrome label is `YouTube Music`.
- Compact layouts reclaim34–40 DIP of the former title band. Notes Add/search and viewport move up with the top-right controls reserved. Calendar uses compact date strips and wraps small Lumen navigation so it does not collide with hover controls. Music compact/gallery layouts move up; header draws last above artwork. Clock compact face and emblem hit region use the same content inset. Saved widget bounds are unchanged.
- Existing Calendar layout regression now checks2D disjoint navigation and header hit targets, supporting the intentional two-row compact navigation. Full suite141 passed/5 ignored, exit0; safety guard passes. No new mirroring tests added for the typography change.
- Root coordinated source/reload ownership with the four chats. Clock confirmed PID23872 was idle and desktop QA released; root reopened the final source build in ordinary tray mode. Native render/HWND tests cover compact layouts; this entry does not claim exhaustive manual QA on all monitor combinations.

## Shared gradient surfaces and custom color wheel — 2026-11-10

- Added one shared surface library for all five widgets: Afterglow, Dawn, Dusk, Dust, Aurora, Ocean, Rose, Ember, Mist, Midnight, Wallpaper Auto and Custom color. Gradients have actual preview swatches; bounded pixel caches keep native image handles inside renderer lifetime.
- Flat legacy palette IDs migrate to equivalent custom hex colors. Gradient/custom choices persist independently of widget content themes, note colors and saved bounds. Managed surfaces replace the old flat tint pass, avoiding double application; wallpaper-auto behavior remains intact.
- Added app-owned HSV wheel with saturation/value square, hex preview and release-based Apply/Cancel. The picker disables only its settings owner; caller state borrows are released across its modal loop and owner identity is checked after return.
- Full serialized native suite:160 passed,0 failed,6 ignored,exit0. Safety guard passed. Source debug build passed after final picker styling adjustment. Tests cover migration/persistence/render adapters, picker Apply/Cancel/owner lifecycle and repeated renderer teardown.
- Live inspect QA verified gradient swatches, opened the wheel, changed its SV selection and cancelled without modifying Wallpaper Auto; reopened and applied Custom, confirmed `custom:#403898` persisted; selected Afterglow and confirmed its key; restored original Wallpaper Auto through UI. Tint18%,opacity100% and widget bounds were preserved. This is not an exhaustive manual theme-by-theme pass for every widget.
- Final source build reopened in ordinary tray mode with all five widgets, PID33772; tray registration and all widget startup logs succeeded. No packaged/distributable output was created.

## Native source packages, theme cleanup and tiny layouts — 2026-11-10

- All six packages now register native providers from widgets/<id>/widget.json and their own native source. The manager discovers the manifests, uses visibility toggles with separate Settings, and persists visibility by stable ID. Control Center's UI and device backend run in Rust; no Electron companion is required. Launch Lumen.pyw builds changed local native source when closed and summons an existing instance when running.
- Calendar9/Music7/Notes4 theme materials fill the complete chrome, with one compositor-owned outer mask/bevel. Removed inset month-panel/material rims and fixed Orbit artwork clearance and compact Calendar navigation. Reviewed native render artifacts for those themes. Notes refined-theme capture rerun includes160×160,160×320 and320×160 plus larger sizes for all4 designs.
- Minimum DIP bounds: Dock180×120, Clock120×120, Calendar168×190, Music168×104, Notes160×160, Control Center160×112. Core content and actions adapt; auxiliary labels/details hide and return on enlargement. Dock grouping retains its Done/Cancel band in compact mode. Notes Add is36/28 DIP with Music-equivalent glass lens/reflection.
- Serialized native suite after tiny integration:185 passed,0 failed,7 ignored, exit0. Notes hidden render capture passed separately. TypeScript typecheck and safe-mode guard passed. Native Rename Apply/Cancel/render regression passed after switching its native EDIT parent to regular GDI composition, avoiding layered-child rendering incompatibility.
- Live Rust Control Center, detached Settings and Edit controls inspected; no actual audio/display/HDR writes exercised. Read-only device diagnostic previously returned1 display,3 outputs,3 inputs and0 errors. A complete live Rename interaction and exhaustive monitor/hardware matrix are not claimed; its lifecycle is covered by the hidden native regression.

## Hover starvation / cursor and drag fix — 2026-11-10

- Reproduced a shared header event feedback loop at ordinary sizes: non-client caption movement incorrectly armed CLIENT TME_LEAVE. Each immediate WM_MOUSELEAVE then rearmed the same request while the cursor was still over the caption. Temporary local profiling produced a flood of repeated hit tests with a stationary pointer, starving input and painting. Removed diagnostic logging after diagnosis.
- Register TME_NONCLIENT for WM_NCMOUSEMOVE, handle WM_NCMOUSELEAVE, and arm tracking only from actual movement. Leave/timer/capture/exit messages never rearm leave tracking. Preserves animated reveal and idle timer shutdown.
- Added regression for client/non-client flags and all non-movement messages that must never restart leave tracking. Full serialized suite187 passed,0 failed,7 ignored; safe-mode guard passed.
- Source preview PID21408 loaded the fix. Live Clock Settings opened; normal pointer/header reveal observed, title-area drag moved the window and was returned to its original position; right-edge shrink worked directly without first enlarging. User resumed Music Settings interaction, so further automated desktop input stopped. This is not a claim of exhaustive per-widget resize/monitor profiling; no user content or hardware setting was changed.

## Dock-style rounded-corner resize — 2026-11-10

- All5 non-Dock packages now share geometry::widget_edge:10-DIP edge band and40-DIP diagonal corner region matching Dock. This reaches the visible corner arc at roughly14×14 DIP, where the former thin rectangular edge tests returned caption/client.
- Notes keeps interior Add/footer targets but lets its outer4-DIP curved rim resize. No minimum size, snapping or saved geometry changed.
- Added fractional-DPI/corner/central-content checks and actual WM_NCHITTEST assertions for four arcs on test-owned Clock, Calendar, Music, Notes and Control Center HWNDs. Full native suite188 passed,0 failed,7 ignored, exit0; safe guard passed. This validates real hit dispatch without moving the user's widgets during QA.

## Immediate corner-mask and shadow geometry refresh — 2026-11-10

- Shaped Buffer reuse now includes radius as well as size/DPI. Each main widget registers its current geometry with fusion before calculating background cache keys, avoiding a first-frame key made from the previous/default contour.
- Added radius-aware WindowShadow sync. Notes and Control Center pass their30-DIP chrome contour; Calendar passes its layout radius; Dock/Clock/Music retain34 DIP. A radius change at identical bounds invalidates both the alpha template and upload. The shadow's internal hole now follows the same contour as the glass instead of an unrelated34-DIP preset.
- Native regression switches34→30 DIP at unchanged bounds across100%,125%,175%,200% DPI and checks the updated shadow equals a freshly created one. Checks the shaped alpha cut changes as well. Clock Lumen startup render and a completely fresh repaint have identical alpha cuts without resizing; reviewed .scratch/clock-corner-review/corner-first-lumen.png.
- An initial full-suite run caught a separate Music queue-hit overflow introduced by its proportional layout work. Coordinated the fix with the existing Music chat; root retained ownership of native tests/reload. No user content or saved geometry edited for this change.
- Final serialized native suite after integration:197 passed,0 failed,7 ignored, exit0 (25.89s). Safe-mode guard passed. Reloaded from the local Rust source launcher in ordinary tray mode.

## Fuse theme-to-theme color blending — 2026-11-11

- Replaced the wallpaper-only intermediary in bevel_for/shared_material with a common field sampled from the widgets' raw backgrounds. Neighbor colors and gradients cross-fade into each other; bridge fill samples the same field. Alpha is untouched by the material blend, so Background opacity remains explicit user control.
- Capture happens before blend/bevel/content/opacity. Each member retains at most64×64 RGB samples (12 KiB); equality suppresses feedback. Material changes invalidate the component's background/bridge cache, and destroy releases the tile. A16-physical-pixel globally aligned field keeps samples continuous across windows and avoids per-raster-pixel exponential evaluation/full-resolution material-cache copies.
- New pure regressions verify red/blue mixture at the seam, continuity, bilinear gradient sampling and edge extension. Native regression renders two actual fused layered HWNDs with saturated red/blue backgrounds: the join contains both colors, no green/wallpaper wash and alpha255. Three repaint cycles keep a stable revision rather than accumulating blend or scheduling a feedback loop. Existing chain/T-junction, opacity and detach tests remain passing.
- Full serialized suite:202 passed,0 failed,7 ignored, exit0 (42.18s). Safety guard passed. No desktop capture, user bounds/theme changes or installer output needed for QA.

## Independent button backgrounds — 2026-11-11

- Added retain_control_backing: fill only missing premultiplied alpha underneath already-rendered controls using the existing opaque backdrop. It preserves text/icons/reflections and press states, rounded coverage and DC clip. Fully opaque pixels are skipped; no per-control bitmap allocation.
- Wired Control Center tiles/Edit/Add and viewport clipping, Music's painted lens rectangles (including disabled controls and proportional scaling), Notes Add/hover toolbar, Calendar Today/selected/hover and the common header hover material. Header hover/reveal still fades and idle headers gain no permanent opaque rectangle. Existing search/group/card backing remains intact.
- Pixel regression compares100/50/0% background opacity with an opaque reference, verifies glyph pixels unchanged, rounded corners and empty chrome retain their original alpha, and premultiplied color stays valid. Native Control Center regression checks actual tile backing/colors across all4 designs at100/50/0%, while empty chrome remains transparent. Safety guard passes.
- Final serialized suite:204 passed,0 failed,7 ignored,exit0 (42.71s). Reloaded the local Rust source app in normal tray mode.

## Smooth fused spotlight sampling — 2026-11-11

- Fusion background capture now area-averages each source region instead of selecting one grain pixel. A small separable positive Gaussian kernel smooths localized caustics before enlargement. Original widget grain/spotlight and content rendering remain intact; only the shared fusion material is filtered.
- The globally aligned blend field now retains floating-point BGR until final rasterization. This removes a redundant8-bit rounding step which amplified steps/cells in broad highlights. Retained source storage remains at most12 KiB/member.
- Regressions verify phase-locked checkerboard grain averages to the correct constant color, a bright impulse becomes a monotone soft peak without ringing, fractional lighting values survive the field, and saturated widget-to-widget color/opaque alpha tests continue passing.
- Full serialized native suite210 passed,0 failed,7 ignored,exit0 (45.87s); safety guard passes. Reloaded local native source in normal tray mode. No user content, theme or bounds modified for this fix.

## Live Music seam diagnosis — 2026-11-11
- Inspected user's actual desktop using computer-use. Clock pane ended at2041 and Music began at2033:8 physical pixels overlap at175% DPI. They fused indirectly through lower widgets but failed direct material adjacency. Earlier coating change did not fix this cause.
- Direct-neighbor detection now accepts small HWND-margin overlap, while rejecting visible gaps. Material blending uses the same contact predicate and the midpoint of overlapped edges. Added regression with those exact saved coordinates.
- Fusion tests23 passed plus exact-layout regression passed. Live before/after inspection confirmed the hard vertical seam became a continuous color transition. Saved positions and sizes preserved.


### 2026-11-11: tray, package manager and login startup
- Tray menu now has exactly two entries: General Settings / Widget Manager and Quit Lumen.
- Package-discovered manager cards have lightweight native illustrated previews, visibility toggles, and separate Settings targets. Manager native-buffer preview visually inspected at .scratch/manager-preview.png.
- Full serialized Rust suite: 213 passed, 7 ignored; shared settings shell render/scroll/dispatch/multiple-DPI and renderer-cache lifetime tests included. Subsequent manager render/config tests passed after final preview changes.
- Python startup tests: 3 passed (first-run on, saved off, quoted paths, corrupt preferences preserved). Real HKCU Lumen startup value disable/enable/readback verified; left enabled. Source-launch registration is idempotent.
- Removed floating-window UI controls across all six widget packages and General; normal host ignores legacy topmost preference.
- Safe-mode source guard passed. Desktop automation inventory did not expose the native tool windows; visual QA used the app-owned render buffer, not a claim of completed mouse-driven UI QA.


### 2026-11-11: public native portable release and updater
- Source and all six widget source packages published to https://github.com/nhatnguyends/Lumen (dedicated checkout; parent CustomTools remote preserved). Excluded user config, browser profiles, build outputs, attachments and local handoffs.
- v0.1.0 Windows x64 portable ZIP and source ZIP published by a passing Windows GitHub Actions pipeline: https://github.com/nhatnguyends/Lumen/actions/runs/42968903938.
- Full local Rust suite passed: 219 passed / 8 ignored before the additional update-UI test; the update-UI test and all General tests then passed. 11 JS safety/Music tests and 3 Python startup tests passed. GitHub ran the complete updated native suite successfully.
- Static-CRT portable PE imports inspected: Lumen, bundled Node, and Koffi use Windows system DLLs; no separate VC++ runtime detected. Bundled Node license and Rust/Koffi dependency notices included.
- Relocated ZIP with spaces in its path, removed developer tools from PATH, smoke-started all six widgets, loaded bundled Koffi against user32.dll and started/stopped Music JSON bridge successfully. Smoke instances use a private mutex and skip startup registration.
- Isolated real helper test waited for a live portable parent to exit, replaced files and restarted; config fixture remained intact. An initial automatic-handoff test did not complete in its timeout and was investigated with the explicit live-parent helper test rather than counted as passing.
- Opt-in Rust integration test live_portable_stage_and_apply passed against the actual published GitHub ZIP: native HTTPS, digest verification, archive validation, staging, apply, previous-file backup and config preservation. Published test on main in 9e4b377.
- Portable ZIP target: Windows 10/11 x64, writable user folder. Browser playback requires Chrome or Edge. ARM64 native / 32-bit and independently installable binary widget plugins are not implemented. Release is unsigned.


### 2026-11-11: Lumen name, compact settings and desktop band
- Renamed the Cargo app target to lumen; source launcher and portable packager use lumen.exe. Dialog and tray branding now say Lumen. Existing classes/config identities remain compatible.
- Two-column manager preview/hit geometry verified by native render at .scratch/manager-two-columns.png. Odd grid tails and narrow single-column fallback tested. Removed global show/hide and opacity actions.
- Added desktop-layer HWND subclass for widgets and fused surfaces. Native stacking test rejects TOPMOST promotion and preserves bridge ordering behind a member. Keyboard activation remains available for Dock search.
- Full serialized native suite passed after updating an obsolete Music assertion to allow its newly added sign-in button (appearance controls still verified as sliders). Source safety and 13 JS safety/Music tests passed. New desktop-layer test also passed after the registration no-op optimization.
- Gracefully quit old source instance through the app command and started the renamed source build. Verified process lumen.exe at native-dock/target/debug/lumen.exe; old lumen-native-dock process no longer running.
- Refreshed and verified local portable ZIP only. No Git commit/push/tag/release performed; publishing remains on hold.

### 2026-11-11: shadow stacking during movement and icon reset
- Removed Win32 ownership from shadow decoration HWNDs: owned popups are forced above their owner. Shadows now follow the owner stacking band and desktop guard on every move/sync, including cache reuse.
- Native regression moves a widget/shadow four times and verifies no owner, no topmost flag, immediate ordering behind the widget, and ordering below an application. Full serialized suite: 225 passed, 9 ignored.
- App Dock now offers Reset icon for items with an override. It clears the override plus image, scaled-image, tile and shadow caches, saves configuration, and resolves the original target icon. Launcher tests passed after this addition.
- Restarted source app successfully: native-dock-ready and all enabled widget startup messages observed. Local changes only; publishing remains paused.

### 2026-11-11: optional Desktop synchronization
- Added Sync Desktop items toggle to App Dock settings, default off, persisted in native dock config. Uses Desktop/Public Desktop Known Folder APIs for redirected locations. Hidden/system entries excluded; only immediate children mirrored.
- Native FindFirst/NextChangeNotification handles are checked with zero timeout on the existing 1-second timer. No background thread or idle directory scanning. Handles close when disabled; managed items removed without touching Desktop files or manually pinned entries.
- Reconciliation keeps existing icon caches and defers during window/icon movement. Tested real file add/rename/delete notifications, rearming, unchanged snapshot and preservation of manual pins. Full serialized suite: 227 passed, 9 ignored; source safety check passed.

### 2026-11-11: fuse join responsiveness
- Added explicit ignored native timing diagnostic profile_large_join_and_detach using two 900x650 windows. Baseline bridge refresh 192.82 ms, optimized 72.93 ms on this machine (~62% reduction). This measures bridge creation, not complete interactive frame latency. Detach topology update 0.17 ms; widget repaint cost is separate.
- Bridge raster skips conservative covered member interiors, and antialiasing reuses the contour distance. Member coverage gets the same interior fast path. Exact fractional-DPI coverage regression compares optimized coverage to contour evaluation at 1x/1.25x/1.5x/2x; curved edges retain original AA.
