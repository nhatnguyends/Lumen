# Native Music handoff

## Files and scope

- `src/music/{mod,model,view,backend}.rs`: Rust/Win32 UI, independent config,
  shared Glass/Buffer/Canvas, desktop/floating, shadow, edge resize, frozen frost
  while moving, global contrast, hide/reopen, browser frame and JSON adapter.
- `src/main.rs`, `src/window.rs`, `src/tray.rs`: module, Music toolbar,
  tray command 5, `--music`/saved-open startup and shutdown. Shared files were
  edited before the coordinator requested a freeze; preserve other chats' work.
- `../src/main/nativeMusicBridge.ts`: existing MusicPlayer/CDP/window mounting
  without Electron; serial controls, frame resize, quit/EOF, cached artwork.
- `../scripts/build-native-music.mjs`, `../run_native_dock.py`: adapter bundle to
  `target/debug/music-bridge.cjs`, launcher builds it and adds `--music`.
- `../scripts/test-native-music.mjs`, `../package.json`: two protocol tests added
  to `npm run test:music`. `../docs/youtube-music.md` documents operation.

## Behavior

Coverflow: centered cover and angled queue covers. Compact: small art and track
details. Sleeve: large centered art. Turntable: vinyl/tonearm and cover label.
All use actual metadata/art, timeline, previous/toggle/next and one Browse button.
Playlist rows select tracks; wheel scrolls. Release-inside activates controls;
lost capture cancels. Opacity affects chrome; the deck retains its own backing.
Design, opacity, tint and floating settings retain bounds. Font sizes stay in DIP
across resize; pixel rasterization follows DPI rather than stretching text.

`config/native-music.json` is independent. Imports only Electron design/opacity
on first use. `config/native-music-player/` isolates the browser profile to avoid
conflicts with the Electron player; first native use requires sign-in. No cookies
are copied. Widget close hides; browser close minimizes; dock quit sends EOF/quit.

On hide, Music releases frame/backdrop/overlay/shadow/artwork buffers and pointer
state. Without a player, its timer stops. With live playback, only a slow 1.5s
metadata drain remains (no drawing or image decoding); this keeps the pipe drained
and playback state current. Show restores art from cached paths and normal ticks.

Coordinator integration API: `Host::hwnd()`, `is_visible()`, `hide()`,
`show_settings()`, `refresh()`, `apply_common(opacity, floating)` and `show()`. General Settings/status bar,
widget management and snap/fuse are assigned to the coordinator. No fusion code
or further shared host/compositor modifications were made after that instruction.
Music now calls `background.bevel_for(self.hwnd)` and compares
`crate::fusion::revision(self.hwnd)` against its cached fusion revision before
reusing the background. The latest targeted tests compile with that shared API.

## Validation actually run

- `python run_native_dock.py --check` passed at an integration snapshot:
  80 Rust tests passed, 3 ignored, six guard regressions and debug build passed.
  Other chats continued editing afterward; this is not a final review of theirs.
- Latest `npm run typecheck`: passed.
- Latest `npm run test:music`: 18 passed (16 existing, 2 adapter tests).
- Targeted Cargo `test --locked --manifest-path native-dock/Cargo.toml music::`:
  seven passed after the fusion API and resize fix landed; renders every design offscreen at
  320x240, 320x340, 560x480, 1000x920, queue on/off, checks control bounds,
  playback/Browse overlap, transparent outer chrome and opaque deck backing.
  Includes config import/round-trip/invalid-data preservation regression.
- `node scripts/check-safe.mjs`, `node scripts/build-native-music.mjs`: passed.

## Pending coordinated QA / limitations

No live Music window, sign-in or playback was exercised. UI discovery was
read-only and found no visible Lumen window. Before the coordinator's no-restart
request arrived, an existing dev dock process was stopped once to unblock the
locked binary for check/build. This task did not launch a dock afterward.

Coordinate one `python run_native_dock.py --music --inspect` run: user sign-in,
real covers, queue selection, seek, ad-disabled controls, hide/play/reopen,
browser minimize/reopen, resize its child, dock quit cleanup, wallpaper/DPI and
zero-opacity input. Mock adapter tests prove routing, not real browser parenting.

This is a Rust UI with a Node playback adapter. Coverflow slide animation, rotating
vinyl, related-radio fallback, keyboard queue navigation, full palette/sliders
and disk cache eviction remain unported. JPEG/PNG are supported; WebP is not
enabled in the existing image crate. Surface settings currently cycle values.

Electron `run_lumen.py` remains primary. No release output or installer was made.

## Resize/reentrant Win32 follow-up

The coordinator's live QA found repeated 22px shrinkage per paint and a native
move/resize loop that bypassed min-size/state persistence. Fixed in
`src/music/mod.rs`: `WM_NCCALCSIZE` and erase handling now run before the RefCell
guard, so synchronous layered uploads always keep a full-window client area.
Min/max constraints are likewise callable during reentry. All default forwarding,
`WM_WINDOWPOSCHANGED`, destruction and DPI `SetWindowPos` drop State before native
calls. Cancel messages reach default processing. Show/common settings release
their borrow before native mode changes; the local Floating action defers mode
application through its own message.

Two new hidden test-owned HWND regressions pass: native SetWindowPos/DPI callbacks
remain borrowable, enforce 320x240 DIP constraints and persist final exit bounds;
eight consecutive real layered paints preserve identical outer/client dimensions.
The latest `cargo test ... music::` has 7 passes; safety guard passes. No running
dock was stopped, reloaded or manipulated for this follow-up. Coordinator can
now rebuild/reload and rerun the live edge-drag/resize test.


## 2026-11-10 shared-kit integration blocker (in progress)

Music settings now use settings_ui::Host; header uses the common drawing/hit
helpers. Current targeted render test completes eight real layered paints and
drops its hidden Music Host, then crashes with STATUS_ACCESS_VIOLATION during
Runtime/thread teardown. Repro: cargo test ... music::tests::repeated_layered
-- --nocapture --test-threads=1. Diagnostic stdout confirms both paint and Host
drop completed. Likely shared bug: settings_ui::HEADER_SYMBOLS is TLS-cached
Symbols containing GDI+ Images, while render::Runtime::drop clears renderer TLS
caches then GdiplusShutdown; HEADER_SYMBOLS is still alive and drops afterward.
Please have the kit owner clear that cache before renderer shutdown, or avoid
owning GDI+ Images in this TLS cache. Music has not edited shared source for it.


### Urgent test-process report / pause acknowledged

Last command: python -c "import run_native_dock as n, subprocess; c,e=n.toolchain();
raise SystemExit(subprocess.call([c,'test','--locked','--manifest-path',str(n.MANIFEST),
'music::tests::repeated_layered','--','--nocapture','--test-threads=1'], env=e))"

Last test: repeated_layered_paints_keep_the_native_outer_and_client_size_identical.
Last PID 36420, prior isolated repro PID 14020. Final output:
`music-paint-regression: eight paints completed`;
`music-paint-regression: hidden host released`;
then `exit code: 0xc0000005, STATUS_ACCESS_VIOLATION`.
No live dock was restarted/manipulated. Native/HWND test execution has stopped;
no popup was dismissed. Remaining work uses source analysis/cargo check only
until root authorizes one serialized native run.


## 2026-11-10 READY — detached settings, common header, compact Music, Coverflow

Source edits are complete; no remaining Music implementation items block the
coordinated root reload. Only Music module source and this handoff were edited
for this request. Root owns the debug build/runtime and desktop QA.

Files: `src/music/mod.rs`, `model.rs`, `view.rs`, new `settings.rs`, new
`coverflow.rs`. Existing playback adapter/browser/session code is unchanged.

- `settings::Host` is the shared `crate::settings_ui::Host`, one Option per Music
  Host. No inline settings flag/page, no opacity/tint cycle buttons remain.
  Gear and public `Host::show_settings()` open the same separate window without
  changing Music visibility/config/open state, content, bounds, design or player.
  Shared controls provide sliders/values, toggles, palette cards and design
  choices; Music only supplies rows and applies validated changes.
- Music schema IDs: 101 frosted, 102 tint, 103 strength, 104 blur, 105 opacity,
  106 palette, 201 design, 202 queue, 203 floating. Notify WM_APP+83; timer92
  coalesces slider config writes and blur rebuilds after180ms. Palette choices
  are the shared kit constants. General apply_common synchronizes the open UI.
- Music keeps a lazy PRIVATE Glass variant for its blur/strength. Shared Glass
  is read-only (settings shell/palette/sharp wallpaper). Changing Music blur
  never calls set_blur on the dock/other widgets' source. Local variant, card
  cache and other visual buffers are released on hide; playback still lives.
- Shared draw_widget_header/widget_header_hits draw Music's common title and
  stable Settings/Hide targets, following SETTINGS-UI-CONTRACT.md. Theme content
  type remains independent. No private header/control copy was added.
- Minimum is240x180DIP in model validation and native min-track; saved small
  bounds round-trip. At minimum, thumbnail/title, seek/times, previous/play/next
  and Browse stay available. Queue uses the metadata area as a one-row scroll
  list below232DIP; taller compact layouts show rows below transport. Wheel
  advances one row and restores cached artwork even while paused.
- Compact, Sleeve and Turntable remain distinct. Sleeve keeps centered large art
  and NOW PLAYING copy; Turntable keeps vinyl/tonearm beside metadata with a
  separate control area. The Coverflow deck is not imposed on these designs.

### Screenshot comparison / restored Coverflow composition

Inspected both user images directly:
- Old: C:/Users/Admin/AppData/Local/Temp/codex-clipboard-1717c9f5-73fc-4510-ad46-a5a65a3c0f83.png
- Before this fix: C:/Users/Admin/AppData/Local/Temp/codex-clipboard-c4e371ea-fef7-41fb-bab9-48cd841e3ae7.png

Old image: large overlapping rounded album CARDS, each with cover + title/artist,
center dominant, side cards receding; bottom horizontal pill with transport,
nested now-playing capsule/mini-art/progress, then external/queue icons.
The previous native image instead had a small album strip, a giant standalone
song title, full-width seek and vertically separated transport/Browse.

`coverflow.rs` replaces that composition at normal sizes. Central card width is
35% of widget width (196DIP at560x480), artwork is rounded14DIP inside a20DIP
card, and title/artist sit BELOW each cover. Near/far cards use .91/.78 scale,
.55/.94 card-width offsets, soft Y-perspective and .8/.65 brightness. Cached
premultiplied pixel warps preserve clear outer corners and avoid black fringes.
The bottom pill is80DIP tall, with a nested current-track capsule and progress;
at widths below480 it wraps to116DIP like the legacy CSS. External and queue
icons live on the right of the pill. Very short layouts deliberately use the
compact transport rather than clipping the gallery. Source references were
CoverflowPlayer.tsx, coverflow.css and widget-designs.css.

Updated render matrix can export AFTER previews when root runs it serialized:
set `LUMEN_MUSIC_PREVIEW_DIR` to an output directory and run Cargo filter
`music::tests::all_music_layouts_keep_controls_inside_surface_without_overlap`
with `-- --test-threads=1`. It writes PNGs for all4designs, queue on/off, at100%
and150%DPI and sizes240x180,240x420,900x180,280x300,320x340,560x480,1000x920.
Preview metadata/art are clearly synthetic layout fixtures, not listening data;
the backing is a controlled solid material. Use the preview to compare structure
with the old screenshot, then verify real wallpaper/material and art in live QA.
No AFTER PNG was generated by this chat after the test-execution pause.

### Current validation / root follow-up

Latest `cargo check --tests --locked --manifest-path native-dock/Cargo.toml`:
PASS. Latest `node scripts/check-safe.mjs`: PASS. No packaging/release output.
No HWND executable/test processes were launched after root's explicit pause.
Root reported the shared TLS Image shutdown bug was fixed with a pixel-only
header cache, and an earlier serialized suite passed117/4ignored. That snapshot
predates the final Music schema/layout/fixture changes and is not claimed as
validation of these final tests.

Added/expanded regressions are compiled but await root execution:
- detached settings reuse a different HWND, share the common schema/controls,
  preserve widget geometry/playback, close/Escape safely, and isolate blur;
- compact/full layouts at the sizes/DPI above, real track hit targets/seek,
  playback/Browse separation, independent opaque deck and transparent chrome;
- pure Coverflow proportions/nested deck/wrapping/queue placement;
- model persistence now explicitly round-trips240x180; existing native DPI,
  min-track, final bounds and repeated-paint tests retain the borrow/NCC fixes.

Please run one serialized final Music filter/full suite, export/inspect previews,
then desktop QA settings (gear/manager/tray), sliders/toggles/designs while paused
and playing, min-size drag/restart bounds, queue scroll at240x180, normal
Coverflow vs the old image, and all4designs. No additional root wiring should be
needed beyond the shared settings_ui module already added. Existing Host APIs
show/show_settings/hide/is_visible/hwnd/apply_common/refresh are unchanged.
Animation polish (Coverflow sliding/rotating vinyl), related-radio fallback and
art disk-cache eviction remain previously documented follow-ups.


## 2026-11-10 current UI/glass refinement — awaiting serialized verification

Direct user request: Music glass is too dark vs Dock; refine all4designs using
an iOS UI skill. No installed iOS UI skill was found after searching user,
system and plugin skills; this was disclosed. Apple official HIG materials,
layout and button guidance is the fallback reference (no skill falsely claimed).

New `src/music/chrome.rs` uses Dock's existing pane_overlay/coating/glow methods
in the SAME order, tint/100 normalized exactly once, same light flag,34DIP rim,
same fallback colors and tint*2.55 alpha. Removed the extra direct reflection
pass (Music previously applied24 instead of.24 twice), hard-coded dark palette
coating and the dark-blue Now Playing capsule. Default tint now uses Dock's
Default; existing user config values remain unchanged. Local blur stays isolated.

All4designs now share circular vector transport controls, rounded cached artwork
(Sleeve22DIP; Compact10DIP; vinyl label is circular), quieter glass/card surfaces
and revised spacing/typography. Browse/queue remain reachable at240x180; compact
content uses widget_content_top and reserves the shared header area. Full-size
Compact centers the player instead of leaving it pinned at the top. Coverflow
keeps its distinct card/pill layout with lighter side cards and capsule; Sleeve
keeps large art; Turntable keeps vinyl/tonearm. The common header still draws
last, says YouTube Music, and registers hits last so cards cannot intercept it.

Only Music source is edited. No restart/desktop manipulation or live QA slot is
owned by this chat. Root compact-title changes are preserved. cargo check --tests
has passed during this work; final test/capture execution is pending one root-
coordinated serialized slot after these latest source changes. The141/5ignored
suite reported by root does NOT validate this latest material/UI work.


### Current refinement READY / source freeze for root verification

Source edits are complete. Latest cargo check --tests and safety guard pass.
No native test/capture process has been executed for this latest refinement;
no app restart/desktop work occurred. Root can now take a serialized slot and
run `cargo test --locked --manifest-path native-dock/Cargo.toml music:: --
--test-threads=1`, then the existing ignored capture_music_themes export.

New regression `music_chrome_matches_dock_coating_for_light_dark_and_tint_endpoints`
compares actual cached pixels against Dock's coating order/values for Ocean and
Porcelain, tint0/24/100. Existing size/DPI matrix covers changed theme controls.
Rounded artwork cache is bounded24entries, keyed by image identity/size/DPI and
radius, and released on hide. Production backing and hit-test radius now match
Dock34DIP/inset4. Default tint follows Dock::default; persisted user tint/blur and
bounds remain unchanged. The browser/backend/session source was not changed.

Header remains the shared YouTube Music header, drawn/hit-tested last, large-
only title and compact content-top reclaim. Compact now relocates Queue beside
Browse instead of squeezing metadata under three header actions. A larger
Compact widget centers its player; minimum remains240x180. Sleeve now has a
rounded22DIP cover, left-aligned21DIP semibold title/13DIP artist, secondary open
button and a separate48DIP circular transport. Turntable keeps its own layout
with a circular cover label. Coverflow preserves large perspective cards, uses
quieter shared material and a brighter circular play button; its capsule no
longer has a separate dark-blue fill. All themes use the same vector transport.

iOS references consulted: Apple official HIG materials, layout, buttons. No
installed iOS UI SKILL.md exists in user/system/plugin roots; this was disclosed
in commentary. Do not describe this work as invoking an installed iOS skill.
Apple material guidance used as inspiration while retaining Windows text/input
and Dock glass, independent settings and each Music design's composition.


## Coverflow follow-up COMPLETE — previous-session artwork / glass buttons / icons

Direct user request: keep previous-session cover visible, soften/round buttons
with glass highlights, replace broken Playlist/Open resource icons.

- New src/music/session.rs persists config/native-music-player/last-view.json
  atomically: previous metadata, real queue/thumb associations, artwork manifest,
  last successfully decoded current cover. Reads it BEFORE first frame with live
  capabilities/playing disabled. Loading this display cache starts no browser.
- Keep the last good cover during loading, missing artwork, failed downloads,
  disconnection and hide/reopen. Replace it only once current artwork decodes.
  Metadata/known queue artwork survives incomplete page transitions. The fallback
  uses the actual decoded previous image, including rounded mini-art caches.
- Legacy upgrade without last-view.json uses the newest decodable image already
  in the native player's art cache. That historical format stored no title/URL
  map, so no song title/artist/queue association is invented. It can be a cached
  queue cover; subsequent confirmed sessions persist the exact last current cover.
- Icons: assets/music contains SIX real Lucide SVG resources and complete local
  ISC/Feather notices. New src/music/icons.rs owns bounded per-widget native
  images (no GDI+ Image TLS). Open/Playlist are no longer hand-drawn paths; all
  media controls share the same stroke/cap family and DPI rasterization.
- Coverflow controls have circular soft glass backing, shadow, coating, localized
  top highlight, hover/press and an active Queue tint. Other design composition
  is preserved. Dock-matched outer glass remains unchanged from the prior fix.
- Native adapter prioritizes current art + five visible neighbors before the rest
  of a long queue, so Coverflow at a late playlist index does not wait behind20
  unrelated thumbnails. No playback/browser session control behavior was changed.

Actual validation: latest Music Rust filter13passed,1ignored (capture utility);
Node Music tests19passed; TypeScript check passed; safety guard passed; bridge
bundle rebuilt. New regressions cover restart/cache restore with transport flags
OFF, pending/failed replacement, empty/disconnected state, new successful cover,
hide/reopen and Lucide antialiasing. Targeted native tests ran serialized with
only hidden test-owned HWNDs; no live app was closed/restarted/manipulated.

Fresh native captures inspected:
.scratch/coverflow-refined/music-coverflow.png (clearly marked fixture gallery)
.scratch/coverflow-last-session/music-coverflow.png (real existing cached photo;
unknown legacy metadata is labeled Last session / Cached artwork in capture only).
Capture extension: set LUMEN_MUSIC_CAPTURE_LAST_SESSION=1 along with the existing
LUMEN_MUSIC_CAPTURE_DIR and run the ignored capture_music_themes test. This reads
only local cache files; it starts no browser and writes no user runtime settings.

Latest production source changed: music/{mod,model,view,chrome,session,icons}.rs,
src/main/nativeMusicBridge.ts, scripts/test-native-music.mjs, vendored music SVGs
and licenses. Root needs a coordinated source build/reload to activate these
changes in the user's running instance; this chat did not restart it.


## Sculpted Coverflow glass buttons — user Pinterest reference

Reference examined: .codex-attachments/728546bc-3a84-4e17-b5e6-e37d9009442d/
57d7f811-c18d-4d0a-866f-4786ed378da9/Screenshot_20261110_130309_Pinterest.jpg.
User wants curved translucent depth rather than a flat filled circle.

New music/lenses.rs generates premultiplied, twice-device-DPI lens sprites:
smoky translucent belly, inner lower/right shade, broad curved upper reflection,
one directional antialiased rim and a weak lower caustic. Hover/pressed/active/
disabled are separate material states. Cache is widget-owned,24entries, keyed by
physical diameter, DPI, palette and state; cleared on hide. No GDI+ Image TLS.
media_button uses this ONLY for Coverflow, keeping hit areas/icons/actions.

Fixed the actual missing depth: Canvas::soft_shadow expects0..255 opacity, but
Coverflow passed.12/.22 for buttons (rounded to0). Corrected to12..46 depending
on state, soft6DIP falloff and1.5..3DIP offset. Corresponding previously intended
Coverflow card/deck shadows also used fractions and now use proper alpha units.
The main widget's Dock-matched glass remains untouched.

Validation: targeted Rust Music run14passed,1ignored; new pure-pixel regression
checks directional highlight vs lower edge, translucent belly, clear corners
and valid premultiplied alpha. Updated hidden-window native capture passed;
image inspected: .scratch/coverflow-sculpted-glass/music-coverflow.png. It shows
real legacy cached artwork, no player startup, and no live application restart.
Safety check passed. Only Music source + this handoff changed for this request.

## Three additional Music themes — user references, 2026-11-10

Added Orbit (half photo CD above a milky card), Disc (overlapping CD and
horizontal player), and Sky (blue-to-white album view, now-playing glass pill,
Browse/Library/Queue navigation). Existing four designs remain. Select them in
Music settings / Music design. Switching designs preserves user bounds.

Implementation: music/themes.rs, model/settings/view/chrome/icons/mod.rs,
nativeMusicBridge.ts, two vendored Lucide SVGs and adapter navigation test.
All use the existing playback, seek, queue, cached-cover fallback and sculpted
glass buttons. Sky uses real album metadata when present and track title
otherwise. No fabricated album stats or inactive mock navigation. Library uses
the existing MusicPlayer and browser page, navigating to /library.

Photo discs use cached premultiplied circular masks, metallic concentric hub
and transparent spindle hole; Orbit also masks against the rounded panel.
Small layouts fall back to compact controls. Expanded queues reflow Orbit/Disc
and Sky shows additional real loaded rows where space permits. Shared outer
Dock glass/header and user runtime config are untouched.

Validation: Rust Music 14 passed / 1 capture test ignored in regular runs;
capture test passed separately; JS Music 20 passed; typecheck, safe-mode check,
and native music adapter debug build passed. Geometry tests now include the
three natural reference sizes plus 420x480, at DPI 1/1.5, queue on/off.

Final PNGs: .scratch/music-added-themes/music-{orbit,disc,sky}.png. Captures
use the previous locally cached artwork in a hidden test-owned HWND. Legacy
cache has no metadata, so preview labels explicitly say Last session / Cached
artwork. No real player launched and no live shared app restarted. Coordinated
source build/reload is required to activate themes in the running instance.

## Brighter Music controls — 2026-11-10

User requested brighter buttons across themes. Updated lenses.rs to use a
translucent pearl-white body, stronger curved upper reflection and lower
caustic, with substantially less lower/rim black shading. Reduced control
shadow opacity from 34 to 19 at rest (hover 46 to 28). All seven designs now
share this luminous glass material; icons follow each layout's foreground.
No main background or cover persistence changes.

Rust Music tests: 14 passed, 1 ignored; capture test passed separately;
check-safe passed. Captured all seven themes to .scratch/music-brighter-buttons
and visually inspected Disc and Coverflow on light/dark content. No live app
restart; source reload is still coordinated with the main app work.

## Shared gradient surface adapter — ready for root integration, 2026-11-10

Implemented the published SURFACE-PALETTE-CONTRACT.md in Music-owned files only:
- music/model.rs: normalize persisted legacy palettes before validation;
  validation uses surface::valid, including custom hex, excluding bare picker.
- music/settings.rs: surface::color supplies representative/exact color;
  palette_choices() supplies common choices; palette guard uses surface::valid
  and canonicalizes accepted values. Existing cache invalidation retained.
- music/chrome.rs: surface::paint is invoked only from cached backdrop chrome,
  immediately after flat tint and before coating/overlay/spotlight. Uses tint/100,
  with existing tint=100 when frosting is off. Desktop opacity pipeline unchanged.
- music/mod.rs tests: material reference now includes Afterglow/Dawn/Dusk/Dust
  and custom hex; settings accepts gradients/custom, rejects bare custom/bad hex,
  and invalidates caches. No header/layout/content-theme hooks changed.

Added model test surface_palettes_round_trip_and_legacy_flat_migrates_without_changing_color:
gradient/custom save-load, legacy Ocean/Porcelain/Obsidian equivalence, invalid
picker/hex rejection, design/position/size retention.

Validation so far: Music rustfmt and check-safe passed. At handoff surface.rs
had not yet been published by root; cargo compilation and all test execution
are deferred to the root QA slot. No native test executable/HWND process run,
no runtime restart, no shared source files edited. Ready against contract API.

Contract follow-up integrated: gradient test/config keys now use gradient:
prefix. Music chrome skips the legacy flat tint pass for surface::managed
palettes, then paints exactly once at tint/100 before coating. Material
reference test follows the same managed/legacy split; legacy auto stays intact.
`cargo check --tests --locked` passed against the now-published shared API
(only shared-kit dead-code warnings). Music rustfmt passed. No tests/native
HWND process or app restart run; ready for root serialized QA.

## Music title removed and Coverflow controls inset — 2026-11-10

User screenshot requested no YouTube Music heading and more room around media
buttons. Current source moved to widgets/music/native/. view.rs passes an empty
header title while retaining Settings/Hide and their existing reserved area.
coverflow.rs moves wide left controls inward 12 DIP, right controls inward
16 DIP, and adjusts metadata bounds to keep separation. Narrow decks distribute
five controls with 16 DIP horizontal inset and 10 DIP top inset, wrapping
metadata 6 DIP lower to preserve a gap below controls.

Music rustfmt, cargo check --tests --locked and check-safe passed. No shared
header source changed; no live restart or native HWND test process run under
the existing root QA coordination constraint. Source ready for reload/visual QA.

## Coverflow card bottom edge and broad shadows — 2026-11-10

User screenshot showed an odd cut under tilted cards and requested large
shadows. Changed only widgets/music/native/coverflow.rs. Removed rectangular
soft_shadow knockout behind perspective cards. Each cached card now includes a
large blurred shadow derived from its actual rounded/projected alpha, with
64 DIP scaled padding, 14 DIP blur, 16 DIP downward/4 DIP rightward offset and
46% shadow opacity. Draw expands only the cached image bounds; layout/hit rects
stay at the original card geometry.

Projection now samples pixel centers and transparent bilinear taps outside the
source rather than discarding the final source row, smoothing the bottom slope.
Pure regression covers both lean directions, surviving bottom-row coverage,
antialiased edges, broad shadow below the card, transparent padding and valid
premultiplied alpha. Two pure Coverflow tests passed, cargo check --tests and
check-safe passed. No HWND tests, capture window or live restart performed;
visual QA/reload remains coordinated with root.
