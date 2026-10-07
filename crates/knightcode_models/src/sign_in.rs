use gpui::{Context, Entity, Render, Subscription, Task, Window};
use knightcode_engine::client::{LoginKind, PromptKind};
use ui::{
    Button, ButtonStyle, Color, Label, LabelSize, ParentElement as _, Styled as _, h_flex,
    prelude::*, v_flex,
};
use ui_input::InputField;
use util::ResultExt as _;

use crate::state::{LoginStep, State};

pub struct SignInView {
    state: Entity<State>,
    input: Entity<InputField>,
    prompt_id: Option<String>,
    loading: bool,
    error: Option<String>,
    task: Option<Task<()>>,
    _subscription: Subscription,
}

impl SignInView {
    pub fn new(state: Entity<State>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputField::new(window, cx, ""));
        let subscription = cx.observe(&state, |_, _, cx| cx.notify());
        let mut this = Self {
            state,
            input,
            prompt_id: None,
            loading: false,
            error: None,
            task: None,
            _subscription: subscription,
        };
        this.refresh(cx);
        this
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        let refresh = self.state.update(cx, |state, cx| state.refresh(cx));
        self.task = Some(cx.spawn(async move |this, cx| {
            let outcome = refresh.await;
            this.update(cx, |this, cx| {
                this.loading = false;
                this.error = match outcome {
                    Ok(()) | Err(language_model::AuthenticateError::CredentialsNotFound) => None,
                    Err(error) => Some(error.to_string()),
                };
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn sign_out(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let Some(client) = self.state.read(cx).engine().read(cx).client() else {
            self.error = Some("The engine is not running".into());
            cx.notify();
            return;
        };
        self.loading = true;
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let outcome = client.sign_out(&provider_id).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match outcome {
                    Ok(()) => this.refresh(cx),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn refresh_provider(&mut self, provider_id: String, cx: &mut Context<Self>) {
        let Some(client) = self.state.read(cx).engine().read(cx).client() else {
            self.error = Some("The engine is not running".into());
            cx.notify();
            return;
        };
        self.loading = true;
        self.error = None;
        self.task = Some(cx.spawn(async move |this, cx| {
            let outcome = client.refresh_provider_models(&provider_id).await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match outcome {
                    Ok(_) => this.refresh(cx),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }
}

impl Render for SignInView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.read(cx);
        let login_options = state.login_options.clone();
        let mut root = v_flex().gap_2();
        if self.loading {
            root = root.child(Label::new("Loading provider connections…").color(Color::Muted));
        }
        if let Some(error) = &self.error {
            root = root.child(Label::new(error.clone()).color(Color::Error));
        }

        for account in &state.accounts {
            let provider_id = account.provider_id.clone();
            let refresh_id = provider_id.clone();
            let kind = match account.kind {
                LoginKind::Oauth if account.is_subscription => "subscription",
                LoginKind::Oauth => "account",
                LoginKind::ApiKey => "API key",
            };
            root = root.child(
                h_flex()
                    .justify_between()
                    .child(Label::new(format!(
                        "Signed in to {} ({kind})",
                        account.provider_name
                    )))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new(
                                    format!("refresh-provider-{refresh_id}"),
                                    "Refresh models",
                                )
                                .style(ButtonStyle::Outlined)
                                .disabled(self.loading || state.login.is_some())
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.refresh_provider(refresh_id.clone(), cx);
                                    },
                                )),
                            )
                            .child(
                                Button::new(format!("sign-out-{provider_id}"), "Sign out")
                                    .style(ButtonStyle::Outlined)
                                    .disabled(self.loading || state.login.is_some())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.sign_out(provider_id.clone(), cx);
                                    })),
                            ),
                    ),
            );
        }

        if let Some(active) = &state.login {
            let title = format!("Signing in to {}", active.option.provider_name);
            root = root.child(Label::new(title).size(LabelSize::Small));
            root = match &active.step {
                LoginStep::Starting => root.child(Label::new("Contacting the engine…")),
                LoginStep::Browser(url) => root.child(Label::new(format!(
                    "Finish signing in in your browser. If it did not open: {url}"
                ))),
                LoginStep::DeviceCode { code, url } => {
                    root.child(Label::new(format!("Enter {code} at {url}")))
                }
                LoginStep::Prompt(prompt) => {
                    let message = prompt.prompt.message.clone();
                    let prompt = prompt.clone();
                    if self.prompt_id.as_deref() != Some(prompt.id.as_str()) {
                        self.prompt_id = Some(prompt.id.clone());
                        self.input = cx.new(|cx| {
                            let input = InputField::new(
                                window,
                                cx,
                                prompt.prompt.placeholder.as_deref().unwrap_or(""),
                            );
                            if matches!(prompt.prompt.kind, PromptKind::Secret) {
                                input.masked(true)
                            } else {
                                input
                            }
                        });
                    }
                    let mut content = root.child(Label::new(message));
                    if matches!(prompt.prompt.kind, PromptKind::Select) {
                        for option in &prompt.prompt.options {
                            let value = option.id.clone();
                            content = content.child(
                                v_flex()
                                    .gap_1()
                                    .child(
                                        Button::new(
                                            format!("login-choice-{}", option.id),
                                            option.label.clone(),
                                        )
                                        .style(ButtonStyle::Outlined)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                this.state.update(cx, |state, cx| {
                                                    state.submit_prompt(value.clone(), cx)
                                                });
                                            }),
                                        ),
                                    )
                                    .when_some(option.description.clone(), |this, description| {
                                        this.child(
                                            Label::new(description)
                                                .size(LabelSize::Small)
                                                .color(Color::Muted),
                                        )
                                    }),
                            );
                        }
                        content
                    } else {
                        content.child(self.input.clone()).child(
                            Button::new("submit", "Continue").on_click(cx.listener(
                                |this, _, window, cx| {
                                    let value = this.input.read(cx).text(cx);
                                    this.input.update(cx, |input, cx| input.clear(window, cx));
                                    this.state
                                        .update(cx, |state, cx| state.submit_prompt(value, cx));
                                },
                            )),
                        )
                    }
                }
                LoginStep::Failed(reason) => {
                    root.child(Label::new(format!("Sign-in failed: {reason}")))
                }
            };
            root = root.child(
                Button::new("cancel", "Cancel")
                    .style(ButtonStyle::Outlined)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.input.update(cx, |input, cx| input.clear(window, cx));
                        this.state.update(cx, |state, cx| state.cancel_login(cx));
                    })),
            );
            return root;
        }

        if self.prompt_id.take().is_some() {
            self.input.update(cx, |input, cx| input.clear(window, cx));
        }

        let mut buttons = h_flex().flex_wrap().gap_1();
        for option in &login_options {
            let option = option.clone();
            buttons = buttons.child(
                Button::new(
                    format!("login-{}-{:?}", option.provider_id, option.kind),
                    format!("{} — {}", option.provider_name, option.label),
                )
                .style(if option.is_subscription {
                    ButtonStyle::Filled
                } else {
                    ButtonStyle::Outlined
                })
                .disabled(self.loading)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state
                        .update(cx, |state, cx| state.start_login(option.clone(), cx));
                })),
            );
        }
        root.child(buttons).child(
            Button::new("refresh-connections", "Refresh connections and models")
                .style(ButtonStyle::Outlined)
                .disabled(self.loading)
                .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
        )
    }
}
