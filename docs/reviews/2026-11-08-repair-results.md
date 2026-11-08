# Repair results — 2.5 working preview

Historical repair snapshot recorded before the 2.5.0 release; the build identifier
and publication status below describe that earlier verification point.

The eight findings in [Azuros's review](2026-11-08-azuros-architecture.md) were
addressed without adding product features. The published release remains 2.0.0.
The working launcher points to `5387c7ff8cb4`; its executable hash was checked
against the verified release build. Existing user processes were not closed.

| Finding | Repair and evidence |
| --- | --- |
| Descendant selection scope | Explicit managed paths and retained ownership; URL-based actual-selection delta preserves unrelated native marks. Real Yazi tests cover A+B+C, removing B, and clearing only the Gallery selection. |
| Overwritten pending mutations | Immutable ordered request IDs, bounded outstanding work, acknowledgements after observed state matches. Paused-child replace/preserve/replace cases pass. |
| Stale snapshot and blocking handoff | Fresh logical read IDs, cancelled-read retirement and deferred dispatcher continuations. Status remains responsive; late responses cannot execute stale actions. |
| Concurrent settings loss | OS-locked fresh-document mutation and per-field/row CAS; complete merged-profile semantic validation. Conflicting same-key Quicklinks preserve the draft and saved/live state. |
| Same-path stale textures | External create/rename/replacement invalidates the affected content and old decode generation while preserving unrelated textures; native red-to-green screenshot checks pass. |
| Remaining Settings DDS dependency | Snapshot, mutations and runtime preference restoration share the private protocol. Exit-9 and hanging helper stubs are never invoked. |
| Album delete rejected by root-parent assumption | Captured exact collection membership and a collection identity stable across refresh, distinct on reopening. Core scope tests cover descendants and stale confirmation rejection; real Recycle Bin operations were not used for verification. |
| Unbounded API admission | Eight clients, eight queued requests, 16 MiB request frames, I/O/reply deadlines, and a per-frame dispatcher budget. Native incomplete-client admission and recovery tests pass. |

The follow-up additionally found and closed cancellation of queued Rename/Delete/
Commands continuations and semantic conflicts created by otherwise valid profile
merges. See the [independent follow-up](2026-11-08-azuros-follow-up.md) for its
evidence, inspected binaries and verification limits.

## Latency

Checked file reads repeatedly reserved their exact buffer capacity for every
64 KiB chunk. They now reserve the checked initial file length once and reject
unexpected growth/shrink. Identity, byte verification, staged publication,
no-clobber and rollback checks remain. PNG filtering and JPEG quality were not
changed for speed.

Transforms prepare a bounded preview from the verified encoded pixels and return
the published file version. The UI accepts it only for the matching operation,
session, collection, path and version. It retains the prior image until the new
texture is ready instead of briefly rendering Loading.

Two full native runs on identical deterministic half-noise 3840×2160 PNGs:

| Run | Previous build | Repaired build | Blank API samples, previous/current |
| --- | --- | --- | --- |
| `latency-ca38972b` | 875 ms | 250 ms | 7 / 0 |
| `latency-25e9157e` | 844 ms | 219 ms | 7 / 0 |

These measure request-to-current-preview availability, include API polling and
texture upload, and are not a claim about every presented monitor frame or every
source format. The benchmark and raw records are in `scripts/check-latency.py`
and the corresponding directories under `target/`.

The final paused-handoff fixture measured status response latency up to 9.2 ms.
Settings waited for a real transient user task before applying; an 8-second user
task was refused after the 5-second deadline with the previous Yazi and saved
profile intact. Private protocol work itself does not count as a user task.

## UI Foundation and scope

Shared spacing/type/control tokens and Section/Row/Control/command-row primitives
drive the existing Settings pages. The [UI contract](../ui-foundation.md) and
GUI `AGENTS.md` describe reuse and verification for future agents. Real widget
geometry, long localized labels, focus/click, narrow/normal/wide layouts and
100%/150%/200% DPI variants are tested. Fifteen native page/width API states pass.

The pinned immediate-viewport renderer cannot capture the separate Settings
window. The API reports that limitation explicitly; native Settings screenshots
and subjective visual acceptance are not claimed. Root-window captures are
verified. No history, SQLite, ComfyUI, new image actions or release publication
were added. The original review's longer-term architectural recommendations
remain guidance for later development, not a claim of a complete rewrite.
