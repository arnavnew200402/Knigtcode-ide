---
title: Packaging offline desktop resources
description: "Acquire and verify the fixed WebView2 runtime and English dictation model for Windows installers."
---

# Packaging offline desktop resources

Acquire the reviewed desktop resources before building a Windows installer.
The installer includes a fixed WebView2 runtime and the Whisper `tiny.en`
English model. Missing resources stop packaging. There is no first-use
resource download or Evergreen runtime fallback.

## Acquisition {#desktop-resource-acquisition}

Run these commands from the repository root in Windows PowerShell 5.1 or
PowerShell 7:

```powershell
New-Item -ItemType Directory -Path target/resources -Force
./script/acquire-desktop-resources.ps1 -OutputDirectory target/resources/desktop
./script/acquire-desktop-resources.ps1 -OutputDirectory target/resources/desktop -VerifyOnly
```

Acquisition requires a fresh destination and an existing parent. It publishes
the destination only after verification succeeds. Use `-VerifyOnly` for an
existing destination. To retain verified downloads for later acquisitions,
create a cache folder and pass `-DownloadCacheDirectory` with its path. Cache
entries are named by their SHA-256 and are checked again before use.

The reviewed acquisition specification is
[`script/desktop-resources.json`](../../../script/desktop-resources.json).
It records every explicit HTTPS URL, version or immutable revision, byte count,
SHA-256, notice transformation, and verification evidence. Binary assets and
the generated inventory stay in ignored `target/resources/`.

## Reviewed assets {#reviewed-desktop-assets}

The 2026-10-04 review pins these downloaded files:

| Download                                          | Version or revision                                  |       Bytes |
| ------------------------------------------------- | ---------------------------------------------------- | ----------: |
| Microsoft fixed WebView2 x64 CAB                  | `154.0.4258.53`                                      | 307,996,323 |
| GGML Whisper `tiny.en`                            | `5359861c739e955e79d9a303bcbc70fb988958b1`           |  77,704,715 |
| Microsoft WebView2 SDK NuGet package, for notices | `1.0.3800.47`                                        |   8,973,762 |
| Microsoft fixed-runtime terms JSON                | Reviewed response, pinned by digest                  |      25,671 |
| OpenAI Whisper MIT license                        | `86098128c0b4f24f0e2aa2994de830614b474227`           |       1,063 |
| whisper.cpp MIT license                           | `2eeeba56e9edd762b4b38467bab96c2517163158` (`1.8.3`) |       1,078 |

The runtime archive SHA-256 is:

```text
ec12b2db6423d127fb8e1935d34e2e68abc70fe8ecb1f6162ba1c1ccc2825f6d
```

Microsoft's official download page supplies the explicit CAB URL. No separately
published SHA-256 was found. This digest is computed from the official download,
whose CAB Authenticode signature Windows verifies as `Valid`. The extracted
`msedgewebview2.exe` and `msedge.dll` also have valid signatures and report the
pinned product version. The signer is Microsoft Corporation, issued by
Microsoft Code Signing PCA 2024, with certificate thumbprint
`4028CAD637509D4744B17EC5B42AED8D7A31E6AF`. Acquisition checks the archive digest
and Microsoft signature before CAB extraction.

The model SHA-256 is:

```text
921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f
```

This digest and byte count are independently provided in the repository's
[immutable Git LFS pointer](https://huggingface.co/ggerganov/whisper.cpp/raw/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-tiny.en.bin).
The downloaded model matches both. Notice digests are computed from official
sources; their SHA-256 values are not independently published. The fixed-runtime
terms endpoint is not immutable, so changed response bytes fail verification
and require review rather than being accepted automatically.

The verified payload contains 257 unmodified runtime files, totaling
701,143,409 bytes. Model and notices bring the total to 263 files and
778,872,114 bytes, excluding the generated `manifest.json`. Verification pins
the entire runtime inventory, so deleting a locale or other runtime file and
regenerating the inventory still fails.

## Licenses {#desktop-resource-licenses}

The payload includes these notices, verified against the specification:

| Installed notice                    | Terms                                                      |  Bytes |
| ----------------------------------- | ---------------------------------------------------------- | -----: |
| `notices/WebView2.txt`              | Microsoft fixed-runtime license, rendered from `fixedHtml` | 16,468 |
| `notices/WebView2Loader.txt`        | Microsoft SDK `LICENSE.txt`, BSD-style three-clause terms  |  1,487 |
| `notices/WebView2Loader-NOTICE.txt` | Microsoft SDK third-party `NOTICE.txt`                     |  3,894 |
| `notices/Whisper.txt`               | OpenAI Whisper MIT license                                 |  1,063 |
| `notices/WhisperCpp.txt`            | whisper.cpp/ggml MIT license                               |  1,078 |

The loader notice comes from the SDK version used by `webview2-com-sys 0.39.1`.
MSVC builds statically link the supplied architecture-matched loader library.
Acquisition extracts only the SDK notices; it does not replace the dependency's
loader. The runtime's own embedded and standalone third-party notices remain
in the complete runtime tree. Normal generated Rust dependency licenses are
still part of the bundle process.

## Windows bundle and updates {#desktop-resource-packaging}

`script/bundle-windows.ps1` verifies resources before toolchain setup or builds,
copies them recursively into `inno/<architecture>/resources/desktop`, verifies
the copy, and checks it again immediately before invoking Inno Setup. It
generates Windows Whisper bindings with the MSVC/Windows SDK headers through
the exact LLVM/libclang prerequisite and keeps `GGML_NATIVE=OFF` for the
reviewed CPU-only native dependency. The native build still needs MSVC, CMake,
and LLVM/libclang.
Release jobs must run acquisition explicitly before calling the bundle script.

Optional environment variables:

- `KNIGHTCODE_DESKTOP_RESOURCES_DIR`: acquired resource folder; defaults to
  `target/resources/desktop`.
- `KNIGHTCODE_DESKTOP_RESOURCES_SPECIFICATION`: reviewed specification; defaults
  to `script/desktop-resources.json`.

The checked-in specification covers x64. An ARM64 bundle requires a separately
reviewed ARM64 specification and matching runtime. Selecting ARM64 with x64
resources fails preflight.

The installed layout is:

```text
KnightCode.exe
resources/desktop/manifest.json
resources/desktop/webview2/msedgewebview2.exe
resources/desktop/webview2/<complete fixed-runtime tree>
resources/desktop/speech/ggml-tiny.en.bin
resources/desktop/notices/<license and notice files>
```

Inno Setup recursively installs `resources/` through the update-aware
`GetInstallDir` path. During an update, this is `install/resources/`.
The updater backs up the existing `resources/` folder to `old/resources/`
and moves the new folder into place before removing `install/`. The existing
reverse-order rollback restores these moves on failure. The backup is optional
for installations that predate bundled resources; the new payload is required.

Both staging and installation grant runtime read/execute access to
`S-1-15-2-1` and `S-1-15-2-2`, as
[Microsoft requires for fixed runtimes on Windows 10](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#the-fixed-version-runtime-distribution-mode).
Installer extraction reapplies these permissions to the installed or update
staging path. The updater's same-volume directory rename retains them.
Uninstallation removes the resource folder, including files added by updates.

## Targeted checks {#desktop-resource-checks}

After acquisition, run:

```powershell
./script/test-desktop-resources.ps1
```

This test runs offline against actual acquired resources. It checks recursive
bundle staging and AppContainer permissions, runtime signatures and PE
architecture, model/license corruption, manifest version changes, complete
runtime inventory enforcement, untracked files, unsafe paths, ZIP traversal,
non-HTTPS URLs, corrupted archive-cache rejection before extraction, and failed
acquisition cleanup. It also checks the installer/updater path contract and
portable native build settings. Temporary test copies are removed afterward.

Acquisition and these tests passed under native Windows PowerShell 5.1 during
the review. Inno compilation, native application smoke tests, and an actual
update/rollback remain coordinator checks. Native prerequisite checks also
report missing MSVC Spectre libraries, a complete Windows SDK, PowerShell 7,
and Inno Setup 6. A full installer build has not been validated.

See [Windows development prerequisites](./windows.md) and the browser/speech
integration requirements in `crates/browser_preview/INTEGRATION.md` and
`crates/voice_input/INTEGRATION.md` before application integration testing.
