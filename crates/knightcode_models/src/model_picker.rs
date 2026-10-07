//! The one model picker, used by first run's third step and by the KnightCode
//! section of Settings > AI.
//!
//! It shows what the engine reports and nothing else: no per-provider table, no
//! recommended model, no fallback when the catalog is empty. Choosing writes
//! through the engine, which publishes `models.changed`, which refreshes
//! [`State`] — so the tick moves because the engine stored the choice, not
//! because this view assumed it would.

use gpui::{Context, Entity, Render, Subscription};
use ui::{
    Button, ButtonStyle, Color, Icon, IconName, IconSize, Label, LabelSize, ParentElement as _,
    SharedString, Styled as _, h_flex, prelude::*, v_flex,
};

use crate::state::State;

pub struct ModelPicker {
    state: Entity<State>,
    /// The last write's error, shown rather than swallowed: a rejected choice
    /// that silently does nothing looks like a broken button.
    error: Option<String>,
    _subscription: Subscription,
}

impl ModelPicker {
    pub fn new(state: Entity<State>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            error: None,
            _subscription: subscription,
        }
    }

    fn choose(&mut self, reference: String, cx: &mut Context<Self>) {
        self.error = None;
        let write = self
            .state
            .update(cx, |state, cx| state.set_default_model(reference, cx));
        cx.spawn(async move |this, cx| {
            if let Err(error) = write.await {
                this.update(cx, |this, cx| {
                    this.error = Some(error.to_string());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }
}

impl Render for ModelPicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let chosen = state.default_model().map(|model| model.reference.clone());

        if state.models.is_empty() {
            return v_flex().gap_1().child(
                Label::new("No models yet. Sign in above, and the ones your account can reach appear here.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            );
        }

        // Grouped by provider, in the engine's own order, the way the agent
        // panel's session picker groups them, so the two agree.
        let mut root = v_flex().gap_2();
        let mut providers: Vec<&str> = Vec::new();
        for model in &state.models {
            if !providers.contains(&model.provider_id.as_str()) {
                providers.push(&model.provider_id);
            }
        }

        for provider in providers {
            let name = state
                .models
                .iter()
                .find(|model| model.provider_id == provider)
                .map(|model| model.provider_name.clone())
                .unwrap_or_else(|| provider.to_owned());
            let mut group = v_flex()
                .gap_1()
                .child(Label::new(name).size(LabelSize::Small).color(Color::Muted));
            for model in state.models.iter().filter(|m| m.provider_id == provider) {
                let reference = model.reference.clone();
                let is_chosen = chosen.as_deref() == Some(reference.as_str());
                group = group.child(
                    Button::new(
                        SharedString::from(format!("model-{reference}")),
                        model.name.clone(),
                    )
                    .style(if is_chosen {
                        ButtonStyle::Filled
                    } else {
                        ButtonStyle::Outlined
                    })
                    .when(is_chosen, |button| {
                        button.start_icon(
                            Icon::new(IconName::Check)
                                .size(IconSize::Small)
                                .color(Color::Accent),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.choose(reference.clone(), cx);
                    })),
                );
            }
            root = root.child(group);
        }

        if let Some(error) = &self.error {
            root = root.child(
                h_flex().child(
                    Label::new(format!("Could not record that choice: {error}"))
                        .size(LabelSize::Small)
                        .color(Color::Error),
                ),
            );
        }
        root
    }
}
