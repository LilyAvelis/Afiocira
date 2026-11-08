# Modifier capture and keyboard-only inline assignment

The pinned egui emits physical left/right modifier key events. Both shortcut
recorders previously accepted the first fresh key, so Ctrl+H could save
`Ctrl ControlLeft` before H arrived. The shared recorder now ignores physical
Control, Alt, Shift and Super keys, waits for the main key, and retains its
modifier mask. API/manual parsing rejects modifier-only bindings. Settings now
uses this same recorder instead of duplicating its event selection.

The command palette's Ctrl+Shift+Comma editor shows the current binding (or
"not assigned"). Ctrl+Backspace clears it through the existing SetShortcut
dispatcher and persists an explicit null override. Plain Delete remains
bindable. No file command receives recording/navigation/clear input.

Conflict messages name the requested chord, the existing owner and the view
context, and explain that replacement removes the owner's binding. Replacement
remains explicit. Existing malformed saved assignments are not guessed or reset;
the user can remove or replace them in the repaired editor.

Conflict buttons support Left/Right/Up/Down, Tab/Shift+Tab and Enter, with a
visible selected button; Escape cancels. In recording mode, Tab first moves to
buttons, preserving the ability to record arrow-key bindings. Buttons wrap at
narrow widths. The release gate drains the chord that opened recording or
produced a conflict before accepting the next operation.

Verification: 259 workspace tests plus 3 doctests, strict Clippy, formatting and
diff checks passed. Regression cases include both sides of Ctrl/Alt/Shift in
single and separate frames, physical-modifier rejection, explicit empty override,
owner/chord conflict text, arrow/Enter/Ctrl+Backspace/Escape ownership, and editor
geometry at 240/420/650 logical widths with DPI 1/1.5/2.

The user explicitly chose to perform the live keyboard/computer verification.
No UI input or synthetic GUI launch was performed for that verification. Working
version `60f4f12b3788` is installed through the permanent desktop shortcut; its
executable hash matches the built release. Existing application windows remain
untouched.

## Follow-up: legacy binding poisoned all assignment saves

The first fix rejected physical modifier bindings in the parser but missed a
legacy compatibility path: conflict detection used `?` on every other command's
binding. Existing `space.rename = "Alt AltRight"` therefore rejected every new
assignment in Space, even plain letters. This was reproduced with the actual
shortcut map, read without changing the user's preferences.

Conflict detection now treats invalid existing bindings as inactive, matching
runtime dispatch, while still rejecting invalid newly proposed input. Existing
values are retained; both clearing and repairing the old assignment work.
Non-conflict validation failures keep the inline editor in recording mode and
invite a new chord, rather than silently switching it to button-only input.
Genuine conflicts continue to require explicit replacement.

Regressions cover the real old modifier assignment alongside ordinary bindings,
plain/modified assignment, removal, repair, retained unrelated preferences and
genuine conflict detection. Inline recovery tests verify fresh letters and
modified chords immediately after a failed validation. The isolated native
fixture is seeded with the malformed old assignment and exercises save/clear
through the file-backed shared dispatcher. No user window receives test input.

Follow-up verification: 261 workspace tests plus 3 doctests, strict Clippy,
formatting and diff checks passed. Native fixture
`target/edit-panels-e2b79ee6/results.json` passed, including the malformed legacy
assignment save/clear regression. Working version `516271a7c1a4` installed;
installed executable hash matches the tested release.
