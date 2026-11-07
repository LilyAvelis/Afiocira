# Local patch: exact terminal colors

Based on egui_tty 0.2.0 (MIT), https://github.com/antoineMoPa/egui_tty.
Source copied from the pinned Cargo registry package; upstream LICENSE retained.

Adds TerminalStyle.brighten_bold (default true, preserving upstream behavior).
Afio disables it to retain exact resolved foreground colors in Yazi. Upstream
simulates bold by mixing foreground with UI strong_text_color; that changes
powerline caps while their adjoining backgrounds stay unchanged, and brightens
dark selected text. No Yazi theme or user config is rewritten.

Regression test exercises both cap and selected-text colors with/without mixing.
A real bold font face is not added; Afio currently preserves color and uses the
regular Nerd Font weight for SGR bold. Remove patch when equivalent upstream
control is available. Keep local source edits here, never in the Cargo cache.
