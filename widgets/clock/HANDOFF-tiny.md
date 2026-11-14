# Clock tiny-size source handoff

## Superseded by direct user request: preserve theme design on resize

The user rejected early layout switches. Removed compact.rs and the 280/160-DIP
alternate layout branches. All nine themes now use their original proportional
vector artwork at every supported size, including 120 DIP. New native/layout.rs
provides one continuous transform with header clearance at small heights; full
normal-size geometry remains identical. Afterglow emblem uses the same transform
and remains visible at small sizes. Location can yield when it no longer fits;
secondary text only yields below scale .45 if the result is under 6 DIP. No
reordering of cards, dials, timeline, time/date/footer or meters. Text continues to
be rendered at native DPI, with no bitmap squeezing.

Added pure geometry/continuity coverage through the previous breakpoints and
narrow/wide frames. Existing expanded render matrix is still queued for root's
serialized QA. Rustfmt and safety guard pass. No app build, native test process,
restart or desktop operation performed. Earlier reduced-face notes below are
historical and no longer describe current behavior.

Minimum is now 120x120 DIP through model::MIN_SIZE, used by persisted validation
and WM_GETMINMAXINFO. Existing 160+ layouts and normal content themes are unchanged.
Below 160 DIP on either axis, compact.rs renders a deliberately reduced primary
face with 12-DIP edges and 44-DIP header clearance. Digital time stays at 28 DIP
(30 DIP thin Afterglow / split-flap), paper/neon retain analog hands, split-flap
retains two tiles, modular its blue tile, and meter themes retain a progress line.
Secondary date, location, seconds labels and animated Afterglow artwork are hidden
at tiny sizes. Normal Afterglow graphic size remains unchanged.

Files: native/model.rs, compact.rs, face.rs, tests.rs. Package lifecycle adapter
native/package.rs untouched. Shared chrome helpers and host APIs unchanged.

Added model persistence/clamp tests for 120/128 square and narrow/wide minima,
pure tiny-layout bounds/header-clearance checks, expanded all-nine-face 12/24h
render matrix at four DPIs, and checks that tiny content below the header changes
visible pixels. Native minimum assertions use MIN_SIZE.

Rustfmt and scripts/check-safe.mjs pass. Native tests/build/restart/desktop QA
deliberately not run because root owns PID37520 and the serialized QA slot.
Root should run new pure/layout tests and expanded owned-surface render matrix,
then visually inspect tiny exports before final reload.
