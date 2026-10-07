use anyhow::{Result, anyhow, bail};
use gpui::{Context, Entity, Render, Subscription, Task, Window};
use knightcode_engine::{
    EngineEvent,
    client::EngineClient,
    providers::{
        DiscoveredLocalModel, LocalModel, LocalModelCost, LocalProviderConfig, LocalProviderKind,
        LocalProviderPreset, ProviderSummary, Providers,
    },
};
use std::collections::BTreeMap;
use ui::{Button, ButtonStyle, Color, Label, LabelSize, h_flex, prelude::*, v_flex};
use ui_input::InputField;
use util::ResultExt as _;

use crate::State;

struct ModelDraft {
    id: String,
    name: Option<String>,
    selected: bool,
    context_window: Entity<InputField>,
    max_tokens: Entity<InputField>,
    reasoning: Option<bool>,
    input: Option<Vec<String>>,
    cost: Entity<InputField>,
}

impl ModelDraft {
    fn new(
        model: DiscoveredLocalModel,
        selected: bool,
        window: &mut Window,
        cx: &mut Context<LocalModelsView>,
    ) -> Self {
        let context_window =
            cx.new(|cx| InputField::new(window, cx, "Context tokens").label("Context window"));
        let max_tokens = cx
            .new(|cx| InputField::new(window, cx, "Output tokens").label("Maximum output tokens"));
        let cost = cx.new(|cx| {
            InputField::new(
                window,
                cx,
                r#"{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}"#,
            )
            .label("Cost per million tokens")
        });
        if let Some(value) = model.context_window {
            context_window.update(cx, |input, cx| {
                input.set_text(&value.to_string(), window, cx)
            });
        }
        if let Some(value) = model.max_tokens {
            max_tokens.update(cx, |input, cx| {
                input.set_text(&value.to_string(), window, cx)
            });
        }
        if let Some(value) = model.cost {
            match serde_json::to_string(&value) {
                Ok(value) => cost.update(cx, |input, cx| input.set_text(&value, window, cx)),
                Err(error) => log::error!("Could not display local model costs: {error}"),
            }
        }
        Self {
            id: model.id,
            name: model.name,
            selected,
            context_window,
            max_tokens,
            reasoning: model.reasoning,
            input: model.input,
            cost,
        }
    }

    fn model(&self, cx: &Context<LocalModelsView>) -> Result<LocalModel> {
        let context_window = self
            .context_window
            .read(cx)
            .text(cx)
            .trim()
            .parse::<u64>()
            .map_err(|_| anyhow!("{}: enter a positive context window", self.id))?;
        let max_tokens = self
            .max_tokens
            .read(cx)
            .text(cx)
            .trim()
            .parse::<u64>()
            .map_err(|_| anyhow!("{}: enter a positive output limit", self.id))?;
        if context_window == 0 || max_tokens == 0 {
            bail!("{}: token limits must be positive", self.id);
        }
        let reasoning = self
            .reasoning
            .ok_or_else(|| anyhow!("{}: specify reasoning support", self.id))?;
        let input = self
            .input
            .clone()
            .ok_or_else(|| anyhow!("{}: specify supported inputs", self.id))?;
        let cost: LocalModelCost = serde_json::from_str(&self.cost.read(cx).text(cx))
            .map_err(|_| anyhow!("{}: enter input, output, cacheRead and cacheWrite costs, or select zero token cost", self.id))?;
        if [cost.input, cost.output, cost.cache_read, cost.cache_write]
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            bail!("{}: token costs must be finite and nonnegative", self.id);
        }
        Ok(LocalModel {
            id: self.id.clone(),
            name: self.name.clone(),
            context_window,
            max_tokens,
            reasoning,
            input,
            cost,
        })
    }
}

pub struct LocalModelsView {
    state: Entity<State>,
    providers: Providers,
    kind: Option<LocalProviderKind>,
    editing_id: Option<String>,
    provider_id: Entity<InputField>,
    name: Entity<InputField>,
    base_url: Entity<InputField>,
    api_key: Entity<InputField>,
    manual_model: Entity<InputField>,
    clear_key: bool,
    models: Vec<ModelDraft>,
    loading_providers: bool,
    operation: Option<&'static str>,
    error: Option<String>,
    status: Option<String>,
    load_task: Option<Task<()>>,
    operation_task: Option<Task<()>>,
    _subscription: Subscription,
}

impl LocalModelsView {
    pub fn new(state: Entity<State>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = state.read(cx).engine();
        let subscription = cx.subscribe(&engine, |this, _, event, cx| match event {
            EngineEvent::Ready(_) | EngineEvent::ModelsChanged => this.load(cx),
            EngineEvent::Stopped | EngineEvent::Failed(_) => {
                this.providers = Providers::default();
                this.load_task = None;
                this.operation_task = None;
                this.loading_providers = false;
                this.set_operation(None, cx);
                this.error = Some("The engine is not running".into());
                cx.notify();
            }
            _ => {}
        });
        let mut this = Self {
            state,
            providers: Providers::default(),
            kind: None,
            editing_id: None,
            provider_id: cx
                .new(|cx| InputField::new(window, cx, "Provider id").label("Provider id")),
            name: cx
                .new(|cx| InputField::new(window, cx, "Connection name").label("Connection name")),
            base_url: cx
                .new(|cx| InputField::new(window, cx, "HTTP API base URL").label("API base URL")),
            api_key: cx.new(|cx| {
                InputField::new(window, cx, "Optional API key")
                    .label("API key (write-only)")
                    .masked(true)
            }),
            manual_model: cx.new(|cx| {
                InputField::new(window, cx, "Exact model id").label("Add a model manually")
            }),
            clear_key: false,
            models: Vec::new(),
            loading_providers: false,
            operation: None,
            error: None,
            status: None,
            load_task: None,
            operation_task: None,
            _subscription: subscription,
        };
        this.load(cx);
        this
    }

    fn client(&self, cx: &Context<Self>) -> Result<EngineClient> {
        self.state
            .read(cx)
            .engine()
            .read(cx)
            .client()
            .ok_or_else(|| anyhow!("The engine is not running"))
    }

    fn set_operation(&mut self, operation: Option<&'static str>, cx: &mut Context<Self>) {
        self.operation = operation;
        for field in [
            &self.provider_id,
            &self.name,
            &self.base_url,
            &self.api_key,
            &self.manual_model,
        ] {
            field.update(cx, |input, cx| {
                input.editor().set_read_only(operation.is_some(), cx)
            });
        }
        for model in &self.models {
            for field in [&model.context_window, &model.max_tokens, &model.cost] {
                field.update(cx, |input, cx| {
                    input.editor().set_read_only(operation.is_some(), cx)
                });
            }
        }
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let client = match self.client(cx) {
            Ok(client) => client,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.loading_providers = true;
        self.load_task = Some(cx.spawn(async move |this, cx| {
            let outcome = client.providers().await;
            this.update(cx, |this, cx| {
                this.loading_providers = false;
                match outcome {
                    Ok(providers) => {
                        this.providers = providers;
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn choose_preset(
        &mut self,
        preset: &LocalProviderPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.kind = Some(preset.kind);
        self.editing_id = None;
        self.provider_id
            .update(cx, |input, cx| input.set_text(&preset.id, window, cx));
        self.name
            .update(cx, |input, cx| input.set_text(&preset.name, window, cx));
        self.base_url
            .update(cx, |input, cx| input.set_text(&preset.base_url, window, cx));
        self.api_key.update(cx, |input, cx| input.clear(window, cx));
        self.clear_key = false;
        self.models.clear();
        self.error = None;
        self.status = None;
        cx.notify();
    }

    fn edit(&mut self, provider: &ProviderSummary, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = &provider.local else {
            return;
        };
        self.kind = Some(config.kind);
        self.editing_id = Some(provider.id.clone());
        self.provider_id
            .update(cx, |input, cx| input.set_text(&provider.id, window, cx));
        self.name
            .update(cx, |input, cx| input.set_text(&config.name, window, cx));
        self.base_url
            .update(cx, |input, cx| input.set_text(&config.base_url, window, cx));
        self.api_key.update(cx, |input, cx| input.clear(window, cx));
        self.clear_key = false;
        self.models = config
            .models
            .iter()
            .map(|model| {
                ModelDraft::new(
                    DiscoveredLocalModel {
                        id: model.id.clone(),
                        name: model.name.clone(),
                        context_window: Some(model.context_window),
                        max_tokens: Some(model.max_tokens),
                        reasoning: Some(model.reasoning),
                        input: Some(model.input.clone()),
                        cost: Some(model.cost.clone()),
                    },
                    true,
                    window,
                    cx,
                )
            })
            .collect();
        self.error = None;
        self.status = None;
        cx.notify();
    }

    fn discover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.client(cx).and_then(|client| {
            Ok((
                client,
                self.kind.ok_or_else(|| anyhow!("Choose a server preset"))?,
            ))
        });
        let (client, kind) = match result {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let base_url = self.base_url.read(cx).text(cx).trim().to_owned();
        let api_key = self.api_key.read(cx).text(cx);
        let api_key = if self.clear_key {
            Some(String::new())
        } else {
            (!api_key.is_empty()).then_some(api_key)
        };
        let provider_id = self.editing_id.clone().filter(|id| {
            self.providers.providers.iter().any(|provider| {
                provider.id == *id
                    && provider.local.as_ref().is_some_and(|config| {
                        config.base_url.trim_end_matches('/') == base_url.trim_end_matches('/')
                    })
            })
        });
        self.set_operation(Some("Discovering models…"), cx);
        self.error = None;
        self.status = None;
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let outcome = client
                .discover_local_models(kind, &base_url, provider_id.as_deref(), api_key.as_deref())
                .await;
            this.update_in(cx, |this, window, cx| {
                this.set_operation(None, cx);
                match outcome {
                    Ok(models) => {
                        this.status = Some(format!(
                            "Found {} models. Complete unreported metadata before saving.",
                            models.len()
                        ));
                        let mut previous: BTreeMap<_, _> = this
                            .models
                            .drain(..)
                            .map(|model| (model.id.clone(), model))
                            .collect();
                        this.models = models
                            .into_iter()
                            .map(|model| {
                                previous
                                    .remove(&model.id)
                                    .unwrap_or_else(|| ModelDraft::new(model, false, window, cx))
                            })
                            .collect();
                        this.models
                            .extend(previous.into_values().filter(|model| model.selected));
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.client(cx).and_then(|client| {
            let models = self.models.iter().filter(|model| model.selected).map(|model| model.model(cx)).collect::<Result<Vec<_>>>()?;
            if models.is_empty() { bail!("Select at least one actual model before saving"); }
            let provider_id = self.provider_id.read(cx).text(cx).trim().to_owned();
            if self.editing_id.as_ref().is_some_and(|id| id != &provider_id) { bail!("The id of a saved provider cannot be renamed; choose a preset to add another connection"); }
            let config = LocalProviderConfig {
                kind: self.kind.ok_or_else(|| anyhow!("Choose a server preset"))?,
                name: self.name.read(cx).text(cx).trim().to_owned(),
                base_url: self.base_url.read(cx).text(cx).trim().to_owned(), models,
            };
            Ok((client, provider_id, config))
        });
        let (client, provider_id, config) = match result {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let api_key = self.api_key.read(cx).text(cx);
        let api_key = if self.clear_key {
            Some(String::new())
        } else {
            (!api_key.is_empty()).then_some(api_key)
        };
        self.api_key.update(cx, |input, cx| input.clear(window, cx));
        self.set_operation(Some("Saving provider…"), cx);
        self.error = None;
        self.operation_task = Some(cx.spawn(async move |this, cx| {
            let outcome = client
                .save_local_provider(&provider_id, &config, api_key.as_deref())
                .await;
            this.update(cx, |this, cx| {
                this.set_operation(None, cx);
                match outcome {
                    Ok(()) => {
                        this.editing_id = Some(provider_id);
                        this.clear_key = false;
                        this.status =
                            Some("Provider saved. Models are refreshed from the engine.".into());
                        this.load(cx);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn remove(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let client = match self.client(cx) {
            Ok(client) => client,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        self.set_operation(Some("Removing provider…"), cx);
        self.error = None;
        self.operation_task = Some(cx.spawn(async move |this, cx| {
            let outcome = client.remove_local_provider(&provider_id).await;
            this.update(cx, |this, cx| {
                this.set_operation(None, cx);
                match outcome {
                    Ok(()) => {
                        if this.editing_id.as_deref() == Some(provider_id.as_str()) {
                            this.editing_id = None;
                            this.kind = None;
                            this.models.clear();
                        }
                        this.status = Some("Provider removed".into());
                        this.load(cx);
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn add_manual_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.manual_model.read(cx).text(cx).trim().to_owned();
        if id.is_empty() || self.models.iter().any(|model| model.id == id) {
            self.error = Some("Enter a nonempty, unique model id".into());
        } else {
            self.models.push(ModelDraft::new(
                DiscoveredLocalModel {
                    id,
                    name: None,
                    context_window: None,
                    max_tokens: None,
                    reasoning: None,
                    input: None,
                    cost: None,
                },
                true,
                window,
                cx,
            ));
            self.manual_model
                .update(cx, |input, cx| input.clear(window, cx));
            self.error = None;
        }
        cx.notify();
    }
}

impl Render for LocalModelsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.loading_providers || self.operation.is_some();
        let mut root = v_flex().gap_2()
            .child(Label::new("Local LLM"))
            .child(Label::new("Discover actual models from a server. Saved connections and credentials are owned by the engine and shared with the CLI.").size(LabelSize::Small).color(Color::Muted));
        if self.loading_providers {
            root = root.child(Label::new("Loading local providers…").color(Color::Muted));
        }
        if let Some(operation) = self.operation {
            root = root.child(Label::new(operation).color(Color::Muted));
        }
        if let Some(error) = &self.error {
            root = root.child(Label::new(error.clone()).color(Color::Error));
        }
        if let Some(status) = &self.status {
            root = root.child(Label::new(status.clone()).size(LabelSize::Small));
        }
        for provider in self
            .providers
            .providers
            .iter()
            .filter(|provider| provider.local.is_some())
        {
            let edit = provider.clone();
            let provider_id = provider.id.clone();
            root = root.child(
                h_flex()
                    .gap_2()
                    .justify_between()
                    .child(Label::new(format!("{} ({})", provider.name, provider.id)))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new(format!("edit-local-{}", provider.id), "Edit")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.edit(&edit, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(format!("remove-local-{}", provider.id), "Remove")
                                    .style(ButtonStyle::Outlined)
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove(provider_id.clone(), cx)
                                    })),
                            ),
                    ),
            );
        }
        let mut presets = h_flex().flex_wrap().gap_1();
        for preset in &self.providers.presets {
            let preset = preset.clone();
            presets = presets.child(
                Button::new(format!("preset-{}", preset.id), preset.name.clone())
                    .style(ButtonStyle::Outlined)
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.choose_preset(&preset, window, cx)
                    })),
            );
        }
        root = root.child(presets).child(
            Button::new("reload-local-providers", "Refresh connections")
                .style(ButtonStyle::Outlined)
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
        );
        if self.kind.is_none() {
            return root;
        }
        root = root.child(self.provider_id.clone()).child(self.name.clone()).child(self.base_url.clone())
            .when(!self.clear_key, |this| this.child(self.api_key.clone()))
            .child(Button::new("clear-local-key", if self.clear_key { "Use keyless access on save" } else { "Clear saved key on save" })
                .toggle_state(self.clear_key).disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.clear_key = !this.clear_key;
                    if this.clear_key { this.api_key.update(cx, |input, cx| input.clear(window, cx)); }
                    cx.notify();
                })))
            .child(Label::new("An empty key field keeps the saved key. Clearing it is an explicit choice above.").size(LabelSize::Small).color(Color::Muted))
            .child(Button::new("discover-local-models", "Discover / refresh models").style(ButtonStyle::Outlined).disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.discover(window, cx))))
            .child(self.manual_model.clone())
            .child(Button::new("add-local-model", "Add model id").disabled(busy)
                .on_click(cx.listener(|this, _, window, cx| this.add_manual_model(window, cx))));
        for (index, model) in self.models.iter().enumerate() {
            let mut row = v_flex().gap_1().child(
                Button::new(
                    format!("select-local-model-{index}"),
                    model.name.clone().unwrap_or_else(|| model.id.clone()),
                )
                .toggle_state(model.selected)
                .disabled(busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(model) = this.models.get_mut(index) {
                        model.selected = !model.selected;
                    }
                    cx.notify();
                })),
            );
            if model.selected {
                row = row
                    .child(
                        Label::new(model.id.clone())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(model.context_window.clone())
                            .child(model.max_tokens.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new(
                                    format!("local-reasoning-{index}"),
                                    "Supports reasoning",
                                )
                                .toggle_state(model.reasoning == Some(true))
                                .disabled(busy)
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if let Some(model) = this.models.get_mut(index) {
                                            model.reasoning = Some(true);
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                Button::new(format!("local-no-reasoning-{index}"), "No reasoning")
                                    .toggle_state(model.reasoning == Some(false))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(model) = this.models.get_mut(index) {
                                            model.reasoning = Some(false);
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new(format!("local-text-{index}"), "Text only")
                                    .toggle_state(model.input.as_deref().is_some_and(
                                        |input| matches!(input, [text] if text == "text"),
                                    ))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(model) = this.models.get_mut(index) {
                                            model.input = Some(vec!["text".into()]);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(format!("local-image-{index}"), "Text and images")
                                    .toggle_state(model.input.as_ref().is_some_and(|input| {
                                        input.iter().any(|value| value == "image")
                                    }))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(model) = this.models.get_mut(index) {
                                            model.input = Some(vec!["text".into(), "image".into()]);
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(model.cost.clone())
                    .child(
                        Button::new(
                            format!("local-zero-cost-{index}"),
                            "Local / zero token cost",
                        )
                        .disabled(busy)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                if let Some(model) = this.models.get(index) {
                                    model.cost.update(cx, |input, cx| {
                                        input.set_text(
                                        r#"{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}"#,
                                        window,
                                        cx,
                                    )
                                    });
                                }
                                cx.notify();
                            },
                        )),
                    );
            }
            root = root.child(row);
        }
        root.child(
            Button::new("save-local-provider", "Save selected models")
                .style(ButtonStyle::Filled)
                .disabled(busy || !self.models.iter().any(|model| model.selected))
                .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
        )
    }
}
