# Tiny Calendar source handoff — 2026-11-10

READY for root compilation/serialized QA. Calendar minimum is now **168×190 DIP**.
Source changes are restricted to `widgets/calendar/native`. No build, native test
process, HWND creation, app reload/restart or desktop action was performed.

## Implementation

- `model.rs`: one `MIN_WIDTH=168` / `MIN_HEIGHT=190` pair, used by both legacy
  import and native persisted-config clamps. Defaults and existing larger bounds
  are retained. Round-trip coverage now includes minimum, below-minimum,
  narrow/tall and wide/short sizes.
- `mod.rs`: the same constants feed WM_GETMINMAXINFO, manual resize and fusion
  resize/snap limits. Physical minima round upward at fractional DPI. Tiny hit
  testing includes only header, month navigation/Today and 42 actual dates; hidden
  controls are not left as invisible targets. Mist tiny navigation advances months
  rather than seven days; Agenda tiny navigation also advances months rather than
  years because its twelve-month strip is hidden.
- `layout.rs`: below 240 DIP wide OR 280 DIP high, all nine designs prioritize
  month/date content with no hero/progress/sidebar. Month label at upper left,
  common header at upper right, separate 24-DIP navigation row, weekday labels,
  then 42 cells. At 168×190 cells are 18×12 DIP, with adaptive date typography.
  Pure regression covers all nine at minimum, narrow/tall, wide/short and both
  sides of the compact breakpoint, checking every control/date against each
  other and shared header, plus restoration of normal layouts.
- `view.rs`: dedicated tiny rendering keeps theme background/accent identity and
  foreground contrast, readable short month/year, weekdays and selected/today/
  hover/pressed/keyboard-focus states. Secondary controls disappear only while
  small. Mist's saved Weekly preference returns when enlarged; it is not rewritten.

Preserved root's removal of the whole month-panel rim, `nav_top = top + 30.` for
normal Lumen's two-row navigation, and the short-glyph ellipsis fix in typography.
The full-frame material/single outer bevel fix, package lifecycle adapter and
widget.json remain unchanged.

## Validation

Rustfmt parse/check passed on the four edited Rust files. Tests were added/updated
but intentionally not executed under root's no-build/no-native-process constraint.
Recommended root filters:

```
calendar::layout::tests::
calendar::model::tests::
calendar::tests::compact_themes_keep_all_date_targets_inside_bounds_and_blur_is_private
```

The native compact test now covers 168×190, 168×520 and 800×190 at four DPIs in
addition to previous sizes, and expects 42 cells for tiny Mist. Desktop QA should
resize each theme down/up, check date input/keyboard navigation, shared header,
Today, week-start setting and restore Mist Weekly/Agenda sidebar on enlargement.

Cross-thread tool messaging was not used because it requires direct human
authorization; the delegation itself does not grant that authorization. Reporting
through the response and this handoff without waiting for root to poll status.
