# Afiocira — public alpha release readiness

Status: **private installer candidate; public release blocked**. This record covers Windows 11 x64 only. It does not claim portability to another machine.

## Verified on the development Windows 11 machine

- The pinned Rust workspace builds with a static CRT and remapped source paths.
- Four consecutive full workspace test runs passed with an empty Yazi configuration. Native ConPTY tests covered folder reports, sort order, selection, and mouse input. An earlier test race was corrected by waiting for the initial hovered folder rather than sending an extra navigation key.
- The Windows installer compiled from the staged release binaries, bundled Yazi 26.5.6 and the Microsoft Visual C++ Redistributable. Downloads were SHA-256 checked.
- A silent installation to an isolated directory placed the GUI, CLI, Yazi, sample PNGs, licenses, and uninstaller. The installed CLI listed all three sample PNGs. The per-user Open With registration appeared; silent uninstall removed the GUI and that registration.
- The local API rejects a JSON command frame over 16 MiB and limits simultaneous client handlers to eight. It remains unauthenticated at the application layer, and a stalled client can occupy a slot.
- A generated report covers licenses of the locked Rust dependencies included for the Windows target, including bundled font terms. A separate license text is packaged with Yazi.
- A local `git archive` source snapshot was expanded and scanned. No development user-profile path or internal Notion URL was found in that snapshot; the product's old `bigfoot` pipe identifier remains. The two newly built EXEs had no checked development profile path string. This narrow scan does not certify the absence of all secrets or private metadata.

## Still required before making the repository or installer public

1. Run the installer and the complete viewer workflow on a second, unprepared Windows 11 x64 machine. Check Open With, photo viewing, zoom, Yazi, Tab, marks, sort, mouse, CLI/API, reinstall and uninstall. Verify runtime installation if it is initially absent.
2. Inspect licenses and notices for dependencies already compiled into the bundled Yazi executable. The bundled `LICENSE` alone is not evidence that all transitive notices are covered.
3. Publish from a fresh source snapshot after a dedicated history and binary scan. The current private Git history contains old personal filesystem paths and internal links; do not make that history public.
4. Run and inspect the new GitHub Actions Windows package workflow. It has not produced a confirmed successful run yet.
5. Rebuild the final installer from the chosen public source commit, generate SHA-256 alongside it, and test that exact file. The current installer is unsigned, so Windows may show a SmartScreen prompt.

Rust, Zig, and Inno Setup are developer build tools; they are not required on the second machine when using the installer. Afiocira's code is MIT licensed by owner decision. Inno Setup's use terms must be revisited before commercial distribution of an installer made with that compiler.
