# Windows CI/CD

The application does not need a frozen core to benefit from caching. Every run
checks out the requested source, compiles the workspace and executes its tests.
The dependency cache restores prior work; Cargo still checks its fingerprints
and rebuilds changed inputs. See [Cargo's build cache](https://doc.rust-lang.org/cargo/reference/build-cache.html).

## Triggers

| Event | Work |
| --- | --- |
| Push to main or pull request | Format, offline pipeline guard tests, compile bootstrap/tests, strict Clippy, full Rust/native tests |
| Manual run, default inputs | The same checks; useful for measuring a warm rerun |
| Manual run with package enabled | Checks followed by verified Setup/Portable candidate artifacts; no publication |
| Push of a stable vX.Y.Z tag | Checks, static-CRT packaging, verification, then GitHub Release publication |

Ordinary commits do not rebuild installers. Older checks on the same branch are
cancelled when a newer commit arrives. Different tags have independent groups.

## Caches and invalidation

- **Rust dependency artifacts:** separate check/debug and static-release caches,
  managed by pinned [rust-cache](https://github.com/Swatinem/rust-cache). Keys
  include Rust identity, runner image and build environment; Cargo manifests,
  lockfiles and project toolchain configuration affect compatibility. Workspace
  crates are not retained in this cache, so application/core code is compiled
  from the checked-out source. Failed checks may still save valid dependencies.
- **Zig and native resources:** verified Zig archive/extraction, Zig's global
  content-addressed cache, pinned Yazi/font/VC++ archives and Everything helper
  archives. Keys include platform, setup/dependency configuration and Cargo.lock;
  compatible lockfile updates can restore earlier native resources. Cached
  archives are SHA256-checked before use. These resources live in `.ci-cache/`,
  outside the Rust `target/` tree: Rust-cache prunes that tree before saving and
  must not remove a different cache's inputs.
- **Test results are never cached.** A cache hit does not skip tests, Clippy,
  version checks or package verification. Compilation errors and failed tests
  are not retried. Named transient Zig fetch failures get at most three build
  attempts; downloads use bounded timeouts/retries and atomic promotion.

Cold runs, dependency/compiler changes and runner-image updates still cost time.
Warm cache performance must be measured on actual runs, not inferred from cache
configuration. Each check job adds its hit status to the job summary; use job
and step durations in Actions for comparisons. Avoid caching the entire target
tree without pruning: old binaries, debug symbols and incremental variants can
make transfer and storage more expensive than the saved compilation.

## Release procedure

1. Update the four application package versions together. Add complete release
   notes at docs/releases/vX.Y.Z.md. Update pinned dependency metadata/notices if
   dependencies changed; changing Yazi also requires reviewing its adapter and
   reference presets.
2. Push the source to main and require a green Windows check.
3. Create an annotated vX.Y.Z tag at the intended source commit and push it.
   The tag must agree with the package version. Published tags are immutable.
4. The workflow builds static-CRT binaries with remapped source paths, stages
   from a clean owned output directory, compiles Inno Setup and creates the ZIP
   and SHA256SUMS.txt. The manifest records the exact source SHA and file hashes.
5. Verification checks asset names, sizes, SHA256, ZIP CRC/content, manifest
   source/version and first-party binary build paths. Only after check/package
   success does a separate tag-only job receive contents-write permission.
6. The publisher uses the job's GH_TOKEN, validates the actual remote tag SHA,
   creates a marked draft, uploads and verifies all three server digests, then
   publishes. No personal token or local gh credentials are needed.

A failed upload can resume only its marked draft. An already-public managed
release is accepted without writes only when its exact assets/tag match; public
or unmarked drafts are never overwritten. A manual candidate run cannot publish.
Keep release assets immutable; fix released product behavior with a new version.

## Local checks and candidates

Use PowerShell 7. The project-scoped rust-toolchain.toml pins Rust without changing
the developer's global default. Setup verifies Zig 0.15.2 and accepts Inno Setup
6.7.1 through 6.x, logging the actual engine version. Missing Inno installs the
specified fallback; an unsupported existing compiler is not silently replaced.

```powershell
./scripts/setup-ci-tools.ps1 -Installer
./scripts/tests/test-ci.ps1
./scripts/setup-native-tests.ps1
cargo build --locked -p photo-cli
cargo test --locked --workspace
./scripts/build-release.ps1
```

The local SkipBuild option is for packaging smoke tests only; CI rejects it.
Nondefault build directories require that explicit option so a normal build
cannot silently package a different set of binaries. Partial default stages
carry a separate ownership marker and can be safely restaged; nonempty unknown
directories and links are preserved and rejected.

The offline guard suite uses synthetic archives and mocked network/gh processes.
It tests interrupted downloads, checksum/CRC/source corruption, retry limits,
tag and publication guards, and draft/public-release idempotence without actual
GitHub writes or clipboard interaction. Native Spot tests wait for the real MIME
data required by Yazi instead of relying on fixed startup delays.

Future SQLite dependencies enter the existing Cargo cache through Cargo.lock.
If Python/ComfyUI integration is added, give its locked Python/CUDA dependencies
a separate integration job and cache; keep model weights and optional runtime
downloads out of ordinary Rust checks. No database or ComfyUI behavior is added
by this pipeline.
