# Yazi selection caps and Everything icons

2026-11-07: user screenshots showed light rounded caps at both ends of the hovered Yazi row and missing square glyphs in Everything results.

## Root cause and fix

The user's claude-inspired flavor sets hovered/indicator foreground #171412, background #C76A4F, bold=true; Powerline caps U+E0B6/U+E0B4 use the adjoining background as their foreground. egui_tty 0.2.0 Run::flush simulated bold by mixing glyph ink with the egui strong-text color, leaving cell backgrounds unchanged. That brightened caps and washed out dark selected text. This is in Afio's terminal renderer, independent of file icon mappings.

vendor/egui_tty is a minimal MIT source patch to the pinned0.2.0 package, wired through Cargo [patch.crates-io]. Adds TerminalStyle.brighten_bold (default true for upstream compatibility); Afio sets false. Explicit resolved terminal colors now reach egui unchanged. No user Yazi configuration, flavors, padding glyphs, or icons rewritten. Bold currently uses the regular font weight in Afio; this fixes color fidelity without claiming a real bold font face. Local patch provenance/license and upgrade notes: vendor/egui_tty/AFIO-PATCH.md.

Everything result rows now use Lucide SVG icons by folder/media/document/archive/audio/code/generic type, independent of text-font glyph coverage. Filename and subdued path are separate truncated lines; hover shows full path. Unknown formats get a generic file icon. This is a type-based set, not Windows Shell application icons or an import of the user's Yazi icon theme.

## Verification

Regression test terminal::tests::preserved_bold_ink_matches_yazi_background_and_dark_selected_text compares exact cap and selected-text colors and proves old mixing changes them. Self-contained render-fixes example feeds identical ANSI fixture bytes to old/new renderer modes, draws the production Everything result-row function, captures its own frame, and exits. Run cargo build -p photo-gui --example render-fixes; set AFIO_RENDER_PROBE to an absolute PNG path; launch target/debug/examples/render-fixes.exe. Requires the same installed JetBrains Mono Nerd Font as Yazi. Captured target/render-fixes.png visually reproduces the user's pale caps above and a uniform orange selection with dark text below; vector icons render in all six fixture rows. It does not control the desktop or read/change clipboard.

Full checks also caught two independent pre-existing issues and fixed them: explicit Everything folder navigation is guarded against late hovered-child reports until actual terminal input (deterministic injected-report native test); Windows rename buffer now owns a terminating UTF-16 NUL while FileNameLength excludes it, avoiding path-alignment-dependent failure. The existing rename test failed in full runs before this fix; added16 destination-length cases and reran the workspace successfully. FILE_RENAME_INFO reference: https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info.
