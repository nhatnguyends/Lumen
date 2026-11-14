# Settings UI + Calendar follow-up — 2026-11-10

## URGENT native test crash report / test freeze

Stopped native/HWND test launches after root's Application Error message. No
desktop interaction, popup dismissal or app restart was performed by this chat.

Two earlier native test commands crashed (before the header cache fix):

1. Cargo test filter `calendar::` via `python -c ... run_native_dock.toolchain()`.
   Model tests printed pass; native tests had not completed. Exit **0xc0000005,
   STATUS_ACCESS_VIOLATION** from
   `native-dock/target/debug/deps/lumen_native_dock-0114aacfc302aa34.exe calendar::`.
2. Isolated filter
   `calendar::tests::compact_themes_keep_all_date_targets_inside_bounds_and_blur_is_private -- --nocapture --test-threads=1`.
   Last output was that test name, then the same **0xc0000005** exit. This is the
   likely source of the user's still-open Application Error dialog.

Source cause identified and fixed locally: initial shared header helper stored
`Symbols` in thread-local storage. Its native GDI+ Image handles survived the
`render::Runtime` shutdown, so TLS destruction/reuse could access disposed GDI+
state. The header now caches **only plain RGBA pixel Vecs**, bounded to 8 entries;
each native Image is scoped to one draw and dies before Runtime. No edits to
render.rs or symbols.rs were made. Correct gear SVG is toolbar **5.svg**, not 4.

Latest command sequence **after that fix**, just before root's stop request:

```text
cargo test --locked --manifest-path native-dock/Cargo.toml calendar::
cargo test --locked --manifest-path native-dock/Cargo.toml settings_ui::
cargo test --locked --manifest-path native-dock/Cargo.toml general::
```

All ran in one Python subprocess sequence, exited **0**: Calendar 8 passed /
1 ignored; settings_ui 3 passed; General 2 passed. Last running test was
`general::tests::hidden_host_emits_real_widget_actions_and_releases_surfaces` and
it passed. Added `settings_ui::tests::shared_header_cache_survives_successive_renderer_sessions`
to exercise the same-thread Runtime/cache teardown/restart; it passed. Native
tests use `crate::native_ui_test_lock`, but that does not serialize separate
cargo-test processes from other chats. Root should perform final serialized QA.

## Source ready for integration

**Static build ready / final peer state:** after root's test freeze, only source
analysis, rustfmt, `cargo check` and `node scripts/check-safe.mjs` were run; both
checks passed. Root subsequently reported the full suite passed serially:
117 tests passed / 4 ignored, including Music's repeated layered-window regression
and the shared header Runtime cache regression. No further native test process
was launched by this chat after the freeze.

Final small source follow-ups after that serial run: shell hide releases capture
if its slider/resize owns it; Calendar common-setting changes refresh values in
an already-open separate settings window. No runtime restart needed by this chat.

- `src/settings_ui.rs`: shared Dock-derived separate settings window; schema,
  async Change queue, section cards, real switches/sliders/value labels/palette
  choices, header close/back, drag, edge resize420×360, clipped scroll and draggable
  scrollbar, keyboard controls, DPI, shadow, singleton reopen, hidden cleanup.
- `SETTINGS-UI-CONTRACT.md`: published stable API/schema/message ownership.
- Shared widget chrome helper exported by same module: x20/title12-DIP semibold,
  title case, y8..48 row40 DIP, 20-DIP outline gear/hide, 32-DIP hit rectangles.
  Pure pixel gear cache avoids native resource lifetime outside Runtime.
- `src/calendar/mod.rs`: settings gear and manager/tray Host::show_settings now
  open one separate shared-kit HWND. Widget contents/date/theme/bounds remain;
  inline settings branch removed. Private calendar Glass keeps blur independent.
- `src/calendar/model.rs`: minimum240×280 (import, validate, restore); blur,
  frostStrength and palette default fields added, old config remains compatible.
- `src/general.rs`: migrated from custom-painted separate controls to a thin
  shared-kit adapter. Public Root API and Action enum/message unchanged.
  General list uses Show/Hide/Settings choice cards for each of five widgets;
  all-widget opacity slider, floating and snap real switches use the kit.

Root already added `mod settings_ui`; no shared main/window/tray/fusion/compositor
or Dock settings.rs was edited by this chat. Existing root wiring is preserved.

## Compact Calendar

At240×280 and other narrow/short sizes, grid uses12-DIP horizontal padding,
month labels omit the year, all42 cells retain >=24×20-DIP targets, Today and
month arrows remain distinct. Theme artwork becomes a compact selected-date
strip when width<320 or height<380; larger themes retain hero/progress artwork.
Lumen stays a clean month grid with **no hero** at any size. Native reference
image was Almanac, whereas old reference was Lumen; no user theme was reset.

Tested hidden owned renders: 6 themes × sizes240×280,240×520,800×280,420×460 ×
DPI1,1.25,1.5,2. Date targets/content remain in bounds. Model regression confirms
240×280 survives restart. Settings singleton, hidden cleanup, unchanged date/
geometry and independent blur were exercised. Final desktop QA remains root-owned.

## Limits / review points

- No desktop QA or app restart for this follow-up. Hidden buffer visual previews
  still need final inspection after the test freeze; previous v1 preview files
  must not be represented as current v2 screenshots.
- Calendar uses a private Glass to isolate blur. This duplicates wallpaper
  decoding for now; shared decoded source/independent blur variants would be a
  later optimization outside this chat's allowed shared-file scope.
- Dock settings remain original settings.rs. Kit mirrors those drawing constants
  and controls; root can refactor Dock onto kit later without changing the API.
- General dialog position field is retained in config for compatibility, but
  the shared shell currently re-centers by owner when first created.
- Header gear rebuilds a scoped native Image from cached pixels each draw;
  this avoids cross-Runtime handles. Profile if needed after correctness QA.
