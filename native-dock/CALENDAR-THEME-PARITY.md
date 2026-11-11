# Calendar theme port: legacy source → Rust

## Three additional image references — 2026-11-10

Calendar settings now offer nine themes. The three new designs are implemented
in `calendar/reference.rs`, with shared draw/input geometry in `layout.rs`:

| Theme | Reference details and working controls |
| --- | --- |
| Mist | Blue-gray glass face, inset Weekly/Monthly segment, oversized light month/day, curved translucent seven-day ribbon, blue selected circle, note and event footer. Weekly navigation advances seven days and respects Monday/Sunday; Monthly switches to the full 42-cell grid. The shared gear opens the separate settings window. |
| Agenda | Rounded red/white split, large white selected day and uppercase weekday, actual selected-date events, past-event toggle, note/event actions. Right panel has year arrows, twelve working month selectors, red circular selection and real event dots. Narrow/short windows reflow to a compact date header plus full-width grid and footer. |
| Soft | Warm white raised face, soft directional inset edge/shadow, day/month header, vector mini calendar with coral cap, four-part month progress track, muted weekday initials, coral rounded-square selection. All calendar navigation remains available. |

No clock was added. Screenshot note/event controls use local day entries in the
existing atomic `config/native-calendar.json` store; no Graph/ICS, network feed,
or fabricated sample events. Notes and events are shared between the new views.
The modeless owned editor has two real Windows multiline inputs (Vietnamese IME,
clipboard, selection and undo), Save/Cancel, Tab/Shift+Tab, Escape and Ctrl+Enter.
An event is one line; saving an empty day removes its entry. Editing an existing
date replaces that date only. Existing configs default to empty entries.

The new face materials are cached with the wallpaper backing before shared
surface painting and background alpha; text, selection and the red agenda card
retain normal content alpha. The shared gradient/custom HSV surface contract is
implemented in Calendar's model/settings/backdrop adapter: legacy solids normalize
to matching `custom:#RRGGBB`, gradient/custom keys validate and round-trip, and the
old flat tint is skipped for managed materials. Shared compact title/header hooks,
geometry, independent blur and all six legacy theme renderers are preserved.

Verification for this pass: static `cargo check --tests`, pure calendar layout,
model and artwork groups, plus `node scripts/check-safe.mjs`. HWND/render/editor
interaction tests and desktop screenshots are reserved for the root's serialized
QA slot; this chat has not restarted the shared application. The ignored existing
`render_calendar_preview` now produces all nine themes in a 3×3 sheet. Root QA:
exercise both Mist modes, Agenda month/year and previous events, Vietnamese entry
save/cancel/reopen, compact 240×280 and wide 800×280, and zero background opacity
with gradient/custom material selection. New native setting assertions are
compiled but deliberately not executed here.

## Six legacy themes

Reference implementation read in full: `src/renderer/src/CalendarApp.tsx`,
`calendar.css`, calendar rules/container queries in `widget-designs.css`,
`src/shared/designs.ts`, `Glass.tsx`, and the inherited font/button rules in
`styles.css`. This port uses the actual CSS details, rather than one card layout
with six accent colors.

| Theme | Details now carried into native Calendar |
| --- | --- |
| Lumen | Month-only layout, 42 cells, 13.5-DIP semibold month with adjacent smaller/muted year, uppercase tracked weekdays, 12.5-DIP fixed-size dates, Today pill and vector arrows. Selected date uses the 145° accent gradient, subtle elevation and upper highlight; an unselected Today has an outline/dot. Selected Today does not retain the dot. |
| Almanac | 12-DIP outer radius, paper135° background, plain editorial date area (no generic rounded hero tile), uppercase weekday with2-DIP tracking, **Georgia** selected-day number in `#af482f`, Georgia19-DIP month and Georgia grid dates. Month/year is rotated vertically at the right and anchored22 DIP above the date area's bottom. At560 DIP it becomes the original .7:1 date/month columns with a vertical divider. Narrow layout uses a horizontal divider. |
| Blocks | Dark145° background, equal orange `#ff9029` date and blue `#306cf3` progress cards with22-DIP corners; light-weight tracked number, weekday and month/year. Month stays below both cards even in wide layout. Month has its own bordered22-DIP frame. Selected day is an orange rounded rectangle with `#14110b` text; Today outline has no dot. |
| Pop | `#090a0c` backing, blue `#3378ff` date with white weight650 digits, yellow `#ffe100` progress with `#131313` text, 2-DIP date inset/month border,34-DIP outer contour. Circular30-DIP day markers: selected `#ff4b32` with white text; Today outline without dot. |
| Prism | 155° purple→pink→gold date gradient, pink upper-right and cream lower-left radial layers, top/bottom inner highlights, white weight250 date and the inherited0/1/15-DIP white text halo. Month/year is dark `#4a203d` semibold without the halo. Progress has the original dark145° gradient plus lower-left green radial layer and `#f4ffd9` text. Selected circle `#f564b4` / `#260c23`;28-DIP outer contour and restrained month inset highlight. |
| Grove |155° forest backing; date `#172619` plus lower-right `#45620b` radial, soft green rim, weight500 `#c7f17c` digits and `#e1f6c3` labels. Progress160° `#143b32`→`#071d17`, `#57dfaf` text, **8-DIP segmented track with4-DIP bars/2-DIP gaps** and `#43e2a8` fill. Month has translucent `#16231a44` backing; selected circle `#b4ed38` / `#152109`;26-DIP outer contour. |

Pop/Prism/Grove retain the narrow1.15:1 pair of cards above the month, and at560
DIP switch to the original left column of stacked date/progress cards with the
month spanning the right column (1:1.8,14-DIP gap). They do not share Blocks'
wide layout. Progress uses the **selected day**, Gregorian month length and the
rounded percentage; the percent sign is rendered at40% of the number's size.

The Lumen `glass`, `paper` and `midnight` surface choices from the old calendar
config are also preserved, separately from the six designs. Paper `#e6ecf2` and
Midnight `#191919` use the red `#ff493f` Today circle and restrained outlined
selection of the old CSS. `config/calendar.json`'s surface choice is import-only;
native state saves `lumenSurface` without changing legacy files. An explicit
Surface palette overrides that native Lumen backing, like the old widget-theme
override. These choices are shown under Lumen in the separate settings window.

## Implementation

- `src/calendar/layout.rs`: one geometry calculation for drawing and hit targets,
  including560-DIP reflow and new minimum-size extensions.
- `src/calendar/artwork.rs`: cached rounded gradients, radial composition,
  directional inner highlights, premultiplied antialiasing and true1/2-DIP rims.
- `src/calendar/typography.rs`: theme-local Georgia/Segoe UI Variable Text,
  weight/letter-spacing and native measured widths, vertically rotated type,
  premultiplied glyphs and cached Prism text halo. Glyphs are rasterized at final
  DPI and composed1:1, never squashed into a resized tile. Native image handles
  are owned by the Calendar instance and cleared when hidden; no TLS handles.
- `src/calendar/view.rs`: individual theme content, states and interactions.
- `src/calendar/mod.rs`: existing shared chrome, source-wallpaper material,
  opacity, settings, lifecycle, independent blur and fuse integration retained.

The common animated chrome header and separate shared-kit settings window remain
the current app standard. At tiny heights, secondary artwork becomes a compact
date/progress strip so all42 dates remain available at240×280. Narrow/tall
windows keep full artwork when it fits. These are deliberate new-size extensions
of the legacy layouts, rather than restoring legacy clipping at small sizes.

## Checks and remaining visual QA

Static `cargo check --locked --manifest-path native-dock/Cargo.toml --tests`
passed. Pure layout tests cover all six designs, narrow/tall and wide/short
sizes; pixel tests cover CSS colors/gradient ordering, premultiplied rounded
edges and1/2-DIP border thickness; model tests cover migration, persistence and
date arithmetic. These tests do not initialize a native graphics/window runtime.

The added typography/native-render regressions compile but were **not executed
in this chat**, preserving the coordinator's pause on parallel native test
processes. Final app-owned render/desktop comparison belongs to serialized QA.
GDI font rasterization and the native glass compositor differ from Chromium;
no pixel-identical screenshot claim is made without that comparison.
