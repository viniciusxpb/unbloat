# Unbloat v0.2 — validation record

Date: 2026-10-17 (America/Sao_Paulo).

Environment: Windows 11 25H2 x64 (build 26200), Rust/Cargo 1.95.0,
MSVC + Windows SDK, independent 7-Zip 26.01. Builds require stable Rust >= 1.93.

## Completed

- The context-menu entry now includes the original Unbloat icon embedded in the
  shell DLL. A 32-bit alpha bitmap is retained for the COM handler's lifetime
  and freed afterward. The native probe checked visible pixel data, repeated
  menu queries, and bitmap cleanup. After updating the user's installation,
  Windows composed icon-bearing menus for an image, a folder, and a mixed
  selection; the bitmap was 16x16 on the tested display configuration.
- Double-click regression fixed: launching `unbloat.exe` without arguments now
  opens a persistent native welcome/install window instead of briefly opening
  a console and exiting. The release uses the Windows GUI subsystem.
  `scripts/test-launcher.ps1` verified window persistence, installation through
  the actual button/installer in an isolated directory, installed-app usage
  instructions, and redirected CLI stdout. The temporary registration was
  removed. A captured native-window image was inspected for readable layout.
- Engine preflight passed **before** shell implementation: two projects with
  distinct ignore files, real scanner/manifest, real LZMA2 solid compression,
  real extraction and content/hierarchy comparison.
- `cargo test --workspace --all-targets --locked`: 14 passing test cases
  (12 engine cases + shared transport roundtrip/cleanup in each package).
- Individual image files, multiple selected files and mixed file/folder
  manifests round-tripped successfully. Explicitly selected files are included
  without loading parent ignore rules; ignore rules still apply inside selected
  directories. The installer now registers both file and directory handlers.
- The user's installed folder-only handler was migrated while its DLL remained
  loaded in Explorer. A content-named DLL and a new COM identity avoided
  overwriting the mapped DLL or restarting Explorer. The Windows shell itself
  composed menus containing a top-level Unbloat entry for a PNG image, a folder,
  and a mixed selection (`shell_probe --explorer-menu`). Registered invocation
  of the installed handler compressed the PNG successfully.
- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets
  --locked -- -D warnings`: passed.
- Release executable and native COM DLL built. PE imports inspected with
  `dumpbin /dependents`: only Windows system DLLs; no external compression DLL,
  Rust runtime DLL, or Visual C++ redistributable DLL required.
- The final ZIP was extracted and its executable successfully archived the
  fixture with PATH containing only Windows directories (no Rust/Git/7-Zip).
  The resulting archive then passed an independent 7-Zip check. The packaged
  installation/uninstallation scripts passed, and the installed executable's
  SHA-256 matched the release build.
- 7-Zip independently tested and extracted the preflight archive. It also tested
  an archive containing this repository's two real Rust package trees: 17 files,
  7 directories, solid LZMA2, no integrity failures.
- `make run` read `development.test_input` and produced the fixture archive.
- Real Windows `IDataObject`/PIDL selection and native `IContextMenu` tested via
  the dynamically loaded DLL: single folder, two siblings, three siblings,
  Unicode, spaces, direct menu text, default-only suppression, class factory
  activation and final `DllCanUnloadNow == S_OK`.
- Each invocation launched a separate executable with a native activity window.
  Cancellation closed safely without publishing an archive. Normal completion
  published exactly one archive and closed its activity window.
- Per-user COM registration, both COM interfaces, registered invocation,
  reinstallation and uninstallation tested with the real PowerShell scripts in
  an isolated directory. Temporary test registration was removed afterward.
- Reinstallation while the real application was compressing a 512 MiB source
  correctly reported the executable in use. The process then accepted safe
  cancellation; no process was forcibly terminated.
- Concurrent archive publication retained existing files and produced distinct
  output names. Tests also covered missing/changed files, mutation after encoding,
  cancellation after encoding, locked source files, unreadable ignore files and
  temporary cleanup.
- The dependency feature tree includes only sevenz-rust2's `compress` feature
  (LZMA/LZMA2 encoder), with default codecs, encryption and utility features off.
  Original license and notice texts were collected for the locked dependency tree.

## Benchmark

One release run; 1,501 included files, 10,000 ignored files. LZMA2 preset 5,
64 MiB maximum solid blocks, one encoder thread. These are observations, not
portable performance guarantees.

| Measurement | Result |
| --- | ---: |
| Scan before adding ignored files | 78.22 ms |
| Scan after adding ignored files | 84.53 ms |
| Walker entries returned, both scans | 1,503 |
| Archive creation + full verification | 1.142 s |
| Peak process working set | 38,010,880 bytes |
| Unbloat archive size | 11,276 bytes |
| Original 7-Zip compression, same approved files | 0.418 s |
| Original 7-Zip archive size | 13,584 bytes |

The walker counter is the number of entries yielded to Unbloat, **not** a count
of OS directory queries. The ignore crate prunes excluded directories before
recursion; adding ignored descendants did not increase yielded entries. Disk
cache/antivirus affect timings. The 7-Zip comparison uses LZMA2, level 5, solid
64 MiB and one thread, but its timer excludes post-write verification and its
metadata/default encoder options differ. Do not infer a compression-speed or
ratio advantage from this small fixture. Rerun with `make benchmark`; its full
report is written to `target/benchmark/RESULTS.md`.

## Remaining manual checks

- Visual inspection of the actual Explorer context menu, including its position
  alongside other handlers. The probe verifies the real top-level menu contract;
  it does not visually inspect the Explorer UI.
- Windows 10 execution/installation and Windows 11 classic menu appearance on
  additional machines, including environments with shell-extension restrictions.
- File symlink creation was unavailable in this host's restricted token (Windows
  error 1314); that conditional test reports the limitation. Real junction
  exclusion/root rejection passed. The common reparse-point filter also excludes
  file symlinks. Rerun the file-symlink case with Developer Mode/appropriate
  privilege before calling cross-environment validation complete.

The implementation and distribution are available; this record does not claim
all Windows versions or visual Explorer behavior have been manually certified.
