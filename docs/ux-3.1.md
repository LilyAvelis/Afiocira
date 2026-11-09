# 3.1 work: focus, continuous preview and editing tools

This is development after v3.0.0, not a published 3.1 release.

- Plain Tab belongs to Gallery/Yazi outside modal/editing panels, even when the
  folder path or toolbar button has focus. It is routed before widgets and also
  cancels egui's already queued focus traversal. Shift+Tab and form navigation
  remain local to their existing contexts; unfinished editors keep their lock.
- Preview retains the last usable image. One worker processes a cached 1280px
  base and one replaceable pending request. Completed intermediate frames can be
  shown during a drag, instead of being starved by newer requests. Cancellation
  epochs still reject old sessions; Apply waits for the exact latest revision.
  The existing texture is updated in place. This is CPU preview, not a new GPU
  engine or a promise of 60 frames/second for all filters.
- Transient spinners/loading labels and operation-running footer text are
  removed. Busy flags and disabled duplicate-submit controls remain functional;
  real errors remain visible. Long header messages truncate with full hover text.
- PTY size changes wait for 80 ms of stable cell dimensions. The existing grid
  stays clipped to the moving pane rather than repeatedly reflowing during drag.
  The terminal application still owns its own final redraw.
- `edit_image` (Редактирование…) is first in Ctrl+K's Редактирование group and
  exposes all registered adjustment groups in the same panel. Individual tools
  remain available. It uses normal command dispatch, API and shortcut assignment.
- The former 256 MiB editing estimate rejected normal 4K/12 MP photos. The bounded
  working estimate is now 1 GiB, with the existing 32 MP/64 MiB encoded limits.
  Error text describes the estimate and limit; the old error did not establish a
  memory leak. Preview and full-size processing share the admission check.
- Yazi navigation acknowledgement gets up to five seconds (other mutations keep
  their existing window), below the host's eight-second timeout. Failure now
  reports the operation and observed directory/cursor. It never assumes success.

Validation uses the existing Windows pipeline. User-side reproduction remains
important for Everything focus, continuous slider dragging and splitter redraw.
Personal `media/` files are not part of this change.
