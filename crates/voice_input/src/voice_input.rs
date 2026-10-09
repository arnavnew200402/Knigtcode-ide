use std::{
    ffi::c_void,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow, bail};
use audio::AudioSettings;
use cpal::{
    Data, DeviceId, SampleFormat, SizedSample, Stream,
    traits::{DeviceTrait, StreamTrait},
};
use gpui::{AppContext, Entity, EventEmitter, Subscription, Task, actions};
use parking_lot::Mutex;
use serde::Deserialize;
use settings::Settings;
use sha2::{Digest, Sha256};
use ui::{ButtonLike, Tooltip, prelude::*};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

mod audio_samples;
use audio_samples::{Downmixer, resample_mono};

actions!(
    voice_input,
    [StartRecording, StopRecording, CancelRecording]
);

pub const MAX_RECORDING_SECONDS: u32 = 120;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoiceStatus {
    Idle,
    Starting,
    Recording,
    Transcribing,
    Cancelled,
    Error(String),
}

#[derive(Clone, Debug)]
pub enum VoiceEvent {
    StatusChanged(VoiceStatus),
    Transcribed(String),
}

pub type ComposerTextCallback = Box<dyn FnMut(String, &mut Window, &mut App) -> Result<()>>;

#[derive(Clone)]
pub struct SpeechModel {
    pub path: PathBuf,
    pub sha256: String,
}

#[derive(Deserialize)]
struct ResourceManifest {
    schema_version: u32,
    speech: SpeechManifest,
}

#[derive(Deserialize)]
struct SpeechManifest {
    model: String,
    sha256: String,
    license: String,
}

impl SpeechModel {
    pub fn bundled() -> Result<Self> {
        let executable = std::env::current_exe().context("Locating IDE executable")?;
        let parent = executable
            .parent()
            .context("Executable has no parent directory")?;
        let resources = parent.join("resources").join("desktop");
        let manifest: ResourceManifest = serde_json::from_slice(
            &std::fs::read(resources.join("manifest.json"))
                .context("Bundled speech manifest is missing; reinstall KnightCode")?,
        )?;
        if manifest.schema_version != 1
            || manifest.speech.model != "tiny.en"
            || manifest.speech.license != "MIT"
            || manifest.speech.sha256.len() != 64
            || !manifest
                .speech
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("Bundled speech model manifest is invalid");
        }
        let path = resources.join("speech").join("ggml-tiny.en.bin");
        if !path.is_file() {
            bail!("Bundled English speech model is missing; reinstall KnightCode");
        }
        Ok(Self {
            path,
            sha256: manifest.speech.sha256,
        })
    }

    fn load(&self, cancelled: &AtomicBool) -> Result<WhisperContext> {
        check_cancelled(cancelled)?;
        // Loading from a verified buffer also supports non-ASCII Windows install paths:
        // whisper.cpp's narrow fopen path does not reliably do so.
        let bytes = std::fs::read(&self.path).context("Reading bundled English speech model")?;
        check_cancelled(cancelled)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if !digest.eq_ignore_ascii_case(&self.sha256) {
            bail!("Bundled speech model checksum mismatch; reinstall KnightCode");
        }
        let mut parameters = WhisperContextParameters::default();
        parameters.use_gpu(false);
        WhisperContext::new_from_buffer_with_params(&bytes, parameters)
            .context("Loading bundled Whisper tiny.en model")
    }
}

struct MicrophoneCapture {
    stream: Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    cancelled: Arc<AtomicBool>,
}

impl MicrophoneCapture {
    fn start(
        device_id: Option<DeviceId>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<(Self, async_channel::Receiver<String>)> {
        check_cancelled(&cancelled)?;
        let device = audio::resolve_device(device_id.as_ref(), true)
            .context("Opening configured microphone")?;
        let supported = device
            .default_input_config()
            .context("Reading microphone capture format")?;
        let config = supported.config();
        if config.channels == 0
            || config.channels > 32
            || !(8_000..=192_000).contains(&config.sample_rate)
        {
            bail!(
                "Unsupported microphone format: {} channels at {} Hz",
                config.channels,
                config.sample_rate
            );
        }
        let format = supported.sample_format();
        if !matches!(
            format,
            SampleFormat::F32
                | SampleFormat::F64
                | SampleFormat::I8
                | SampleFormat::I16
                | SampleFormat::I32
                | SampleFormat::I64
                | SampleFormat::U8
                | SampleFormat::U16
                | SampleFormat::U32
                | SampleFormat::U64
        ) {
            bail!("Unsupported microphone sample format: {format}");
        }
        let samples = Arc::new(Mutex::new(Vec::with_capacity(
            config.sample_rate as usize * 10,
        )));
        let maximum_samples = config.sample_rate as usize * MAX_RECORDING_SECONDS as usize;
        let (sender, receiver) = async_channel::bounded(1);
        let callback_samples = samples.clone();
        let callback_cancelled = cancelled.clone();
        let error_sender = sender.clone();
        let channels = usize::from(config.channels);
        let mut downmixer = Downmixer::default();
        let stream = device
            .build_input_stream_raw(
                &config,
                format,
                move |data, _| {
                    if callback_cancelled.load(Ordering::Acquire) {
                        return;
                    }
                    let mut samples = callback_samples.lock();
                    let result = collect_samples(data, format, |sample| {
                        if !sample.is_finite() {
                            bail!("Microphone returned invalid samples");
                        }
                        if let Some(mono) = downmixer.push(sample.clamp(-1.0, 1.0), channels) {
                            if samples.len() >= maximum_samples {
                                bail!("Recording reached the {MAX_RECORDING_SECONDS}-second limit; stop and try a shorter dictation");
                            }
                            samples.push(mono);
                        }
                        Ok(())
                    });
                    if let Err(error) = result {
                        callback_cancelled.store(true, Ordering::Release);
                        report_capture_error(&sender, format!("{error:#}"));
                    }
                },
                move |error| {
                    report_capture_error(&error_sender, format!("Microphone capture failed: {error}"))
                },
                Some(Duration::from_secs(2)),
            )
            .context("Starting microphone capture; check Windows microphone access in Privacy settings")?;
        check_cancelled(&cancelled)?;
        stream
            .play()
            .context("Microphone access was denied or the device is unavailable")?;
        Ok((
            Self {
                stream,
                samples,
                sample_rate: config.sample_rate,
                cancelled,
            },
            receiver,
        ))
    }

    fn finish(self) -> Result<Vec<f32>> {
        if self.cancelled.swap(true, Ordering::AcqRel) {
            bail!("Microphone capture was interrupted; record again");
        }
        drop(self.stream);
        let samples = std::mem::take(&mut *self.samples.lock());
        if samples.len() < self.sample_rate as usize / 4 {
            bail!("Recording is too short; speak for at least a quarter second");
        }
        resample_mono(&samples, self.sample_rate).map_err(|error| anyhow!(error))
    }
}

fn report_capture_error(sender: &async_channel::Sender<String>, error: String) {
    match sender.try_send(error) {
        Ok(()) => {}
        Err(async_channel::TrySendError::Full(_)) => {}
        Err(async_channel::TrySendError::Closed(error)) => {
            log::debug!("Capture error after cancellation: {error}")
        }
    }
}

fn collect_samples(
    data: &Data,
    format: SampleFormat,
    mut append: impl FnMut(f32) -> Result<()>,
) -> Result<()> {
    fn convert<T: SizedSample>(
        data: &Data,
        append: &mut impl FnMut(f32) -> Result<()>,
    ) -> Result<()>
    where
        f32: cpal::FromSample<T>,
    {
        let samples = data
            .as_slice::<T>()
            .context("Microphone sample format changed")?;
        for sample in samples {
            append(sample.to_sample::<f32>())?;
        }
        Ok(())
    }
    match format {
        SampleFormat::F32 => convert::<f32>(data, &mut append),
        SampleFormat::F64 => convert::<f64>(data, &mut append),
        SampleFormat::I8 => convert::<i8>(data, &mut append),
        SampleFormat::I16 => convert::<i16>(data, &mut append),
        SampleFormat::I32 => convert::<i32>(data, &mut append),
        SampleFormat::I64 => convert::<i64>(data, &mut append),
        SampleFormat::U8 => convert::<u8>(data, &mut append),
        SampleFormat::U16 => convert::<u16>(data, &mut append),
        SampleFormat::U32 => convert::<u32>(data, &mut append),
        SampleFormat::U64 => convert::<u64>(data, &mut append),
        _ => bail!("Unsupported microphone sample format: {format}"),
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    if cancelled.load(Ordering::Acquire) {
        bail!("Voice input cancelled");
    }
    Ok(())
}

unsafe extern "C" fn abort_transcription(user_data: *mut c_void) -> bool {
    // The pointed-to AtomicBool is owned by an Arc held until state.full returns.
    unsafe { &*user_data.cast::<AtomicBool>() }.load(Ordering::Acquire)
}

fn transcribe(model: &SpeechModel, samples: &[f32], cancelled: Arc<AtomicBool>) -> Result<String> {
    check_cancelled(&cancelled)?;
    if samples.iter().all(|sample| sample.abs() < 0.0001) {
        bail!("No speech was recorded; check your microphone");
    }
    let context = model.load(&cancelled)?;
    check_cancelled(&cancelled)?;
    let mut state = context
        .create_state()
        .context("Creating local transcription state")?;
    let mut parameters = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    parameters.set_language(Some("en"));
    parameters.set_n_threads(4);
    parameters.set_no_context(true);
    parameters.set_no_timestamps(true);
    parameters.set_print_progress(false);
    parameters.set_print_realtime(false);
    parameters.set_print_special(false);
    parameters.set_print_timestamps(false);
    parameters.set_suppress_nst(true);
    // 0.16.0's safe abort wrapper boxes a trait object but casts it back to the
    // concrete closure type. Use the raw callback with an explicitly owned pointer.
    unsafe {
        parameters.set_abort_callback(Some(abort_transcription));
        parameters.set_abort_callback_user_data(Arc::as_ptr(&cancelled).cast_mut().cast());
    }
    state
        .full(parameters, samples)
        .context("Transcribing microphone audio locally")?;
    check_cancelled(&cancelled)?;
    let mut text = String::new();
    for segment in state.as_iter() {
        text.push_str(segment.to_str().context("Decoding transcription text")?);
    }
    let text = text.trim().to_owned();
    if text.is_empty() {
        bail!("No speech was recognized; try again");
    }
    Ok(text)
}

pub struct VoiceInput {
    status: VoiceStatus,
    compact: bool,
    copper: bool,
    capture: Option<MicrophoneCapture>,
    cancelled: Arc<AtomicBool>,
    generation: u64,
    callback: ComposerTextCallback,
    task: Option<Task<()>>,
    error_task: Option<Task<()>>,
    _release: Subscription,
}

impl VoiceInput {
    pub fn new(cx: &mut Context<Self>, callback: ComposerTextCallback) -> Self {
        let release = cx.on_release(|this, _| {
            this.cancelled.store(true, Ordering::Release);
            this.capture.take();
        });
        Self {
            status: VoiceStatus::Idle,
            compact: false,
            copper: false,
            capture: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            generation: 0,
            callback,
            task: None,
            error_task: None,
            _release: release,
        }
    }

    pub fn status(&self) -> &VoiceStatus {
        &self.status
    }

    pub fn set_compact(&mut self, compact: bool, cx: &mut Context<Self>) {
        if self.compact != compact {
            self.compact = compact;
            cx.notify();
        }
    }

    pub fn set_copper(&mut self, copper: bool, cx: &mut Context<Self>) {
        if self.copper != copper {
            self.copper = copper;
            cx.notify();
        }
    }

    fn set_status(&mut self, status: VoiceStatus, cx: &mut Context<Self>) {
        self.status = status.clone();
        cx.emit(VoiceEvent::StatusChanged(status));
        cx.notify();
    }

    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.status,
            VoiceStatus::Starting | VoiceStatus::Recording | VoiceStatus::Transcribing
        ) {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = self.cancelled.clone();
        let device = AudioSettings::get_global(cx).input_audio_device.clone();
        self.set_status(VoiceStatus::Starting, cx);
        let capture = cx.background_spawn(async move {
            // Fail before opening the microphone if the installed bundle is incomplete.
            SpeechModel::bundled()?;
            MicrophoneCapture::start(device, cancelled)
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = capture.await;
            if let Err(error) = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((capture, errors)) => {
                        this.capture = Some(capture);
                        this.set_status(VoiceStatus::Recording, cx);
                        this.error_task = Some(cx.spawn_in(window, async move |this, cx| {
                            if let Ok(error) = errors.recv().await {
                                if let Err(update_error) = this.update(cx, |this, cx| {
                                    if this.generation == generation
                                        && this.status == VoiceStatus::Recording
                                    {
                                        this.cancelled.store(true, Ordering::Release);
                                        this.capture.take();
                                        this.set_status(VoiceStatus::Error(error), cx);
                                    }
                                }) {
                                    log::debug!("Voice input target closed: {update_error:#}");
                                }
                            }
                        }));
                    }
                    Err(error) => this.set_status(VoiceStatus::Error(format!("{error:#}")), cx),
                }
            }) {
                log::debug!("Voice input target closed: {error:#}");
            }
        }));
    }

    pub fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(capture) = self.capture.take() else {
            return;
        };
        self.error_task.take();
        let generation = self.generation;
        self.cancelled = Arc::new(AtomicBool::new(false));
        let cancelled = self.cancelled.clone();
        self.set_status(VoiceStatus::Transcribing, cx);
        let transcription = cx.background_spawn(async move {
            let samples = capture.finish()?;
            // Capture uses the same flag to stop its callback. Inference has its own
            // cancellation flag, so stopping capture never cancels transcription.
            transcribe(&SpeechModel::bundled()?, &samples, cancelled)
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = transcription.await;
            if let Err(error) = this.update_in(cx, |this, window, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok(text) => match (this.callback)(text.clone(), window, cx) {
                        Ok(()) => {
                            cx.emit(VoiceEvent::Transcribed(text));
                            this.set_status(VoiceStatus::Idle, cx);
                        }
                        Err(error) => this.set_status(
                            VoiceStatus::Error(format!("Inserting editable dictation: {error:#}")),
                            cx,
                        ),
                    },
                    Err(error) => this.set_status(VoiceStatus::Error(format!("{error:#}")), cx),
                }
            }) {
                log::debug!("Voice input target closed: {error:#}");
            }
        }));
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.cancelled.store(true, Ordering::Release);
        self.capture.take();
        self.error_task.take();
        self.task.take();
        self.set_status(VoiceStatus::Cancelled, cx);
    }
}

impl EventEmitter<VoiceEvent> for VoiceInput {}

impl Render for VoiceInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let recording = self.status == VoiceStatus::Recording;
        let busy = matches!(
            self.status,
            VoiceStatus::Starting | VoiceStatus::Transcribing
        );
        let label = match &self.status {
            VoiceStatus::Idle => "Dictate (English, on device)".to_owned(),
            VoiceStatus::Starting => "Opening microphone…".to_owned(),
            VoiceStatus::Recording => format!("Recording (up to {MAX_RECORDING_SECONDS}s)"),
            VoiceStatus::Transcribing => "Transcribing locally…".to_owned(),
            VoiceStatus::Cancelled => "Dictation cancelled".to_owned(),
            VoiceStatus::Error(error) => error.clone(),
        };
        if self.compact {
            return h_flex()
                .gap_1()
                .on_action(
                    cx.listener(|this, _: &StartRecording, window, cx| this.start(window, cx)),
                )
                .on_action(cx.listener(|this, _: &StopRecording, window, cx| this.stop(window, cx)))
                .on_action(cx.listener(|this, _: &CancelRecording, _, cx| this.cancel(cx)))
                .child(
                    div()
                        .size(px(48.))
                        .rounded_full()
                        .border_1()
                        .border_color(gpui::rgba(0x8c83af30))
                        .when(self.copper, |button| {
                            button
                                .w(px(54.))
                                .h(px(52.))
                                .rounded_lg()
                                .border_color(gpui::rgba(0))
                        })
                        .overflow_hidden()
                        .child(
                            ButtonLike::new("chat-voice-record")
                                .full_width()
                                .height(px(if self.copper { 52. } else { 48. }).into())
                                .style(ButtonStyle::Transparent)
                                .disabled(busy)
                                .tooltip(Tooltip::text(label))
                                .child(
                                    Icon::new(if recording {
                                        IconName::Stop
                                    } else {
                                        IconName::Mic
                                    })
                                    .size(IconSize::Medium)
                                    .color(
                                        if recording || matches!(self.status, VoiceStatus::Error(_))
                                        {
                                            Color::Error
                                        } else {
                                            Color::Custom(
                                                gpui::rgb(if self.copper {
                                                    0xffd09b
                                                } else {
                                                    0xbcb7da
                                                })
                                                .into(),
                                            )
                                        },
                                    ),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if recording {
                                        this.stop(window, cx)
                                    } else {
                                        this.start(window, cx)
                                    }
                                })),
                        ),
                )
                .when(recording || busy, |controls| {
                    controls.child(
                        IconButton::new("chat-voice-cancel", IconName::Close)
                            .tooltip(Tooltip::text("Cancel dictation"))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    )
                })
                .into_any_element();
        }
        h_flex()
            .gap_1()
            .on_action(cx.listener(|this, _: &StartRecording, window, cx| this.start(window, cx)))
            .on_action(cx.listener(|this, _: &StopRecording, window, cx| this.stop(window, cx)))
            .on_action(cx.listener(|this, _: &CancelRecording, _, cx| this.cancel(cx)))
            .child(
                Button::new(
                    "voice-record",
                    if recording {
                        "Stop Dictation"
                    } else {
                        "Dictate"
                    },
                )
                .disabled(busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    if recording {
                        this.stop(window, cx)
                    } else {
                        this.start(window, cx)
                    }
                })),
            )
            .when(recording || busy, |element| {
                element.child(
                    Button::new("voice-cancel", "Cancel")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                )
            })
            .child(Label::new(label).size(LabelSize::Small).color(
                if matches!(self.status, VoiceStatus::Error(_)) {
                    Color::Error
                } else {
                    Color::Muted
                },
            ))
            .into_any_element()
    }
}

pub fn create(
    cx: &mut App,
    callback: impl FnMut(String, &mut Window, &mut App) -> Result<()> + 'static,
) -> Entity<VoiceInput> {
    cx.new(|cx| VoiceInput::new(cx, Box::new(callback)))
}
