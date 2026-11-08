# Metadata size, clipboard feedback and Escape recovery

CopyPath/CopySelectedPath now show a noninteractive two-second confirmation:
"Путь к папке скопирован" or "Путь к файлу скопирован". Raw metadata copy uses
the same feedback. Settings → Afio has an enabled-by-default persisted checkbox;
`SetCopyNotifications` shares the dispatcher and snapshot. New confirmations
replace the previous timer instead of building a queue. Disabling removes it
immediately and does not affect clipboard output or keyboard focus.

All embedded metadata groups start collapsed, scoped to the source path. ComfyUI
workflow graphs show node count/types rather than hundreds of widget/layout
fields. Prompt graphs retain useful bounded parameters. Schema detection also
handles nonstandard PNG keys and nested graphs; unknown JSON and long PNG text
retain full raw payloads within the existing budget.

Raw XML/JSON/text has a visible complete-copy button independent of its collapsed
viewer. Source preview starts at 1024 characters; individual long properties at
240. "Показать всё" is explicit. Full source viewing uses bounded virtual rows,
including single-line JSON, avoiding a megabyte-sized galley. Clipboard output
always uses the full retained original text rather than the visual fragment.

The Ctrl+G metadata panel previously had no Escape owner and could leave users
in Yazi while Tab was disabled by the open panel. Escape is now consumed before
terminal/general routing, including when a text editor is focused. The shared
`CancelMetadata` command clears the UI draft, closes the panel, releases widget
focus and returns the active pane to Gallery. It does not create a save request.
Already admitted saves cannot be undone merely by closing the UI; late receipts
are ignored by the canceled draft. The UI button is explicitly "Отмена · Esc".
Existing navigation protection during an unfinished image edit is retained.

Validation: 296 workspace tests plus 3 doctests passed, strict Clippy/fmt/diff
passed. Tests cover timer expiry/replacement/disable, preference persistence,
workflow summaries and full raw retention, Unicode-safe virtual rows and complete
clipboard contents, metadata cancellation with dirty/loading state, stale
receipts, and Escape event ownership with terminal/text focus. Geometry tests
cover narrow/normal/wide and DPI1/1.5/2.

Native fixture `target/embedded-ee709129` passed using a 600-node workflow:
compact field count, complete JSON survives save, read-only sparse inspection,
CancelMetadata closure/focus/late completion, and persisted notification setting.
The native/API checks do not claim physical keyboard/mouse acceptance; the user
performs live interaction checks. Existing user windows remain untouched.

Existing editing regression fixture `target/edit-panels-15368a55` passed.
Working version `2b301aecc056` installed through the permanent desktop shortcut;
its executable hash matches the tested static-CRT release. Existing user
application windows were left open.
