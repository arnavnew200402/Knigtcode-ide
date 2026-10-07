# Desktop preview integration

Add `crates/browser_preview` and `crates/voice_input` to workspace members,
and these entries to workspace dependencies:

```toml
browser_preview = { path = "crates/browser_preview" }
voice_input = { path = "crates/voice_input" }
```

The new crates pin their external dependencies directly:
`webview2-com = "=0.39.1"` and `whisper-rs = "=0.16.0"` with default features
disabled. Direct sys dependencies pin the inspected native implementations to
`webview2-com-sys = "=0.39.1"` and `whisper-rs-sys = "=0.15.0"`.
They reuse workspace Windows 0.62, raw-window-handle 0.6 and CPAL 0.17.
No new external root dependency declarations are required. Add the workspace
dependencies to the application and to the composer crate where used.

Call `browser_preview::init(cx)` after editor/UI initialization and before opening
workspaces. `browser_preview::OpenPreview` is a command-palette action. It opens a
real `BrowserPreview` workspace Item at `http://localhost:3000`. For a specific
URL, call:

```rust
browser_preview::open_preview(workspace, url, window, cx)?;
```

The URL field supports Enter and Go. The Item includes history navigation,
reload/recovery, explicit Open External, native page focus and page-only PNG
capture. It uses `raw_window_handle::HasWindowHandle` to parent a WebView2
controller to the Win32 window and updates device-pixel bounds and rasterization
scale from GPUI.
Moving an Item between windows reparents the controller. Tab deactivation hides
the native child; dropping the Item closes the controller. WebView2 startup uses
GPUI's existing OLE STA message loop, without a nested blocking message pump.

## Screenshot attachment

Install one application-level handler after the agent UI is initialized:

```rust
browser_preview::PreviewAttachmentService::install(cx, |screenshot, window, cx| {
    // Route to the active workspace's editable agent composer.
    // screenshot.png: Arc<[u8]>, screenshot.page_url: String
    // screenshot.width / height: u32
    // MIME: PreviewScreenshot::MIME_TYPE (image/png)
    // Name: PreviewScreenshot::FILE_NAME (page-preview.png)
    attach_to_composer(screenshot, window, cx)
});
```

The callback must add an attachment without submitting a prompt. It should
return an error if there is no suitable composer, preserving the screenshot
failure in the preview UI. `BrowserPreview::attach_screenshot(window, cx)` and
the `AttachScreenshot` action use that handler. `capture_page(cx)` instead
returns `Task<Result<PreviewScreenshot>>` for callers that manage their own
attachment flow. `PreviewEvent::ScreenshotReady` is emitted after successful
attachment. Capture uses WebView2 `CapturePreview(PNG)`, not the IDE window or
desktop; URL changes during capture produce a retryable error.

## Native child window overlap

Windows HWND children render above GPUI's GPU scene. The Item automatically
hides them while a workspace modal, standard `ContextMenu`, GPUI prompt or drag
is active. Integrators rendering non-modal overlays with a custom key context,
or non-focus-taking tooltips above the browser region, must call
`preview.update(cx, |preview, cx| preview.set_overlay_suspended(true, cx))`
before showing them, and restore `false` after dismissal. This is required for
correct hit testing and layering; a GPU z-index alone cannot cover an HWND.

Cached panes do not execute their canvas on every window frame. A foreground
visibility task checks overlay state every 50 ms without invalidating the GPU
scene, and native focus loss hides immediately. The task and native event routing
follow the Item's current window after workspace moves. Explicit suspension is
the immediate path for custom overlay hosts.
The page canvas also retains its GPUI hitbox; absence from the rendered frame
hides the HWND when another pane is maximized or the workspace is hidden.

The preview does not expose host objects or invoke an agent from web content.
Only HTTP(S) URLs are accepted; page-created windows are navigated in this
embedded host. Missing runtime resources produce a visible error with Reload
recovery. Non-Windows hosts report that embedded preview is unsupported.

## Bundled resources

Installed layout:

```text
KnightCode.exe
resources/desktop/manifest.json
resources/desktop/webview2/msedgewebview2.exe
resources/desktop/webview2/<all other fixed-runtime files>
resources/desktop/speech/ggml-tiny.en.bin
resources/desktop/notices/WebView2.txt
resources/desktop/notices/WebView2Loader.txt
resources/desktop/notices/Whisper.txt
```

For Windows MSVC, the reviewed sys crate statically links its architecture-matched
Microsoft `WebView2LoaderStatic.lib`; no loader DLL or SDK acquisition is needed.
GNU Windows builds would require a separately bundled loader DLL and are outside
this initial MSVC packaging path. Ship all fixed-runtime contents, including
locale and resource files.

`script/acquire-desktop-resources.ps1` consumes a reviewed acquisition JSON,
downloads only at packaging time, verifies each explicit SHA-256 digest before
extraction, stores license notices and writes a per-file inventory. Large
binaries stay outside source control. Specification shape:

```text
schema_version: 1
architecture: "x64" or "arm64"
webview2:
  url: explicit HTTPS fixed-runtime CAB/ZIP URL
  sha256: independently verified 64-digit archive SHA-256
  format: "cab" or "zip"
  version: exact msedgewebview2.exe ProductVersion
  notice: { url: explicit HTTPS distribution-terms URL, sha256: verified digest }
  loader_notice: { url: explicit HTTPS Microsoft WebView2 SDK license URL, sha256: verified digest }
speech:
  model: "tiny.en"
  license: "MIT"
  url: explicit HTTPS ggml-tiny.en.bin URL (prefer an immutable revision)
  sha256: independently verified model SHA-256
  notice: { url: explicit HTTPS Whisper MIT license URL, sha256: verified digest }
```

Acquire to a fresh destination whose parent already exists:

```powershell
./script/acquire-desktop-resources.ps1 -SpecificationPath $reviewedSpecification -OutputDirectory $desktopResources
./script/acquire-desktop-resources.ps1 -OutputDirectory $desktopResources -VerifyOnly
```

The coordinator's bundle script must verify the directory, copy it to
`resources/desktop`, and preserve or reapply runtime read/execute ACLs for
`S-1-15-2-1` and `S-1-15-2-2` on Windows 10.
Run verification again against the staged bundle before making the installer.
Neither runtime code path downloads missing assets or falls back to Evergreen.

## Dependency/build review

Reviewed crates.io release metadata and downloaded sources for webview2-com
0.39.1, webview2-com-sys 0.39.1, whisper-rs 0.16.0 and whisper-rs-sys 0.15.0.
WebView2 uses Windows 0.62, supports x64/arm64 MSVC, and ships prebuilt Microsoft
loader libraries/DLLs in its sys crate; its build script copies them into OUT_DIR
and links advapi32. The sys crate's `link_webview2` macro selects
`WebView2LoaderStatic` for MSVC. It performs no network download. The resource
script acquires the SDK license through `webview2.loader_notice` for the
statically linked Microsoft loader; crates.io's MIT metadata for the bindings
does not replace that notice.

Whisper's sys build copies bundled whisper.cpp into OUT_DIR and invokes CMake
to build static CPU libraries. It must generate bindings with bindgen/libclang;
the shipped `src/bindings.rs` was generated against Linux glibc and is not a
valid Windows MSVC ABI. Windows setup installs the exact LLVM/libclang version
used by the repository's prerequisite script, sets `LIBCLANG_PATH`, and passes
the MSVC target triple through `BINDGEN_EXTRA_CLANG_ARGS` while the Visual
Studio shell supplies the MSVC and Windows SDK headers. Do not set
`WHISPER_DONT_GENERATE_BINDINGS=1` on Windows. For portable release binaries
set `GGML_NATIVE=OFF` so the build machine's CPU extensions are not baked into
the distributed binary. No CUDA, OpenMP, Vulkan or model download is enabled by
these manifests. The speech model is SHA-256 checked in a background task
before loading. The Rust wrapper is Unlicense; whisper.cpp/Whisper weights are
MIT; WebView2 runtime and loader have Microsoft distribution terms. Include
normal generated Rust dependency notices as well as the downloaded resource
notices.

The reviewed Whisper 0.16.0 safe abort callback boxes a trait object and casts
it to a concrete closure pointer. This implementation avoids that wrapper and
uses its raw abort callback API with an AtomicBool owned through inference.

## Coordinator verification

Passed six standalone Rust tests for mono downmixing, resampling duration/DC,
anti-alias filtering, invalid input and screenshot header/truncation/empty bounds.
They were cross-compiled to Linux musl using the installed Rust 1.97.1 compiler
and executed under WSL. Also passed PowerShell syntax and manifest verification
tests, including checksum tampering, architecture mismatch, manifest/archive
path traversal and non-HTTPS URL rejection.

Rustfmt checks passed for both crate roots. Whitespace checks also covered the
new, untracked files. A standalone Windows link attempt failed because the
Windows SDK's `kernel32.lib` was unavailable. Full crate/application type checks
and native smoke tests remain unverified.

Run the repository's Windows-native toolchain setup, then focused checks/tests
for these two crates, followed by the application check. No cargo build was
started by the feature implementer. Native smoke tests still need a correctly
staged bundle: local HTTP navigation/history/reload/error recovery, page-only
screenshot attachment, 100/150/200% DPI and window moves, tab switching and
modal/menu overlap, denied microphone access, stop/cancel during loading and
inference, and editable English dictation without submission.
