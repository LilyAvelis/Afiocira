# Delete latency — 2026-11-07

The user reported a small delay, then confirmed it became imperceptible after
the fix. That user observation is separate evidence from the following timings.

## Method

Two runs of 24 single-file deletes on local synthetic 4096×2730 JPEG copies,
starting with 32 files. Before each delete the current and next full previews
were already loaded. Requests used the isolated live named-pipe API without
CLI process startup. Internal monotonic timers begin at job submission; they
exclude the earlier keyboard event/API dispatch and initial target capture.
Preview readiness is CPU-side texture availability at the end of UI update,
not physical monitor presentation. No desktop input was driven.

Each run also deleted groups of 3 and 5 once. Recycle Bin receipts for all 32
files were verified, then those test files were restored with byte identity.
No personal media, clipboard, cloud originals or full library was benchmarked.

## Results in milliseconds

| Stage | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| Background work, including native recycling | 19.618 | 24.822 | 25.114 | 31.910 |
| Synchronous gallery refresh | 0.272 | 0.295 | 0.337 | 0.398 |
| Job submission → successor selected | 20.969 | 30.353 | 25.666 | 82.865 |
| Job submission → full successor preview ready | 144.165 | 30.918 | 160.772 | 83.341 |
| Successor selected → full preview ready | 123.760 | 0.583 | 133.894 | 1.238 |

The median successor-readiness delay fell about 4.7× (about 79%). Shell work
did not speed up; its separate run-to-run variation is visible in the table.
The removed cost was decoding/upload preparation of the same prefetched image.

## Cause and change

`poll_operations` used `refresh_now`, which cleared every image cache after a
deletion. Even an already-prefetched successor had to be decoded again. The
Delete-only `refresh_after_delete` now refreshes membership and uses the existing
path-based cache remapping to retain surviving textures. F5 and operations that
need full invalidation keep their existing refresh behavior. Filesystem events
still invalidate changed survivors.

In the optimized burst the worker-to-UI handoff had a p95 tail of about 61 ms;
the exact source was not isolated. Live Yazi synchronization still uses short
synchronous DDS calls, and a large-folder scan still runs on the UI thread.
Neither has been certified bottleneck-free. The user reported no noticeable
delay in normal use, so no further speculative optimization was applied.

Single grouped runs after the fix: 3 files applied in about 101 ms, 5 files
in about 187 ms. These are one observation each, not percentile estimates.
Native recycling is currently serial per file; large batches are unmeasured.

## Reproduce and inspect

`scripts/benchmark-delete.py --pipe NAME --image SYNTHETIC.jpg --out result.json`
expects an isolated running GUI and creates its own GUID fixture under target.
It restores its own recycled files on completion. Keep the source synthetic.

Detailed session artifacts: `target/delete-benchmark-baseline.json` and
`target/delete-benchmark-optimized.json`. Status now exposes optional
`operation.timing` (prepare/queue, background work, UI receive, refresh, apply,
preview readiness) and `render` cache-readiness fields for agent diagnostics.
These are not user-facing promises of latency on other folders or hardware.
