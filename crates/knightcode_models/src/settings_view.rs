//! What Settings > AI shows under KnightCode: the sign-in, the model picker and
//! the install-telemetry switch, in that order. Every part is a component first
//! run uses too, so the two never drift apart.

use gpui::{Context, Entity, Render, Window};
use ui::{Divider, ParentElement as _, Styled as _, prelude::*, v_flex};

use crate::local_models::LocalModelsView;
use crate::model_picker::ModelPicker;
use crate::sign_in::SignInView;
use crate::state::State;
use crate::telemetry_switch::TelemetrySwitch;

pub struct KnightCodeSettingsView {
    sign_in: Entity<SignInView>,
    local_models: Entity<LocalModelsView>,
    models: Entity<ModelPicker>,
    telemetry: Entity<TelemetrySwitch>,
}

impl KnightCodeSettingsView {
    pub fn new(state: Entity<State>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = state.read(cx).engine();
        Self {
            sign_in: cx.new(|cx| SignInView::new(state.clone(), window, cx)),
            local_models: cx.new(|cx| LocalModelsView::new(state.clone(), window, cx)),
            models: cx.new(|cx| ModelPicker::new(state, cx)),
            telemetry: cx.new(|cx| TelemetrySwitch::new(engine, cx)),
        }
    }
}

impl Render for KnightCodeSettingsView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_3()
            .child(self.sign_in.clone())
            .child(Divider::horizontal())
            .child(self.local_models.clone())
            .child(Divider::horizontal())
            .child(self.models.clone())
            .child(Divider::horizontal())
            .child(self.telemetry.clone())
    }
}
