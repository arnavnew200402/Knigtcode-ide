//! The only place in the fork that sends an event.
//!
//! Architecture §12.1 is a closed allowlist, and this module is how it is
//! enforced in code rather than in review: the event is an enum with one
//! variant per allowed row, every property is an enum rendered to a fixed
//! string, and there is no `&str` event name and no generic property map. A
//! call site cannot invent an event, and a reviewer sees the whole surface in
//! one screen.
//!
//! The install ping deliberately lives in the engine instead
//! (`packages/cli/src/engine/install-report.ts`), where the shared opt-out
//! already is. These four cannot: `ide_engine_failed` fires precisely when the
//! engine is not running, and first-run steps happen before it may be up. So
//! the IDE keeps a mirror of the answer in its own key-value store, and
//! **absent means do not send** — a machine that has never been asked reports
//! nothing.

use collections::HashSet;
use gpui::{App, BackgroundExecutor, Global};
use http_client::{AsyncBody, HttpClient, Method, Request};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

use crate::process::StartError;

const REPORT_URL: &str = "https://knightcode.dev/api/report-install";
const TIMEOUT: Duration = Duration::from_secs(5);

/// The mirror of the engine's `enableInstallTelemetry`, written when first run
/// answers the question and refreshed whenever the engine reports a change.
pub const CONSENT_KEY: &str = "knightcode_telemetry_consent";

/// How far first run got. Reported with the outcome so a launch-day drop-off
/// can be located rather than merely counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstRunStep {
    Welcome,
    SignIn,
    Model,
    Open,
}

impl FirstRunStep {
    fn as_str(self) -> &'static str {
        match self {
            FirstRunStep::Welcome => "welcome",
            FirstRunStep::SignIn => "sign_in",
            FirstRunStep::Model => "model",
            FirstRunStep::Open => "open",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FirstRunOutcome {
    Completed,
    Abandoned,
}

impl FirstRunOutcome {
    fn as_str(self) -> &'static str {
        match self {
            FirstRunOutcome::Completed => "completed",
            FirstRunOutcome::Abandoned => "abandoned",
        }
    }
}

/// Why the engine never reached ready. Mapped from a [`StartError`] variant,
/// never formatted from one: every `StartError` message carries a path, and a
/// path carries a username.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineFailure {
    /// `locate_binary` found nothing. Reported before a process exists.
    BinaryMissing,
    SpawnFailed,
    ExitedEarly,
    Timeout,
    TokenRejected,
    Unreachable,
}

impl EngineFailure {
    fn as_str(self) -> &'static str {
        match self {
            EngineFailure::BinaryMissing => "binary_missing",
            EngineFailure::SpawnFailed => "spawn_failed",
            EngineFailure::ExitedEarly => "exited_early",
            EngineFailure::Timeout => "timeout",
            EngineFailure::TokenRejected => "token_rejected",
            EngineFailure::Unreachable => "unreachable",
        }
    }
}

impl From<&StartError> for EngineFailure {
    fn from(error: &StartError) -> Self {
        match error {
            StartError::Spawn { .. } => EngineFailure::SpawnFailed,
            StartError::ExitedBeforeReady { .. } => EngineFailure::ExitedEarly,
            StartError::Stalled { .. } => EngineFailure::Timeout,
            StartError::TokenRejected { .. } => EngineFailure::TokenRejected,
            StartError::Io(_) => EngineFailure::Unreachable,
        }
    }
}

/// The five surfaces §15 says one login must serve. Reported once per version
/// each, which is the difference between a launch signal and usage tracking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seam {
    AgentPanel,
    BufferInlineAssist,
    TerminalInlineAssist,
    CommitMessage,
    EditPrediction,
}

impl Seam {
    fn as_str(self) -> &'static str {
        match self {
            Seam::AgentPanel => "agent_panel",
            Seam::BufferInlineAssist => "buffer_inline_assist",
            Seam::TerminalInlineAssist => "terminal_inline_assist",
            Seam::CommitMessage => "commit_message",
            Seam::EditPrediction => "edit_prediction",
        }
    }
}

/// The whole reportable surface of the fork. Adding a variant is an amendment
/// to architecture §12.1, not a commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    FirstRun {
        outcome: FirstRunOutcome,
        step: FirstRunStep,
    },
    EngineFailed {
        reason: EngineFailure,
    },
    /// `provider` is the engine's provider id, which is data rather than a
    /// fixed list: the fork holds no per-provider table. It is sanitised on the
    /// way out and truncated again by the route.
    FirstTurn {
        provider: String,
    },
    SeamFirstUse {
        seam: Seam,
    },
}

impl Event {
    fn name(&self) -> &'static str {
        match self {
            Event::FirstRun { .. } => "ide_first_run",
            Event::EngineFailed { .. } => "ide_engine_failed",
            Event::FirstTurn { .. } => "ide_first_turn",
            Event::SeamFirstUse { .. } => "ide_seam_first_use",
        }
    }

    fn properties(&self) -> Vec<(&'static str, String)> {
        match self {
            Event::FirstRun { outcome, step } => vec![
                ("outcome", outcome.as_str().to_owned()),
                ("step", step.as_str().to_owned()),
            ],
            Event::EngineFailed { reason } => vec![("reason", reason.as_str().to_owned())],
            Event::FirstTurn { provider } => vec![("provider", sanitize(provider))],
            Event::SeamFirstUse { seam } => vec![("seam", seam.as_str().to_owned())],
        }
    }

    /// The key under which "already reported for this version" is remembered,
    /// or `None` for an event that may fire more than once.
    fn once_key(&self) -> Option<String> {
        match self {
            Event::FirstRun { .. } | Event::EngineFailed { .. } => None,
            Event::FirstTurn { .. } => Some("knightcode_reported_first_turn".to_owned()),
            Event::SeamFirstUse { seam } => {
                Some(format!("knightcode_reported_seam_{}", seam.as_str()))
            }
        }
    }
}

/// Provider ids are the one value that is not a closed enum, so it is the one
/// value that needs a guard: lowercase ASCII word characters only, truncated.
/// Nothing else can reach a property.
fn sanitize(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .flat_map(char::to_lowercase)
        .collect()
}

/// Where the consent mirror and the once-per-version marks are kept. A trait so
/// a test does not need the real database, and so this module does not care
/// that the real one is `db::kvp::GlobalKeyValueStore`.
pub trait ReportStore: Send + Sync + 'static {
    fn read(&self, key: &str) -> Option<String>;
    fn write(&self, key: String, value: String);
}

pub struct Reporter {
    http: Arc<dyn HttpClient>,
    executor: BackgroundExecutor,
    version: Arc<str>,
    store: Arc<dyn ReportStore>,
    /// Mirrors `store` for the current process, so a burst of seam events does
    /// not race through the store's asynchronous writes.
    reported: Mutex<HashSet<String>>,
}

struct GlobalReporter(Arc<Reporter>);

impl Global for GlobalReporter {}

impl Reporter {
    pub fn new(
        http: Arc<dyn HttpClient>,
        executor: BackgroundExecutor,
        version: impl Into<Arc<str>>,
        store: Arc<dyn ReportStore>,
    ) -> Self {
        Self {
            http,
            executor,
            version: version.into(),
            store,
            reported: Mutex::new(HashSet::default()),
        }
    }

    /// `None` until first run has asked. Absent means do not send.
    ///
    /// `KNIGHTCODE_OFFLINE` is a hard no, as it is for the CLI and for the
    /// engine's install ping: §12.1 binds all three to the same switches.
    pub fn consent(&self) -> Option<bool> {
        if std::env::var_os("KNIGHTCODE_OFFLINE").is_some() {
            return Some(false);
        }
        match self.store.read(CONSENT_KEY)?.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }

    /// Records the answer first run collected, and the engine's answer whenever
    /// it changes, so the two never disagree for long.
    pub fn set_consent(&self, enabled: bool) {
        self.store
            .write(CONSENT_KEY.to_owned(), enabled.to_string());
    }

    /// The request this event would produce, or `None` if it would not be sent.
    /// Separate from [`Reporter::report`] so a test can pin the whole payload
    /// without a network client.
    pub fn build_request(&self, event: &Event) -> Option<Request<AsyncBody>> {
        if self.consent() != Some(true) {
            return None;
        }
        if let Some(key) = event.once_key() {
            let versioned = format!("{key}:{}", self.version);
            if self.reported.lock().contains(&versioned)
                || self.store.read(&key).as_deref() == Some(self.version.as_ref())
            {
                return None;
            }
            self.reported.lock().insert(versioned);
            self.store.write(key, self.version.to_string());
        }

        let mut uri = format!(
            "{REPORT_URL}?version={}&event={}",
            encode(&self.version),
            event.name()
        );
        for (name, value) in event.properties() {
            uri.push('&');
            uri.push_str(name);
            uri.push('=');
            uri.push_str(&encode(&value));
        }

        Request::builder()
            .method(Method::GET)
            .uri(uri)
            .header(
                "User-Agent",
                format!(
                    "knightcode-ide/{} ({}; rust; {})",
                    self.version,
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
            )
            .body(AsyncBody::empty())
            .ok()
    }

    /// Fire-and-forget. Nothing in startup, first run or the engine's lifecycle
    /// waits on this, and a failure is not an error anyone can act on.
    pub fn report(&self, event: Event) {
        let Some(request) = self.build_request(&event) else {
            return;
        };
        let http = self.http.clone();
        self.executor
            .spawn(async move {
                let send = http.send(request);
                futures::future::select(Box::pin(send), Box::pin(smol::Timer::after(TIMEOUT)))
                    .await;
            })
            .detach();
    }

    pub fn set_global(reporter: Arc<Reporter>, cx: &mut App) {
        cx.set_global(GlobalReporter(reporter));
    }

    pub fn try_global(cx: &App) -> Option<Arc<Reporter>> {
        cx.try_global::<GlobalReporter>()
            .map(|global| global.0.clone())
    }
}

/// The one call every site uses. A missing reporter — a test app, a headless
/// context — is silence, not a panic.
pub fn report(event: Event, cx: &App) {
    if let Some(reporter) = Reporter::try_global(cx) {
        reporter.report(event);
    }
}

/// Minimal percent-encoding for a query value. Every value we send is already
/// `[a-z0-9_-]` or a semver string; this exists so a provider id that slipped
/// through cannot break the URL.
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// The real store. Reads are synchronous; writes are spawned, because the
/// caller is often on a path that must not block.
pub struct KeyValueReportStore {
    executor: BackgroundExecutor,
}

impl KeyValueReportStore {
    pub fn new(executor: BackgroundExecutor) -> Self {
        Self { executor }
    }
}

impl ReportStore for KeyValueReportStore {
    fn read(&self, key: &str) -> Option<String> {
        db::kvp::GlobalKeyValueStore::global()
            .read_kvp(key)
            .ok()
            .flatten()
    }

    fn write(&self, key: String, value: String) {
        self.executor
            .spawn(async move {
                if let Err(error) = db::kvp::GlobalKeyValueStore::global()
                    .write_kvp(key, value)
                    .await
                {
                    log::error!("could not record a launch signal: {error}");
                }
            })
            .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collections::HashMap;
    use std::path::PathBuf;

    #[derive(Default)]
    struct MemoryStore(Mutex<HashMap<String, String>>);

    impl ReportStore for MemoryStore {
        fn read(&self, key: &str) -> Option<String> {
            self.0.lock().get(key).cloned()
        }
        fn write(&self, key: String, value: String) {
            self.0.lock().insert(key, value);
        }
    }

    fn reporter(consent: Option<bool>) -> (Arc<Reporter>, Arc<MemoryStore>) {
        let store = Arc::new(MemoryStore::default());
        if let Some(consent) = consent {
            store.write(CONSENT_KEY.to_owned(), consent.to_string());
        }
        let executor = gpui::BackgroundExecutor::new(Arc::new(gpui::TestDispatcher::new(0)));
        let reporter = Arc::new(Reporter::new(
            http_client::FakeHttpClient::with_200_response(),
            executor,
            "1.0.0+stable",
            store.clone(),
        ));
        (reporter, store)
    }

    fn every_event() -> Vec<Event> {
        vec![
            Event::FirstRun {
                outcome: FirstRunOutcome::Completed,
                step: FirstRunStep::Open,
            },
            Event::EngineFailed {
                reason: EngineFailure::BinaryMissing,
            },
            Event::FirstTurn {
                provider: "anthropic".to_owned(),
            },
            Event::SeamFirstUse {
                seam: Seam::CommitMessage,
            },
        ]
    }

    #[test]
    fn an_unanswered_machine_reports_nothing() {
        let (reporter, _) = reporter(None);
        for event in every_event() {
            assert!(reporter.build_request(&event).is_none(), "{event:?}");
        }
    }

    #[test]
    fn an_offline_machine_reports_nothing_however_it_answered() {
        // SAFETY: single-threaded test, and the variable is removed before it
        // returns, so no other test observes it.
        unsafe { std::env::set_var("KNIGHTCODE_OFFLINE", "1") };
        let (reporter, _) = reporter(Some(true));
        for event in every_event() {
            assert!(reporter.build_request(&event).is_none(), "{event:?}");
        }
        unsafe { std::env::remove_var("KNIGHTCODE_OFFLINE") };
    }

    #[test]
    fn a_declining_machine_reports_nothing() {
        let (reporter, _) = reporter(Some(false));
        for event in every_event() {
            assert!(reporter.build_request(&event).is_none(), "{event:?}");
        }
    }

    #[test]
    fn a_consenting_machine_reports_each_event_once_where_that_is_the_rule() {
        let (reporter, _) = reporter(Some(true));
        for event in every_event() {
            assert!(reporter.build_request(&event).is_some(), "{event:?}");
        }
        // Repeatable events stay repeatable; once-per-version events do not.
        assert!(
            reporter
                .build_request(&Event::EngineFailed {
                    reason: EngineFailure::Timeout
                })
                .is_some()
        );
        assert!(
            reporter
                .build_request(&Event::FirstTurn {
                    provider: "anthropic".to_owned()
                })
                .is_none()
        );
        assert!(
            reporter
                .build_request(&Event::SeamFirstUse {
                    seam: Seam::CommitMessage
                })
                .is_none()
        );
        // A different seam is a different mark.
        assert!(
            reporter
                .build_request(&Event::SeamFirstUse {
                    seam: Seam::AgentPanel
                })
                .is_some()
        );
    }

    #[test]
    fn a_once_per_version_mark_is_per_version() {
        let (reporter, store) = reporter(Some(true));
        let turn = Event::FirstTurn {
            provider: "anthropic".to_owned(),
        };
        assert!(reporter.build_request(&turn).is_some());
        assert!(reporter.build_request(&turn).is_none());

        // The same store, a newer IDE: the launch funnel starts again.
        let executor = gpui::BackgroundExecutor::new(Arc::new(gpui::TestDispatcher::new(0)));
        let next = Reporter::new(
            http_client::FakeHttpClient::with_200_response(),
            executor,
            "1.1.0+stable",
            store,
        );
        assert!(next.build_request(&turn).is_some());
    }

    /// The test that matters. Every `StartError` message carries a path, and on
    /// Windows a path carries the username.
    #[test]
    fn no_start_error_leaks_a_path_or_a_username() {
        let path =
            PathBuf::from("C:/Users/ada.lovelace/AppData/Local/KnightCode/knightcode-engine.exe");
        let errors = [
            StartError::Spawn {
                path: path.clone(),
                source: anyhow::anyhow!("The system cannot find the file {}", path.display()),
            },
            StartError::ExitedBeforeReady {
                status: exit_status(),
                stderr: format!("failed to read {}", path.display()),
            },
            StartError::Stalled {
                timeout: Duration::from_secs(20),
                stderr: format!("stuck opening {}", path.display()),
            },
            StartError::TokenRejected { status: 401 },
            StartError::Io(anyhow::anyhow!("no route to {}", path.display())),
        ];

        let (reporter, _) = reporter(Some(true));
        for error in &errors {
            // Every message really does carry the username, so the assertion below
            // is testing the mapping rather than a vacuous truth.
            assert!(
                error.to_string().contains("ada.lovelace")
                    || matches!(error, StartError::TokenRejected { .. }),
                "{error}"
            );
            let event = Event::EngineFailed {
                reason: EngineFailure::from(error),
            };
            let request = reporter.build_request(&event).unwrap();
            let wire = format!("{} {:?}", request.uri(), request.headers());
            assert!(!wire.contains("ada.lovelace"), "{wire}");
            assert!(!wire.contains("Users"), "{wire}");
            assert!(!wire.contains(".exe"), "{wire}");
        }
    }

    #[cfg(windows)]
    fn exit_status() -> std::process::ExitStatus {
        use std::os::windows::process::ExitStatusExt as _;
        std::process::ExitStatus::from_raw(3)
    }

    #[cfg(not(windows))]
    fn exit_status() -> std::process::ExitStatus {
        use std::os::unix::process::ExitStatusExt as _;
        std::process::ExitStatus::from_raw(3)
    }

    /// Pins the whole wire form of every variant, so adding a property is a
    /// test change someone has to look at.
    #[test]
    fn every_payload_matches_its_fixture() {
        let expected = [
            "https://knightcode.dev/api/report-install?version=1.0.0%2Bstable&event=ide_first_run&outcome=completed&step=open",
            "https://knightcode.dev/api/report-install?version=1.0.0%2Bstable&event=ide_engine_failed&reason=binary_missing",
            "https://knightcode.dev/api/report-install?version=1.0.0%2Bstable&event=ide_first_turn&provider=anthropic",
            "https://knightcode.dev/api/report-install?version=1.0.0%2Bstable&event=ide_seam_first_use&seam=commit_message",
        ];
        let (reporter, _) = reporter(Some(true));
        for (event, expected) in every_event().into_iter().zip(expected) {
            let request = reporter.build_request(&event).unwrap();
            assert_eq!(request.uri().to_string(), expected);
            assert_eq!(
                request
                    .headers()
                    .get("User-Agent")
                    .unwrap()
                    .to_str()
                    .unwrap(),
                format!(
                    "knightcode-ide/1.0.0+stable ({}; rust; {})",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                )
            );
        }
    }

    #[test]
    fn a_provider_id_cannot_smuggle_anything_through() {
        let (reporter, _) = reporter(Some(true));
        let request = reporter
            .build_request(&Event::FirstTurn {
                provider: "C:/Users/ada/Anthropic?x=1&y=2".to_owned(),
            })
            .unwrap();
        assert_eq!(
            request.uri().to_string(),
            "https://knightcode.dev/api/report-install?version=1.0.0%2Bstable&event=ide_first_turn&provider=cusersadaanthropicx1y2"
        );
    }
}
