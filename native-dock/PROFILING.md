# Native dock profiling — 2026-11-09

The earlier animation measurements below used a roughly 400–440 × 348 device-pixel dock,
29 pinned entries and wallpaper glass. Values are individual observed samples,
not statistically controlled benchmarks or guarantees for larger widgets.

| Path | Before | After |
| --- | ---: | ---: |
| Warm frame CPU work | 28–35 ms | about 7–10 ms |
| Warm presentation cadence while animating | about 29–38 ms | commonly about 16.6 ms |
| Window move, through presentation | 47.2 ms | 13.3–15.3 ms |
| Held resize, through presentation | 53.3 ms | 19.3 ms |
| Resize release rebuilding content | 57.3 ms | 26.9 ms |

The profiler split background, header, list, composition and presentation. Warm
frames regenerated zero icon/label/tile caches, yet GDI+ image drawing consumed
about 24 ms per frame. Direct premultiplied sprite blending, cached whole tiles
and group materials, and integer interpolation weights removed much of this
cost. Fractional translation is retained; hover scaling uses continuous bounds.
Cache sizes use canonical dimensions so fractional scrolling cannot invalidate
text or tile caches. Regression checks compare optimized sampling with the
reference affine sampler and verify that sliced group images have no seams.

The intermediate animation pass used a monitor-sized wallpaper cache to avoid
repeated scaling while moving. The later memory pass removes that duplicate:
native moves now keep the existing layered surface frozen until release.
Wallpaper rendering targets only the widget viewport, with the same Fill/Fit
mapping and source resolution. Partial-strength blending needs only a viewport
scratch buffer, shared across draws for its owning window.

The memory pass used 29 pinned entries on a 3840 × 2160 desktop, using
`--smoke-test` before and after. Window size and live use affect memory; these
are observed same-session samples rather than a controlled benchmark:

| Measurement | Before | After |
| --- | ---: | ---: |
| Private memory | 83.83 MiB | 46.25 MiB |
| Working set | 119.10 MiB | 84.89 MiB |
| First window | 769 ms | 521 ms |

These are individual cold-start samples, not guarantees. The 5-second CPU
average includes decoding/initialization and is not an idle CPU measurement.
Private memory fell about 45% in this comparison. Further changes release the
shadow's CPU DIB after upload (Windows retains its surface), allocate the list
fade buffer only while needed, and release the Settings paint buffer on close.
Soft-shadow cache masks use one alpha byte per pixel instead of a duplicate
four-byte black RGBA image. Icons and text keep their original resolution.
A regression check compares direct viewport pixels with the previous full-
monitor crop in Fill and Fit layouts and verifies bounded scratch allocation.

Resize coalesces input on the animation timer, retains current icon/text sprites
while held, and preserves group corners with cached slices. After release, two
existing labels are refined per paint so text layout does not stall one frame.
The expensive grain texture fill was replaced by a deterministic cached pixel
layer. The widget shadow uses its own cached wide mask; the older tight shadow
inside the root bitmap was removed to soften the bottom edge.

Resize still has an occasional frame above the 16.7 ms budget, particularly when
the new glass layer is finalized. Initial shell-icon extraction and previously
unseen search results can also produce cold frames. Large dock sizes and other
DPI/monitor combinations need separate measurements; these samples do not prove
60 FPS for every operation. The first cold frame is excluded from warm figures.

To collect local diagnostic frames, close the running native dock, then run:

```powershell
$env:LUMEN_TRACE_FRAMES='1'
python run_native_dock.py --no-build --inspect
```

The launcher saves output to `config/logs/native-dock.log`. `bg`, `header` and
`tiles` are cumulative elapsed marks from frame start; subtract adjacent marks
for stage cost. `cpu` ends before upload; `present` includes upload/shadow sync;
`ms` also includes optional DWM pacing. `regen` counts new shell icons, scaled
icons, labels and tile sprites. Nested `draw_ns` counters must not be summed.
Normal launches omit the trace flag and `--inspect` and run in the tray.
