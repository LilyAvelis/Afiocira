# Public release validation

Version 2.0 supersedes the 1.0 test build. See [the 2.0 release record](release-2.0.md) for packaging and local checks. Public history retains the original 1.0 commit; private development history is not merged.

The public installer and portable assets include Yazi, ES, Nerd Font and the notices for those components. Optional video setup downloads checksummed libmpv/FFmpeg distributions directly from upstream on the user's machine.

Outstanding: independent testing on a second unprepared Windows 11 x64 machine, including installation, missing VC++ runtime, video download failure/retry and uninstall; a confirmed remote CI run. The installer is unsigned.
