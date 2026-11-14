# Calendar date-only / corner correction — 2026-11-10

READY for root's source integration and serialized QA. This is the latest direct
user steering, superseding the previously added local note/event features.

## User-requested changes

- All nine themes, including tiny mode, now use `CHROME_RADIUS = 34.`. This matches
  the shared WindowShadow template, avoiding the different 12/22/26/28/30/38-DIP
  foreground contours showing the shadow as a crescent/extra panel at corners.
  The frame constructor, coating, reflection and final mask consume the same
  layout radius. Internal card rounding remains theme-specific. Initial cached
  frame radius uses the same constant. Shared renderer/compositor were not edited.
- Agenda's number now receives a full `1.6 × font size + 8 DIP` line box, with
  4-DIP separation above the weekday. Its hero region has enough height for this
  allocation; number size also adapts to available width/height. This fixes the
  cropped lower part of the large numeral reported in the user's screenshot.
- Removed ALL event/note UI, hit targets, actions, editor host, message handling,
  event dots, past-event toggle/list and entry persistence from native Calendar.
  `entry_editor.rs` is deleted. Config remains version1 and reads earlier files
  with unknown entry fields harmlessly; loading alone does not rewrite them.
  Future native config saves serialize only the date/calendar settings.
- Mist/Agenda no longer reserve an empty footer. Their grid/ribbon can use the
  released height with a 20-DIP bottom inset. Weekly/Monthly, month/year navigation,
  Today, date selection, settings and all nine visual themes remain.

## Files

Edited `mod.rs`, `layout.rs`, `model.rs`, `reference.rs`, `view.rs`; removed
`entry_editor.rs`. Changes are only under this native package. Lifecycle adapter,
manifest, shared header, full-frame material painting, root's month-panel rim
removal, typography fixes and 168×190 minimum were preserved.

## Verification

Rustfmt parse/check passed for all five edited Rust files. No app build, native
test process, HWND operation, reload/restart or desktop action was performed;
root still owns the active application and serialized QA slot.

Added pure regression coverage:

```
calendar::layout::tests::themes_share_the_chrome_and_shadow_contour_at_every_size_and_dpi
calendar::layout::tests::agenda_number_has_full_line_height_and_clearance_above_weekday
calendar::model::tests::date_only_calendar_loads_earlier_configs_without_reintroducing_entry_features
```

Recommended root check groups: `calendar::layout::tests::`,
`calendar::model::tests::`, `calendar::artwork::tests::`, then the existing native
calendar compact/render tests in the shared serialized slot. The updated reference
theme round-trip test is now `reference_themes_preserve_materials_and_view_across_restart`.
Visual QA: corners in Almanac/Mist/Soft, Agenda digits 3/8/10/28/31 at compact and
large sizes with fractional DPI; confirm no event/note text/buttons anywhere.

No cross-thread tool message was sent because its direct-human-authorization
requirement is not fulfilled by an incoming delegation; delivering this response
and source handoff immediately instead of waiting for polling.
