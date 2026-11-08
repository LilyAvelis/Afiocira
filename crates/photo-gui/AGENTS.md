# GUI changes

- Reuse the visual contract in `../../docs/ui-foundation.md` and the primitives
  in `src/ui_foundation.rs` for Settings and command rows. Fix shared geometry
  before adding individual offsets to screens.
- Preserve the shared command catalogue and dispatcher used by GUI, shortcuts,
  and the native API. UI-only changes must not create a second action path.
- Follow the current private protocol's acknowledgement, collection identity,
  selection ownership, and request cancellation rules. Publishing a request is
  not proof it was applied.
- Check narrow/normal/wide geometry, long localized labels, and DPI variants for
  shared component changes. Use the native API for synthetic frame captures;
  inspect the images and state functional/visual limitations honestly.
- Keep decoded content visible while a same-path refresh is pending. Do not
  turn a visual update into blocking filesystem or subprocess work on the UI.
