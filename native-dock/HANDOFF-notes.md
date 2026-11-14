# Sticky Notes handoff — 2026-11-09

## Honey / distinct paper materials / shared gradient adapter — 2026-11-10 latest

Source integration ready; root owns the serialized native HWND QA/reload slot.
No runtime restart or HWND tests launched during the gradient integration.

- Added fourth Design `honey` (Honey): amber cover/header, cream content sheet,
  warm-white note cards and stronger first preview line, inspired by the user's
  yellow/cream reference. Existing note/pin/color/edit semantics are preserved.
- Notebook now uses an actual warm paper backdrop, binding/fold, cream page,
  rules and red margin. Index has an opaque deep-green archive backing and
  numbered rows. Pinboard keeps its pastel glass highlights. Theme materials
  rebuild only on backdrop-cache changes; viewport fades/footer remain intact.
- `notes/model.rs` normalizes legacy surface colors to custom hex, validates
  `surface::valid`; `notes/settings.rs` uses shared `palette_choices`, valid and
  color APIs. Managed gradient/custom surfaces skip the previous tint layer
  before calling `surface::paint` in cached backdrop, before coating/bevel.
  Palette is included in backdrop cache identity. Paper/Honey content sheets
  sit above the material so tint100 cannot erase the content-theme identity.
- Model round-trip covers Honey + gradient:afterglow/custom:#7A4D28 and legacy
  Ocean→custom:#1C2E3A while preserving rich note data and negative bounds.
  Settings test covers new theme/material choices and rejects invalid hex/bare
  custom action. Capture test now covers 4 designs and a 2×2 overview.
- `cargo check --locked --tests` passed. Safe-mode guard passed. Pure model
  tests:5 passed/1 deliberately ignored; settings:1 passed. No HWND tests yet.
  Screenshot pass awaits root's serialized QA slot; previous screenshot outputs
  do not include Honey or the new opaque paper materials.
- Root QA capture command when the slot is free: `cargo test --locked
  --manifest-path native-dock/Cargo.toml notes::tests::capture_refined_themes
  -- --ignored --exact --test-threads=1`. This creates hidden HWND only, checks
  invisibility and writes the actual renderer outputs to
  `.scratch/notes-themes-refined/{1-lumen,2-notebook,3-index,4-honey}.png`, matching
  `*-scroll.png`, and `notes-themes.png` (2×2). Synthetic notes only. No live
  native config/theme mutation. Please include this in serialized QA so the
  user's requested theme screenshots can be reviewed.
- Owned files only: `src/notes/{mod,model,settings,editor,typography,theme}.rs`,
  this handoff and NOTES.md. Shared host/settings/render modules untouched.

## Fade / primary add button / footer / theme screenshots — latest

Implemented the user's follow-up requests in `src/notes/mod.rs`:

- Overflowing content now uses the existing compositor's premultiplied
  `fade_list`: smooth upper/lower falloffs (14/26 DIP). Fade strength reveals
  gradually with the amount scrolled; it disappears at a complete first/last
  row. Header/footer/scrollbar and the color picker are not faded. Inline native
  editor snapshots/caret use the same content fade as the cards.
- One cached `list_base` buffer is allocated only for overflow and reused until
  size/DPI changes. It is released when content fits or the board hides. No new
  wallpaper work occurs on each scroll frame.
- New note is now a brighter 44×44 DIP round glass control with a crisp vector
  plus, versus the 32-DIP search height. Hit geometry matches the new circle.
  At narrow widths, focusing search expands it into a full-width row so native
  input remains usable alongside the larger primary control and shared chrome.
- Footer moved inward: text begins at x24, baseline box ends 19 DIP above the
  bottom; Open folder ends x=w−24. Undo and folder hit areas moved with the
  visuals. Content/scrollbar stop above the larger footer safe area.

**User-requested screenshots produced and visually inspected** using the actual
Rust renderer with synthetic notes and a hidden HWND (asserted invisible before
and after each frame); no desktop manipulation, no live data/theme changes,
and no user app stop/restart. This is a deliberate renderer-capture pass for the
new user request, not concurrent desktop QA. Outputs:

- `../.scratch/notes-themes-refined/notes-themes.png` — side-by-side scrolling
  views, left to right Pinboard / Notebook / Index.
- `1-lumen.png`, `2-notebook.png`, `3-index.png` — beginning of each theme.
- `1-lumen-scroll.png`, `2-notebook-scroll.png`, `3-index-scroll.png` — mid-scroll
  frames demonstrating both edge fades, the brighter/larger + and inset footer.

Added an explicit ignored capture case:
`cargo test --locked --manifest-path native-dock/Cargo.toml notes::tests::capture_refined_themes -- --ignored --exact --test-threads=1`.
Last run **passed (1/1, 0.45s)**. It checks both edge pixels return to chrome,
button height dominance, footer folder hit, overflow-buffer release, original
isolated config bytes unchanged and that the HWND never becomes visible.
`cargo check --locked --tests` and `check-safe.mjs` also passed. Capture entry is
ignored by ordinary test runs. No shared renderer/host/fusion changes required.

## iOS-reference refinement — latest 2026-11-10, ready for root reload

User explicitly approved using online iOS references instead of a missing skill.
Reviewed official Apple [Materials HIG](https://developer.apple.com/design/human-interface-guidelines/materials),
[Design principles](https://developer.apple.com/design/human-interface-guidelines/design-principles/),
and [UI Design tips](https://developer.apple.com/design/tips/). Applied their
content hierarchy, legibility, deliberate spacing and restrained material use
to Windows-native Notes; this is an adaptation, not an Apple framework/font port.

- All three designs share a content type scale: **16-DIP regular body**, **11-DIP
  caption/date**, **14-DIP search**, 22-DIP line rhythm (26 for Notebook).
  Inline input insets match preview metrics. The existing shared header and
  root's large/compact title-row logic are untouched.
- Pinboard: retained the recent upper-left/rim highlight; eased color tint from
  88 to 78 alpha so the content material reads as lighter pastel frost.
- Notebook: a quiet warm-paper backing, lower-contrast rules aligned with the
  text leading, a softer margin and more breathing room. Native font remains
  Segoe; existing user RTF formatting is never rewritten for appearance.
- Index: lighter dividers, regular 16-DIP numbering instead of oversized bold
  numbers, consistent body/caption hierarchy and day metadata. Paper/Index hover
  is a subtle fill, avoiding an additional glass rim on every content row.
- Added `src/notes/typography.rs`: a per-State native font measurement cache
  and a bounded 32-entry line-layout cache. Preview wrapping now uses actual
  proportional glyph widths, retains overlong Unicode tokens across lines and
  adds an ellipsis when more lines remain. Measurement DC/font are owned, restored
  before deletion, released on hide/drop; there are no new TLS GDI+ handles,
  per-frame font creation, wallpaper pipelines or dependencies.
- Card restore masks now match the relevant material radii. Notes, RTF, undo,
  theme choices, saved bounds and inline input behavior are preserved.

Validation actually run: `cargo check --locked --tests` **passed**;
`notes::typography::tests::` **2 passed**, including proportional/Unicode wrapping,
native font width measurement at multiple DPIs, ellipsis and resource release.
These tests create **no HWNDs, no GDI+ Runtime and no desktop windows**.
`check-safe.mjs` **passed**. No app restart or desktop manipulation, no live QA
slot owned. Full native/theme screenshot QA remains for the coordinator after
its serialized rebuild/reload. No pending source edits in this pass.

Files in this pass: `src/notes/mod.rs`, `src/notes/editor.rs`, new
`src/notes/typography.rs`, `NOTES.md`, and this report. Shared host/chrome/fusion,
renderer and settings kit were not edited.

## Card glass highlight refinement — 2026-11-10

User noted that legacy note cards had a faint highlight making them read as
glass. In Pinboard cards, moved note tint below `glass_surface` so the reflective
finish and directional rim are no longer muted by the tint painted over them.
Added one thin `coating` pass above that finish to strengthen the soft upper-left
sheen toward the legacy ~9% highlight. This adds no extra border. Notebook and
Index retain their content-specific material. Text, editing, stored notes and
geometry are unchanged.

Only `src/notes/mod.rs` changed for this refinement. `cargo check --locked --tests`
and `check-safe.mjs` passed. No HWND tests, application restart or desktop action;
the running app needs the coordinator's normal rebuild/reload to show the change.

## User requested theme captures — runtime blocked

Captured the live Pinboard window to
`../.scratch/notes-themes-2026-11-10/01-pinboard.png`. Native runtime window IDs
changed repeatedly during capture. After opening Notes settings, the native
windows disappeared from `sky.list_windows`; recovery of the last returned Notes
HWND failed (`window id 658896 was not found`). No further Notes input attempted.
Read-only config metadata still showed `design: lumen` before disappearance;
no note bodies were edited. Notebook/Index captures are not complete.

A source launcher attempt `python run_native_dock.py --no-build --notes` reported
that the native dock was already running. Existing process 14996 (08:24:31) had
an empty MainWindowTitle and no Sky-visible window. The extra launch from this
capture task was process 31768 / Python 7072 (both 08:26:34); only that extra
native process was terminated to unwind its waiting launcher. Existing process
14996 and its Python launcher 35276 were left untouched. No native tests,
packaging, source build or runtime code changes were performed for this capture.
Coordinator/user needs to restore a controllable Notes window before the two
remaining live screenshots can be taken.

## Current ready state — 2026-11-10 (supersedes old prototype limitations below)

**Source ready for the coordinator's one serialized build/reload. No pending
source edits. HWND tests remain paused as requested.**

- **Direct inline editing:** `editor.rs` now creates a `WS_CHILD` host/RichEdit
  inside the actual card or search pill. It creates no top-level Edit note/Find
  notes popup. Text/selection are rendered from the native control into an alpha
  snapshot over the glass; the native input HWND stays at the real text location
  for mouse selection, clipboard, IME and caret anchoring. Formatting lives in
  the card foot; checkmark/Escape commits/closes the inline editor. Stable order
  during typing prevents date sorting from moving the card under the caret.
- **Detached shared settings:** Notes `settings.rs` contains only its schema and
  value application; shell/controls/header use `crate::settings_ui` and its shared
  `SURFACE_PALETTES`, `widget_header_hits`, `draw_widget_header`. Settings gear,
  manager/API reuse the same `Option<settings_ui::Host>`. Sliders cover tint,
  strength, blur and opacity; toggles cover frost/floating; choice cards cover
  palettes/designs. Opening settings keeps the board and inline editor intact.
  Common settings refresh visible Notes controls. Hiding Notes hides settings
  through the shared API to release its surfaces.
- **Scoped appearance:** added backward-compatible `frostStrength`, optional
  `blur`, and `palette` fields. Custom blur creates/updates a Notes-local Glass
  cache; shared Dock Glass is never mutated by a Notes setting. Default source
  remains shared. Existing native JSON gains defaults; Electron files stay
  import-only. Seven legacy note colors/RTF/undo remain supported.
- **240×240 compact mode:** model validation, native min-track, restored size
  and renderer agree. Header is the common 12-DIP semibold/title-case row with
  stable 32-DIP hits. Circular + and recessed search share the next row; a compact
  card retains a native scrollable text viewport and formatting controls;
  board scrollbar and count/Open folder footer remain accessible. Fonts are
  rasterized at native DPI, not stretched label bitmaps; native input zoom
  follows DPI changes.
- **Legacy fidelity comparison inspected:** old
  `codex-clipboard-1717c9f5-73fc-4510-ad46-a5a65a3c0f83.png` vs new
  `codex-clipboard-c4e371ea-fef7-41fb-bab9-48cd841e3ae7.png` from the user's Temp
  attachments, plus legacy Notes CSS/React. Removed giant headline/New note
  button, dark oversized cards, diamond/X actions and Click to edit footers.
  Restored circular +, recessed tinted search/magnifier, ~148-DIP pastel glass
  cards, shaded glass color beads/picker, real outline pin/trash icons, day at
  card foot, count/Open folder footer. Notebook retains paper/rules/margin;
  Index uses numbered compact rows. Common chrome overrides legacy header
  typography as requested without flattening content themes.

### Files and APIs in this update

Only `src/notes/{mod,model,editor,settings,icons}.rs`, `NOTES.md` and this handoff.
No shared host/fusion/main/settings kit edits in this update. Public host API is
unchanged: `hwnd/is_visible/show/hide/show_settings/apply_common`. New local
settings notification is `WM_APP + 104`; inline redraw is `WM_APP + 103`.
Root already added `mod settings_ui`; no further root wiring is needed.

### Validation and exact remaining QA

- Before the coordinator's native-test pause, my actual Notes run returned
  **10 passed, 0 failed, 1 ignored** (1.20 s; exit 0), including real child HWND
  ancestry/no-popup, Vietnamese/emoji + RTF round-trip, inline text autosave,
  stable card order, all three compact designs, separate settings singleton,
  unchanged board bounds/content on settings open, blur cache isolation,
  delete/undo, repeated layered paints and corrupt/source preservation.
- App-owned synthetic renders were saved and visually inspected:
  `.scratch/native-notes-inline.png` (240×240 inline editing) and
  `.scratch/native-notes-preview.png` (normal board). They show actual native
  RichEdit glyphs on the card, no separate editor/opaque rectangle. These PNGs
  predate final small fixes to magnifier/text padding, bead highlight footprint,
  a slightly taller inline text viewport and the restored seven-bead picker.
- Final **`cargo check --locked --tests` passes**; `check-safe.mjs` passes.
  The final small visual/picker/lifecycle changes are typechecked, not rerun
  under native tests after the explicit pause. No runtime restarted, no unknown
  error popup dismissed, no desktop automation performed.
- Root reported the shared crash cause/fix: header TLS Images outlived GDI+
  Runtime; kit now caches plain pixels. Root's serialized suite reportedly
  passed 117/4 ignored. That is coordinator evidence, not an additional run here.
- Coordinator QA still needed after reload: real click/caret/selection/IME on
  the inline child across scroll/DPI; search clearing and formatting; color
  picker, settings sliders/floating, theme-specific details, compact scrolling,
  actual restart persistence and mixed-widget resource/performance profiling.
- Known limits: inactive previews remain plain text and may show missing emoji
  glyphs in shared GDI+ fonts; inline RichEdit preserves/renders Unicode. HTML
  import lists keep visible bullet/number text. Notebook's native text face is
  still shared Segoe rather than the legacy Georgia override. Hover/viewport
  edge-fade polish remains incomplete; do not claim pixel-exact fidelity.

## URGENT native-test crash coordination — 2026-11-10

Stopped launching HWND/native test processes immediately on the coordinator's
warning. No popup dismissed and no live app restart/close performed.

My last executed native command was the workspace-local equivalent of:
`cargo test --locked --manifest-path native-dock/Cargo.toml notes:: -- --test-threads=1`.
It ran `lumen_native_dock-0114aacfc302aa34.exe` and returned **exit code 0**.
Captured output: **10 passed, 0 failed, 1 ignored, 109 filtered out**, test time
**1.20 s**. Last listed running/passing test was
`notes::tests::restored_bounds_preserve_negative_monitors_and_recover_disconnected_positions`.
No crash output, access violation, or Application Error was present in my tool
result. The same executable filename is shared by other chats' cargo tests, so
this does not identify the owner of the popup. Do not infer the popup is harmless
from this successful result. Root owns serialized crash isolation now.

Immediately after that run I viewed app-owned PNG renders and read source/docs;
those calls did not launch any native test process. Continuing source analysis
and `cargo check` only until coordinator releases the native-test freeze.

## Scope / files

- Added `src/notes/mod.rs`, `model.rs`, `editor.rs`, and `NOTES.md`.
- Integrated `mod notes` in `src/main.rs`, an optional Notes host, toolbar button
  2, tray action 7, startup restoration and targeted key routing in
  `src/window.rs`; `--notes` in `../run_native_dock.py`.
- Added Show notes to `src/tray.rs`. While editing that shared action list,
  disambiguated the concurrently added Music/Calendar tray IDs (5/6) and changed
  Calendar's dispatch to 6. Please preserve/reconcile this during integration.
- No Electron notes/state writes, no edits to other widget implementation modules,
  no packaging. `SECURITY.md` was not read, per the user's request.

## Behavior

Shared-process Win32 layered glass board and shared `Rc<RefCell<Glass>>` wallpaper
source; dock compositor/render/shadow reused. Pinboard, Notebook and Index
layouts, search, seven colors, pin sorting, delete/undo, native RichEdit formatting,
plain Unicode paste, autosave/retry/flush, independent migration/persistence,
floating mode, resize/move and smooth wheel/drag scrollbar. See `NOTES.md` for
interaction and storage details.

## Verification actually run

- Full Rust test suite at the first integration checkpoint: **78 passed,
  2 ignored**, before two additional Notes native integration tests were added.
- Notes suite after those additions: **5 passed**, including real RichEdit
  Vietnamese/emoji + formatting round-trip and unchanged-open behavior, all three
  native board designs, search, deletion restore and opacity-zero rendering.
- Saved/visually inspected own test-rendered board at
  `../.scratch/native-notes-preview.png`. This is synthetic test content.
- `node scripts/check-safe.mjs` passed. `cargo check --locked` passed.
- Native host/editor tests briefly opened isolated windows **before** the
  coordinator's no-concurrent-desktop-QA message. No running dock was stopped or
  restarted. Subsequent changes were checked without desktop interaction.

## Remaining QA / known limitations

- Editor is a separate owned native window with an opaque light text surface,
  not inline glass card editing. Search uses an owned native input window.
- Board previews are plain text. HTML list imports preserve visible numbering
  and bullets; new lists are real RichEdit paragraph lists. Emoji can show tofu
  in the shared GDI+ board font; native text round-trip preserves them.
- Hover/scrollbar visibility lacks fade animation; viewport currently clips
  instead of the dock's subtle edge fade. Settings use step buttons, not sliders.
- Sequential manual QA still needed: Vietnamese IME composition, selection/paste,
  lists, multi-DPI moves/resizes, brightness/opacity, z-order/Explorer restart,
  shutdown/restart persistence, scrolling with 400 notes and resource profiling.
- Do not claim the exact old UI or all `design.md` polish is complete.
- The last plain-paste subclass/settings-contrast/search-clear/min-width changes
  need the next coordinated native integration run; final checks here stay
  headless after the coordinator instruction.
- No main executable build/restart requested while shared app is running; use
  `python run_native_dock.py --notes` after coordinating that lifecycle.

## Commands

`python run_native_dock.py --notes` (local dev only); `--check` for the full
launcher checks/build once coordinated. For headless verification use
`cargo check --locked --manifest-path native-dock/Cargo.toml` and
`cargo test --locked --manifest-path native-dock/Cargo.toml notes::model::`.
Workspace-local Cargo/Rustup can be obtained with `run_native_dock.toolchain()`.

## Coordinator follow-up audit

- Added manager entry points: `Host::hwnd()`, `is_visible()`, `hide()`,
  `show_settings()`, `apply_common(opacity: Option<f32>, floating: Option<bool>)`.
  Common settings preserve notes, design and geometry. Coordinator owns wiring
  General Settings/manager and snap/fusion; Notes does not edit shared modules
  for that follow-up.
- Board hide now flushes/destroys its editor, stops animation, settles scrolling
  and releases frame/background/overlay/shadow. Show recreates these lazily.
  Closed editors post an event so the parent destroys their native resources.
  A failed disk save may retain its retry timer intentionally to avoid lost edits.
- Added native-test assertions for hidden buffer/editor/shadow release; compiled
  but not run after the no-desktop-QA instruction. Added and ran a headless
  regression for imported settings, theme/bounds/floating/RTF round-trip and
  unchanged Electron source files.
- Resize minimum now matches settings height (360×420 DIP); maximum track size
  matches persistence limits (1600×1200 DIP). Toolbar minimum width in the native
  editor is 520 DIP so formatting controls remain reachable.
- Settings panel material now follows global ink contrast. Clearing search first
  flushes/closes the owned input to prevent a stale query from reappearing.
- Applied the coordinator's fusion contract in Notes only: main backdrop uses
  `frame.bevel_for(self.hwnd)` and includes `crate::fusion::revision(self.hwnd)`
  in its backdrop cache key. This depends on the shared APIs being added by the
  coordinator. Settings/editor styling stays independent.
- Final headless runs: four Notes model/persistence tests passed, six safety
  guard regressions passed, `check-safe.mjs` passed, Python launcher compilation
  passed, before fusion contract wiring. Native lifecycle assertions were
  compiled but not rerun; newest design/opacity pixel assertions await the
  coordinated integration run.
- Latest `cargo check --locked` after fusion wiring is temporarily blocked by
  the shared APIs not yet existing: `crate::fusion::revision` and
  `Buffer::bevel_for` (same errors in Notes, Clock and Music). Previous pre-fusion
  `cargo check` passed. Re-run once coordinator lands those APIs; do not revert
  the requested fusion wiring merely to hide this expected integration gap.

## Live migration fix — complete

- Coordinator's launcher failure was the missing **Slate** palette entry, not
  damaged note contents. Metadata-only inspection found three valid Electron
  notes using Sky/Slate/Peach; all had string id/body/color, bool pinned and integer
  timestamps, with distinct nonempty IDs. No note contents were printed.
- Restored all **seven** original colors in the original Electron sequence;
  native creation and color cycling use `COLORS.len()` rather than hardcoded 6.
  Slate has its own `#b9c2d8` swatch/tint. Existing native validation stays strict
  for genuinely unknown colors/data. No source normalization or note dropping.
- Added synthetic regression matching the real Electron field/window shape with
  all seven colors. It verifies exact bodies, IDs, timestamps, pin flags, bounds,
  native round-trip, source byte preservation, and rejection of unknown colors.
- Ran the read-only ignored diagnostic
  `notes::model::tests::local_legacy_notes_load_without_writes` against the actual
  local Electron file: **passed**; all three notes loaded identically, source
  bytes unchanged, no `native-notes.json` created. Diagnostic failures withhold
  note body/ID/source bytes to avoid logging private content.
- Headless Notes model suite: **5 passed, 1 ignored**; local diagnostic separately
  **1 passed**. Current shared `cargo check --locked` now **passes** with fusion
  APIs present (only an unrelated unused General hide warning).
  `check-safe.mjs` passes. No app restart, live state write, or desktop action.
- Root/coordinator can now perform the requested launcher/UI recheck sequentially.

## Window discovery / restore follow-up

- Explicit `SetWindowTextW(..., "Lumen · Sticky Notes")` immediately after
  successful `CreateWindowExW`, so window discovery does not depend on title
  initialization during custom nonclient creation.
- Kept nearest-monitor restore and extracted a tested bounds clamp against that
  monitor's real `rcWork`; monitor lookup failure now reports an error instead
  of creating a zero-sized window. Requested rectangle arithmetic saturates.
- Fully valid bounds on a connected negative-coordinate monitor stay unchanged.
  Disconnected/offscreen positions are clamped into the nearest active work area;
  oversized windows shrink to that work area. Actual resulting bounds/DPI are
  retained for the next normal save; Electron source files remain untouched.
- Headless regression covers negative monitor coordinates, a disconnected
  position, oversized bounds and right/bottom overflow: **passed**.
  Added an explicit title assertion to the native host test for the coordinator's
  next desktop-safe run; not run here. `cargo check --locked` **passed**.
- No runtime restart or desktop automation performed.

## Native resize callback reentrancy follow-up

- Both Notes and RichEdit editor wndprocs now drop `RefMut<State>` before
  default Win32 dispatch, including the board's `WM_WINDOWPOSCHANGING` path.
  Thus native move/resize modal loops and default `WM_WINDOWPOSCHANGED` dispatch
  can reenter `WM_SIZE`, min-size and end-move handlers normally.
- Editor hit-test default dispatch and both DPI-change `SetWindowPos` calls also
  release the guard first. Destruction no longer holds a state borrow across
  `DestroyWindow`; `WM_NCDESTROY` clears userdata before default cleanup.
- Added editor capture-cancel cleanup. Extended the native RichEdit regression
  with an actual `SetWindowPos` resize: it asserts the child grows by exactly the
  parent's delta via synchronous default `WM_WINDOWPOSCHANGED`→`WM_SIZE`, and
  checks min-track response. This regression is compiled but awaits the next
  coordinated desktop-safe run, as requested; no live runtime restart here.
- `cargo check --locked` passed after the callback fixes.
- Moved Notes `WM_NCCALCSIZE => 0` and `WM_ERASEBKGND => 1` before any state
  borrow, plus stateless NC create/destroy handling. Even paint-time reentrant
  callbacks now keep the thick-frame HWND fully borderless, avoiding
  `UpdateLayeredWindow` shrinkage. Editor intentionally retains ordinary native
  nonclient framing: it is not a layered HWND and does not upload client-sized
  buffers with `UpdateLayeredWindow`.
- Expanded the native board regression with eight consecutive actual layered
  paints asserting unchanged `GetWindowRect`, and a `WM_NCCALCSIZE` call while
  holding the state guard asserting the rectangle is untouched. Compiled-only
  here; coordinator can execute in the sequential desktop QA slot.
