//! The one install-telemetry switch, used by first run's welcome step and by
//! the KnightCode section of Settings > AI.
//!
//! It reads and writes the engine's `enableInstallTelemetry`, which is the same
//! answer the CLI honours, so the two front doors cannot disagree. It also
//! mirrors the answer into the IDE's key-value store, because
//! `ide_engine_failed` has to be reportable precisely when the engine is down
//! and its settings are unreachable.

use gpui::{Context, Entity, Render, Subscription, Task};
use knightcode_engine::{Engine, Reporter};
use ui::{
    Color, Label, LabelSize, ParentElement as _, Styled as _, SwitchField, ToggleState, prelude::*,
    v_flex,
};

/// Exactly what the ping contains, in the words a stranger can check. Vague
/// wording here would be worse than not asking.
pub const TELEMETRY_DESCRIPTION: &str = "Sends the version, your operating system and architecture, \
and an approximate location derived from the connecting address. No account, no file, no project \
name, and no identifier that follows you between versions.";

pub struct TelemetrySwitch {
    engine: Entity<Engine>,
    enabled: bool,
    /// `KNIGHTCODE_TELEMETRY` is set, so the answer is not the user's to move
    /// from here and the switch says so instead of lying.
    overridden: bool,
    _task: Task<()>,
    _subscription: Subscription,
}

impl TelemetrySwitch {
    pub fn new(engine: Entity<Engine>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&engine, |this, _, cx| this.load(cx));
        let mut this = Self {
            engine,
            // Matches the CLI's default, so a machine that never opens first run
            // behaves the same on both front doors.
            enabled: true,
            overridden: false,
            _task: Task::ready(()),
            _subscription: subscription,
        };
        this.load(cx);
        this
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.engine.read(cx).client() else {
            return;
        };
        self._task = cx.spawn(async move |this, cx| {
            let Ok(setting) = client.telemetry_setting().await else {
                return;
            };
            this.update(cx, |this, cx| {
                this.enabled = setting.enabled;
                this.overridden = setting.overridden;
                // The mirror the reporter reads when the engine is down.
                if let Some(reporter) = Reporter::try_global(cx) {
                    reporter.set_consent(setting.enabled);
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn set(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.overridden {
            return;
        }
        let Some(client) = self.engine.read(cx).client() else {
            return;
        };
        self.enabled = enabled;
        if let Some(reporter) = Reporter::try_global(cx) {
            reporter.set_consent(enabled);
        }
        cx.notify();
        self._task = cx.spawn(async move |this, cx| {
            if client.set_telemetry_setting(enabled).await.is_err() {
                // The engine is the record; if it refused, re-read rather than
                // leave the switch showing something nobody stored.
                this.update(cx, |this, cx| this.load(cx)).ok();
            }
        });
    }
}

impl Render for TelemetrySwitch {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.enabled;
        let mut root = v_flex().gap_1().child(
            SwitchField::new(
                "knightcode-install-telemetry",
                Some("Send an anonymous ping when KnightCode is installed or updated"),
                Some(TELEMETRY_DESCRIPTION.into()),
                if enabled {
                    ToggleState::Selected
                } else {
                    ToggleState::Unselected
                },
                cx.listener(|this, state: &ToggleState, _, cx| {
                    this.set(*state == ToggleState::Selected, cx);
                }),
            )
            .disabled(self.overridden),
        );
        if self.overridden {
            root = root.child(
                Label::new("KNIGHTCODE_TELEMETRY is set in this environment and takes precedence.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
        }
        root
    }
}
