# Composer dictation integration

`voice_input::create(cx, callback)` returns an `Entity<VoiceInput>` that renders
Dictate, Stop Dictation, Cancel and explicit recording/transcription/error state.
Keep it in the composer's entity and render it next to the existing composer
controls. There is no global initialization requirement beyond the IDE's
existing audio settings initialization.

```rust
let composer = cx.weak_entity();
let voice = voice_input::create(cx, move |text, window, cx| {
    composer.update(cx, |composer, cx| {
        // Insert or append at the current cursor using the editor's text-edit API.
        // Keep all existing draft text/attachments and allow editing this text.
        composer.insert_dictated_text(&text, window, cx)
    })?
});
```

The callback returns `anyhow::Result<()>`. It runs on GPUI's foreground thread
with the window and app context. It must edit the draft only, without invoking
the send/submit path. Routing is bound to the composer that began dictation,
not whichever composer happens to be active later. Errors are surfaced in the
voice control. `VoiceEvent::Transcribed(String)` is emitted only after insertion
succeeds; subscribe to `VoiceEvent::StatusChanged(VoiceStatus)` for host UI state.

Public controls are `start(window, cx)`, `stop(window, cx)`, `cancel(cx)` and
`status()`. Corresponding actions are `StartRecording`, `StopRecording` and
`CancelRecording`; host keybindings can dispatch them. Recording is capped at
120 seconds, errors on device disconnect or malformed samples, and cancellation
invalidates pending completions. Closing the composer stops capture and aborts
inference. Device capture, resampling, checksum validation, model loading and
Whisper inference run outside the UI thread.

GPUI does not expose a standalone audio-input service. This crate reuses the
existing `audio::resolve_device` and `AudioSettings::input_audio_device` with
CPAL 0.17 raw capture. Channel frames are averaged to mono, including frames
split across callback boundaries. A low-pass sinc resampler produces 16 kHz
float samples; integer formats are normalized via CPAL's sample conversion.
Silent input is rejected before inference. Only the bundled English tiny.en
model is supported initially.

See `../browser_preview/INTEGRATION.md` for workspace registration, exact
installed resource paths, acquisition specification and native build review.
Do not add a first-use model download. Missing or checksum-mismatched bundled
resources are visible errors.
