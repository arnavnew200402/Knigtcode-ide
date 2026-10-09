use acp_thread::AcpThread;
use agent_client_protocol::schema::v1 as acp;
use anyhow::{Context as _, Result};
use collections::BTreeMap;
use editor::Editor;
use gpui::{AppContext as _, Entity, EventEmitter, Focusable as _, Subscription, Task};
use knightcode_engine::{
    Engine, EngineClient, EngineEvent,
    client::{ClientError, EngineModel},
    tasks::{AgentProfile, SubagentConfig, TaskInput, TaskSnapshot},
};
use ui::{ContextMenu, Divider, PopoverMenu, prelude::*};
use util::ResultExt as _;

pub enum TaskPanelEvent {
    OpenChild(TaskSnapshot),
}

#[derive(Clone, Default)]
enum TaskSelection {
    #[default]
    Default,
    Parent,
    Value(String),
}

impl TaskSelection {
    fn label(&self, default: &str, parent: &str) -> String {
        match self {
            Self::Default => default.to_owned(),
            Self::Parent => parent.to_owned(),
            Self::Value(value) => value.clone(),
        }
    }

    fn resolve(&self, parent: &AcpThread, setting: &str, cx: &App) -> Result<Option<String>> {
        match self {
            Self::Default => Ok(None),
            Self::Value(value) => Ok(Some(value.clone())),
            Self::Parent => parent
                .connection()
                .session_config_options(parent.session_id(), cx)
                .and_then(|options| {
                    options.config_options().into_iter().find_map(|option| {
                        if option.id.0.as_ref() == setting
                            && let acp::SessionConfigKind::Select(select) = option.kind
                        {
                            Some(select.current_value.0.to_string())
                        } else {
                            None
                        }
                    })
                })
                .map(Some)
                .with_context(|| format!("The parent's {setting} setting is unavailable")),
        }
    }
}

pub struct KnightCodeTasks {
    engine: Entity<Engine>,
    parent: Entity<AcpThread>,
    session_id: String,
    config: Option<SubagentConfig>,
    tasks: BTreeMap<String, TaskSnapshot>,
    models: Vec<EngineModel>,
    expanded: bool,
    show_settings: bool,
    prompt: Entity<Editor>,
    profiles: Entity<Editor>,
    profiles_dirty: bool,
    selected_agent: Option<String>,
    selected_model: TaskSelection,
    thinking_level: TaskSelection,
    resume_id: Option<String>,
    error: Option<String>,
    connection_error: Option<String>,
    connected: bool,
    available: bool,
    loading: bool,
    busy: bool,
    refresh_task: Option<Task<()>>,
    operation_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TaskPanelEvent> for KnightCodeTasks {}

impl KnightCodeTasks {
    pub fn new(
        engine: Entity<Engine>,
        parent: Entity<AcpThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let session_id = parent.read(cx).session_id().to_string();
        let prompt = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_placeholder_text("Describe a task or continuation", window, cx);
            editor.set_soft_wrap();
            editor.set_mode(editor::EditorMode::AutoHeight {
                min_lines: 2,
                max_lines: Some(6),
            });
            editor
        });
        let profiles = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_mode(editor::EditorMode::AutoHeight {
                min_lines: 3,
                max_lines: Some(10),
            });
            editor
        });
        let subscriptions = vec![
            cx.subscribe_in(&engine, window, |this, _, event, window, cx| match event {
                EngineEvent::Ready(_) | EngineEvent::EventsConnected => this.refresh(window, cx),
                EngineEvent::TaskChanged { task } if task.parent_session_id == this.session_id => {
                    this.accept_task(task.clone(), cx);
                }
                EngineEvent::ModelsChanged => this.refresh(window, cx),
                EngineEvent::Failed(message) => {
                    this.connected = false;
                    this.refresh_task = None;
                    this.operation_task = None;
                    this.busy = false;
                    this.loading = false;
                    this.connection_error = Some(message.to_string());
                    cx.notify();
                }
                EngineEvent::Stopped => {
                    this.connected = false;
                    this.refresh_task = None;
                    this.operation_task = None;
                    this.busy = false;
                    this.loading = false;
                    this.connection_error = Some(
                        "The engine stopped. Task state will refresh when it reconnects.".into(),
                    );
                    cx.notify();
                }
                _ => {}
            }),
            cx.subscribe(&profiles, |this, editor, event, cx| {
                if matches!(event, editor::EditorEvent::Edited { .. }) {
                    this.profiles_dirty = this
                        .config
                        .as_ref()
                        .and_then(|config| serde_json::to_string_pretty(&config.agents).ok())
                        .as_deref()
                        != Some(editor.read(cx).text(cx).as_str());
                    cx.notify();
                }
            }),
        ];
        let mut this = Self {
            engine,
            parent,
            session_id,
            config: None,
            tasks: BTreeMap::default(),
            models: Vec::new(),
            expanded: false,
            show_settings: false,
            prompt,
            profiles,
            profiles_dirty: false,
            selected_agent: None,
            selected_model: TaskSelection::Default,
            thinking_level: TaskSelection::Default,
            resume_id: None,
            error: None,
            connection_error: None,
            connected: false,
            available: true,
            loading: false,
            busy: false,
            refresh_task: None,
            operation_task: None,
            _subscriptions: subscriptions,
        };
        this.refresh(window, cx);
        this
    }

    fn client(&self, cx: &App) -> Result<EngineClient> {
        self.engine
            .read(cx)
            .client()
            .context("The engine is not ready")
    }

    pub fn active_count(&self) -> usize {
        self.tasks
            .values()
            .filter(|task| task.status.is_active())
            .count()
    }

    pub fn render_composer_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let enabled = self.config.as_ref().is_some_and(|config| config.enabled);
        h_flex()
            .gap_1()
            .child(
                Button::new(
                    "subagent-enabled",
                    if enabled {
                        "Subagents on"
                    } else {
                        "Subagents off"
                    },
                )
                .label_size(LabelSize::Small)
                .toggle_state(enabled)
                .disabled(self.config.is_none() || self.busy || !self.connected)
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(mut config) = this.config.clone() {
                        config.enabled = !config.enabled;
                        this.save_config(config, window, cx);
                    }
                })),
            )
            .child(
                Button::new("task-panel", format!("Tasks ({})", self.active_count()))
                    .label_size(LabelSize::Small)
                    .toggle_state(self.expanded)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded = !this.expanded;
                        cx.notify();
                    })),
            )
    }

    fn accept_task(&mut self, task: TaskSnapshot, cx: &mut Context<Self>) {
        if task.parent_session_id != self.session_id
            || self
                .tasks
                .get(&task.id)
                .is_some_and(|current| current.revision >= task.revision)
        {
            return;
        }
        if let Some(child) = self
            .parent
            .read(cx)
            .subagent(&acp::SessionId::new(task.session_id.clone()))
        {
            child.update(cx, |child, cx| {
                child.update_external_task(
                    task.id.clone(),
                    task.revision,
                    &task.status.label().to_ascii_lowercase(),
                    cx,
                )
            });
        }
        self.tasks.insert(task.id.clone(), task);
        cx.notify();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let client = match self.client(cx) {
            Ok(client) => client,
            Err(error) => {
                self.connected = false;
                self.connection_error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let session_id = self.session_id.clone();
        let revisions = self
            .tasks
            .iter()
            .map(|(id, task)| (id.clone(), task.revision))
            .collect::<BTreeMap<_, _>>();
        self.loading = true;
        self.refresh_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let config = match client.subagent_config(&session_id).await {
                    Ok(config) => config,
                    // Older bundled engines do not expose the optional task
                    // API. That must not surface as a conversation error.
                    Err(ClientError::Status { status: 404, .. }) => return anyhow::Ok(None),
                    Err(error) => return Err(error.into()),
                };
                let tasks = client.tasks(&session_id).await?;
                let models = client.models().await?;
                anyhow::Ok(Some((config, tasks, models.models)))
            }
            .await;
            this.update_in(cx, |this, window, cx| {
                this.loading = false;
                match result {
                    Ok(None) => {
                        this.available = false;
                        this.connected = false;
                        this.connection_error = None;
                    }
                    Ok(Some((config, tasks, models))) => {
                        this.available = true;
                        this.connected = true;
                        this.connection_error = None;
                        this.install_config(config, window, cx);
                        this.tasks.retain(|id, task| {
                            tasks.iter().any(|listed| listed.id == *id)
                                || revisions.get(id) != Some(&task.revision)
                        });
                        for task in tasks {
                            this.accept_task(task, cx);
                        }
                        this.models = models;
                    }
                    Err(error) => {
                        this.connected = false;
                        this.connection_error = Some(error.to_string());
                    }
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn install_config(
        &mut self,
        config: SubagentConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .config
            .as_ref()
            .is_some_and(|current| current.revision > config.revision)
        {
            return;
        }
        if !self.profiles_dirty {
            match serde_json::to_string_pretty(&config.agents) {
                Ok(text) => self
                    .profiles
                    .update(cx, |editor, cx| editor.set_text(text, window, cx)),
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        self.config = Some(config);
    }

    fn save_config(&mut self, config: SubagentConfig, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let client = match self.client(cx) {
            Ok(client) => client,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let session_id = self.session_id.clone();
        self.busy = true;
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = client.set_subagent_config(&session_id, &config).await;
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(config) => {
                        if serde_json::from_str::<Vec<AgentProfile>>(
                            &this.profiles.read(cx).text(cx),
                        )
                        .ok()
                        .as_ref()
                            == Some(&config.agents)
                        {
                            this.profiles_dirty = false;
                        }
                        this.install_config(config, window, cx);
                        this.error = None;
                    }
                    Err(error) => {
                        this.error = Some(error.to_string());
                        this.refresh(window, cx);
                    }
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn dispatch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let overrides = (|| {
            let parent = self.parent.read(cx);
            anyhow::Ok((
                self.selected_model.resolve(parent, "model", cx)?,
                self.thinking_level.resolve(parent, "thinking", cx)?,
            ))
        })();
        let (model, thinking_level) = match overrides {
            Ok(overrides) => overrides,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let input = TaskInput {
            agent: self.selected_agent.clone(),
            prompt: self.prompt.read(cx).text(cx),
            model,
            thinking_level,
        };
        if input.prompt.trim().is_empty() {
            self.error = Some("Enter a task prompt".into());
            cx.notify();
            return;
        }
        let client = match self.client(cx) {
            Ok(client) => client,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        let session_id = self.session_id.clone();
        let resume_id = self.resume_id.clone();
        self.busy = true;
        self.operation_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = if let Some(id) = resume_id {
                client.resume_task(&id, &input).await.map(|task| vec![task])
            } else {
                client
                    .spawn_tasks(&session_id, &[input.clone()], true)
                    .await
            };
            this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(tasks) => {
                        for task in tasks {
                            this.accept_task(task, cx);
                        }
                        if this.prompt.read(cx).text(cx) == input.prompt {
                            this.prompt
                                .update(cx, |editor, cx| editor.clear(window, cx));
                            this.resume_id = None;
                        }
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

    fn cancel_task(&mut self, id: String, cx: &mut Context<Self>) {
        let client = match self.client(cx) {
            Ok(client) => client,
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        };
        cx.spawn(async move |this, cx| {
            let result = client.cancel_task(&id).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(task) => this.accept_task(task, cx),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .log_err();
        })
        .detach();
    }

    fn render_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let concurrent = self
            .config
            .as_ref()
            .map(|config| config.max_concurrent)
            .unwrap_or(0);
        v_flex()
            .gap_2()
            .child(
                Label::new("Disabling subagents blocks new tasks and continuations. Accepted tasks finish.")
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(Label::new(format!("Concurrent tasks: {concurrent}")))
                    .child(
                        Button::new("less-concurrent", "−")
                            .disabled(self.busy || concurrent <= 1 || !self.connected)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(mut config) = this.config.clone() {
                                    config.max_concurrent = config.max_concurrent.saturating_sub(1).max(1);
                                    this.save_config(config, window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("more-concurrent", "+")
                            .disabled(self.busy || concurrent >= 16 || !self.connected)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(mut config) = this.config.clone() {
                                    config.max_concurrent = config.max_concurrent.saturating_add(1).min(16);
                                    this.save_config(config, window, cx);
                                }
                            })),
                    ),
            )
            .child(
                Label::new("Agent profiles (JSON: name, description, optional model, thinkingLevel, tools, systemPrompt)")
                    .size(LabelSize::Small),
            )
            .child(self.profiles.clone())
            .child(
                Button::new("save-profiles", "Save profiles")
                    .disabled(self.busy || !self.profiles_dirty || !self.connected)
                    .on_click(cx.listener(|this, _, window, cx| {
                        match serde_json::from_str::<Vec<AgentProfile>>(&this.profiles.read(cx).text(cx)) {
                            Ok(agents) => {
                                if let Some(mut config) = this.config.clone() {
                                    config.agents = agents;
                                    this.save_config(config, window, cx);
                                }
                            }
                            Err(error) => {
                                this.error = Some(error.to_string());
                                cx.notify();
                            }
                        }
                    })),
            )
    }

    fn render_task(&self, task: &TaskSnapshot, cx: &Context<Self>) -> impl IntoElement {
        let id = task.id.clone();
        let open_task = task.clone();
        let resume_task = task.clone();
        let can_resume =
            !task.status.is_active() && self.config.as_ref().is_some_and(|config| config.enabled);
        v_flex()
            .gap_1()
            .p_2()
            .border_1()
            .border_color(cx.theme().colors().border)
            .rounded_md()
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(Label::new(format!(
                        "{} · {}",
                        task.agent,
                        task.status.label()
                    )))
                    .child(
                        Button::new(format!("open-{}", task.id), "Open child")
                            .disabled(!self.connected)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(TaskPanelEvent::OpenChild(open_task.clone()))
                            })),
                    ),
            )
            .child(Label::new(task.prompt.clone()).size(LabelSize::Small))
            .child(
                Label::new(format!(
                    "{} · {} · {} tools · {} tokens · ${:.4}",
                    task.model,
                    task.thinking_level,
                    task.progress.tool_calls,
                    task.usage.total,
                    task.usage.cost
                ))
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
            .when_some(task.progress.last_tool.clone(), |this, tool| {
                this.child(
                    Label::new(format!("Last tool: {tool}"))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
            .when(!task.progress.text.is_empty(), |this| {
                this.child(Label::new(task.progress.text.clone()).size(LabelSize::Small))
            })
            .when_some(task.result.clone(), |this, result| {
                this.child(Label::new(result).size(LabelSize::Small))
            })
            .when(task.result_truncated, |this| {
                this.child(
                    Label::new("Result truncated; open the child for its full conversation.")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
            .when_some(task.error.clone(), |this, error| {
                this.child(Label::new(error).size(LabelSize::Small).color(Color::Error))
            })
            .child(
                h_flex()
                    .gap_2()
                    .when(task.status.is_active(), |this| {
                        this.child(
                            Button::new(format!("cancel-{}", task.id), "Cancel")
                                .disabled(
                                    !self.connected
                                        || task.status
                                            == knightcode_engine::tasks::TaskStatus::Cancelling,
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.cancel_task(id.clone(), cx)
                                })),
                        )
                    })
                    .when(!task.status.is_active(), |this| {
                        this.child(
                            Button::new(format!("resume-{}", task.id), "Continue")
                                .disabled(!can_resume || self.busy || !self.connected)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.resume_id = Some(resume_task.id.clone());
                                    this.selected_agent = Some(resume_task.agent.clone());
                                    this.selected_model =
                                        TaskSelection::Value(resume_task.model.clone());
                                    this.thinking_level =
                                        TaskSelection::Value(resume_task.thinking_level.clone());
                                    this.prompt.focus_handle(cx).focus(window, cx);
                                    cx.notify();
                                })),
                        )
                    }),
            )
    }
}

impl Render for KnightCodeTasks {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.available {
            return div().into_any_element();
        }
        let can_dispatch = self.connected
            && !self.busy
            && self.config.as_ref().is_some_and(|config| config.enabled);
        let agents = self
            .config
            .as_ref()
            .map(|config| config.agents.clone())
            .unwrap_or_default();
        let models = self.models.clone();
        let model_default = if self.resume_id.is_some() {
            "Keep task model"
        } else {
            "Agent default model"
        };
        let thinking_default = if self.resume_id.is_some() {
            "Keep task thinking"
        } else {
            "Agent default thinking"
        };
        let weak = cx.weak_entity();
        let agent_weak = weak.clone();
        let model_weak = weak.clone();
        let panel = v_flex()
            .id("knightcode-tasks")
            .gap_2()
            .p_2()
            .max_h(rems(28.))
            .overflow_y_scroll()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new(format!(
                        "Tasks · {} active",
                        self.active_count()
                    )))
                    .child(
                        Button::new("task-settings", "Settings")
                            .toggle_state(self.show_settings)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_settings = !this.show_settings;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("refresh-tasks", "Refresh")
                            .disabled(self.loading)
                            .on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(Label::new(error).color(Color::Error))
            })
            .when_some(self.connection_error.clone(), |this, error| {
                this.child(Label::new(error).color(Color::Error))
            })
            .when(self.loading, |this| {
                this.child(
                    Label::new("Refreshing task snapshots…")
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
            .when(self.show_settings, |this| {
                this.child(self.render_settings(cx))
                    .child(Divider::horizontal())
            })
            .child(
                h_flex()
                    .gap_1()
                    .flex_wrap()
                    .child(
                        PopoverMenu::new("task-agent-picker")
                            .trigger(
                                Button::new(
                                    "task-agent",
                                    self.selected_agent
                                        .clone()
                                        .unwrap_or_else(|| "Default agent".into()),
                                )
                                .disabled(self.resume_id.is_some()),
                            )
                            .menu(move |window, cx| {
                                Some(ContextMenu::build(window, cx, |mut menu, _, _| {
                                    let weak = agent_weak.clone();
                                    menu = menu.entry("Default agent", None, move |_, cx| {
                                        weak.update(cx, |this, cx| {
                                            this.selected_agent = None;
                                            cx.notify();
                                        })
                                        .log_err();
                                    });
                                    for agent in &agents {
                                        let name = agent.name.clone();
                                        let weak = agent_weak.clone();
                                        menu = menu.entry(name.clone(), None, move |_, cx| {
                                            weak.update(cx, |this, cx| {
                                                this.selected_agent = Some(name.clone());
                                                cx.notify();
                                            })
                                            .log_err();
                                        });
                                    }
                                    menu
                                }))
                            }),
                    )
                    .child(
                        PopoverMenu::new("task-model-picker")
                            .trigger(Button::new(
                                "task-model",
                                self.selected_model
                                    .label(model_default, "Inherit parent model"),
                            ))
                            .menu(move |window, cx| {
                                Some(ContextMenu::build(window, cx, |mut menu, _, _| {
                                    let weak = model_weak.clone();
                                    menu = menu.entry(model_default, None, move |_, cx| {
                                        weak.update(cx, |this, cx| {
                                            this.selected_model = TaskSelection::Default;
                                            cx.notify();
                                        })
                                        .log_err();
                                    });
                                    let weak = model_weak.clone();
                                    menu =
                                        menu.entry("Inherit parent model", None, move |_, cx| {
                                            weak.update(cx, |this, cx| {
                                                this.selected_model = TaskSelection::Parent;
                                                cx.notify();
                                            })
                                            .log_err();
                                        });
                                    for model in &models {
                                        let reference = model.reference.clone();
                                        let weak = model_weak.clone();
                                        menu = menu.entry(
                                            format!("{} ({})", model.name, model.provider_name),
                                            None,
                                            move |_, cx| {
                                                weak.update(cx, |this, cx| {
                                                    this.selected_model =
                                                        TaskSelection::Value(reference.clone());
                                                    cx.notify();
                                                })
                                                .log_err();
                                            },
                                        );
                                    }
                                    menu
                                }))
                            }),
                    )
                    .child(
                        PopoverMenu::new("task-thinking-picker")
                            .trigger(Button::new(
                                "task-thinking",
                                self.thinking_level
                                    .label(thinking_default, "Inherit parent thinking"),
                            ))
                            .menu(move |window, cx| {
                                Some(ContextMenu::build(window, cx, |mut menu, _, _| {
                                    let default_weak = weak.clone();
                                    menu = menu.entry(thinking_default, None, move |_, cx| {
                                        default_weak
                                            .update(cx, |this, cx| {
                                                this.thinking_level = TaskSelection::Default;
                                                cx.notify();
                                            })
                                            .log_err();
                                    });
                                    for level in [
                                        None,
                                        Some("off"),
                                        Some("minimal"),
                                        Some("low"),
                                        Some("medium"),
                                        Some("high"),
                                        Some("xhigh"),
                                        Some("max"),
                                    ] {
                                        let weak = weak.clone();
                                        menu = menu.entry(
                                            level.unwrap_or("Inherit parent thinking"),
                                            None,
                                            move |_, cx| {
                                                weak.update(cx, |this, cx| {
                                                    this.thinking_level = level
                                                        .map(|level| {
                                                            TaskSelection::Value(level.to_owned())
                                                        })
                                                        .unwrap_or(TaskSelection::Parent);
                                                    cx.notify();
                                                })
                                                .log_err();
                                            },
                                        );
                                    }
                                    menu
                                }))
                            }),
                    ),
            )
            .when_some(self.resume_id.clone(), |this, id| {
                this.child(
                    h_flex()
                        .gap_2()
                        .child(Label::new(format!("Continue task {id}")))
                        .child(
                            Button::new("new-task", "New task instead").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.resume_id = None;
                                    this.selected_model = TaskSelection::Default;
                                    this.thinking_level = TaskSelection::Default;
                                    cx.notify();
                                },
                            )),
                        ),
                )
            })
            .child(self.prompt.clone())
            .child(
                Button::new(
                    "dispatch-task",
                    if self.resume_id.is_some() {
                        "Resume task"
                    } else {
                        "Start task"
                    },
                )
                .disabled(!can_dispatch)
                .on_click(cx.listener(|this, _, window, cx| this.dispatch(window, cx))),
            )
            .children(
                self.tasks
                    .values()
                    .rev()
                    .map(|task| self.render_task(task, cx)),
            )
            .into_any_element();
        v_flex()
            .gap_2()
            .child(self.render_composer_controls(cx))
            .when(!self.expanded, |this| {
                this.when_some(self.error.clone(), |this, error| {
                    this.child(Label::new(error).size(LabelSize::Small).color(Color::Error))
                })
                .when_some(self.connection_error.clone(), |this, error| {
                    this.child(Label::new(error).size(LabelSize::Small).color(Color::Error))
                })
            })
            .when(self.expanded, |this| this.child(panel))
            .into_any_element()
    }
}
