# Gallery and embedded Yazi state

`yazi.rs`, `yazi_protocol.rs`, and `navigator.rs` share one process-private ordered
protocol. It carries Gallery selection changes, cursor/folder navigation, runtime
preferences, theme reloads, order requests, and snapshot barriers. It does not
require `ya emit-to` or a working shared DDS listener for those operations.

Each Yazi process has a private session directory with a process ID, counter, and
timestamp. Request IDs increase within that session. A complete JSON request is
published by renaming a temporary file to `folder.txt.request.<id>`. The Lua
consumer claims that exact ID, runs it, publishes `folder.txt.ack.<id>`, and then
advances. A cursor-only request cannot overwrite a pending selection replacement;
two replacements remain in their published order. Explicit selection on/off
operations are idempotent, including bounded retries within the same request.

Publication is not acknowledgement. Selection and navigation acknowledgements
verify the actual Yazi selection/cursor/folder from a synchronized snapshot.
Selection compares only the managed scope and preserves selection outside it.
Runtime preference commands use Yazi's `ya.exec` completion and verify the actual
sort, hidden, and linemode values. Acknowledgements and snapshot responses are
published atomically; response write failures are reported when an acknowledgement
can still be written. Rust waits until the consumer has retired its claimed file
before collecting the acknowledgement.

Gallery owns its currently loaded paths and the marks it previously supplied or
imported into a loaded Gallery. A replacement removes selected paths absent from
the new mark set only within that scope. Lua calculates the delta from the actual
selection and uses Yazi's explicit URL argument to `toggle`; it does not traverse
every loaded file or clear the current directory wholesale. Marks independently
selected outside that scope survive replacement and Escape-clear. Restoring a
replacement Yazi's complete context does not claim those external marks as Gallery
ownership. Gallery ownership is transferred separately from the previous pane.

`selection_dirty` stays set until the corresponding request is acknowledged and
its published marks still match the same Gallery collection. A reopened collection
has a new identity, so an old acknowledgement cannot clear its pending change.
The latest unsubmitted Gallery state may replace an older unsubmitted state; an
already published mutation is never silently discarded.

Every logical snapshot gets a fresh request ID and response path. Polls of the
same read reuse its request; a timeout cancels that ID. The consumer retires
cancelled requests in order, and Rust collects any late response without importing
it. A later focus handoff cannot accept the previous read's response.

Focus handoffs and actions that need the native selection use pending commands in
`navigator::State`. The GUI continues rendering and servicing API requests while
the snapshot is pending. After a fresh response, the dispatcher imports the
context and resumes the original command. An interaction revision change, timeout,
or transport error cancels the continuation before it can act against stale
Gallery data. API callers retain their reply until that command completes or fails.
Matching explicit cancellation also retires a pending rename, delete, or command
palette continuation, tombstones its snapshot ID, and replies to its original
caller with a cancelled error. An unrelated cancellation or status query leaves
the continuation intact. Late snapshots cannot reopen a dismissed dialog or run
a cancelled rename. Other clients can still query status. API `navigator.pending`
describes scheduled Gallery mark synchronization or unretired transport work;
`request_id` identifies an outstanding private request when one has been published.
This includes cancellation awaiting the consumer. A logical focus timeout is an explicit API error even while its cancelled
transport request is still waiting for a paused Yazi to resume.
The explicit `navigator_snapshot` API response also returns `navigator.runtime`
with the actual native marks, order, preferences, metadata and tab count from
that acknowledged read. Ordinary status responses omit these potentially large
details. Integration checks use this API instead of injecting private files.

The stream allows at most 64 outstanding requests, each at most 4 MiB. Queue
overflow is an explicit error. Requests remain on disk for polling rather than
being overwritten or republished with new identities. Mutation verification and
transport deadlines are bounded. An uncertain delivery failure stops further
submissions with an error requiring a navigator restart; restarting clears pending
continuations and republishes the current Gallery selection. Terminal input and
PTY output use separate bounded queues (64 input packets and 128 output chunks,
at most 8 KiB per output chunk). These limits do not bound the whole process.

Settings uses the same snapshot, preference, selection, and acknowledgement path.
It builds a candidate from the merged current settings document and verifies its
complete runtime context before publishing that profile. A concurrent profile
change fails the prepared commit explicitly, preserving the old live Yazi. The
candidate's transport errors are observed during the apply state machine. The
protocol plugin starts its work through a synchronous entry and an async Lua
callback, so protocol snapshots do not become scheduler tasks themselves. The
native task summary is left intact; real user jobs still block Settings Apply.
Apply waits asynchronously for transient tasks to finish within the original
five-second context deadline. A dialog is rejected immediately; a job still active
at the deadline aborts Apply and preserves the old Yazi and persisted profile.
External tool probes and compatibility shutdown remain bounded. `ya.exe` is used
only for the compatibility shutdown fallback, not managed preference restoration.

Regressions: `yazi::protocol::tests`, the native `yazi::tests::native_*` tests, and
`scripts/check-private-protocol.py`. They use synthetic files/configurations and
suspend only their own fixture's Yazi child; they do not stop a global DDS listener
or operate an existing user GUI.
