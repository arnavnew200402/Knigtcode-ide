//! KnightCode's first run.
//!
//! Four steps, in the order the architecture fixes: welcome, sign in, pick a
//! model, open something. It replaces the stopgap that used to open an empty
//! buffer, and it replaces the upstream editor's own onboarding and welcome
//! pages, which are hidden rather than modified.
//!
//! Steps two and three are the same components Settings > AI shows, so a person
//! who skips first run and a person who does not end up in the same place. Both
//! are skipped when they are already satisfied: a machine where the CLI is
//! signed in is signed in here, because it is the same `auth.json`, and a
//! machine where a model is already recorded already has one.

mod build_layout;
mod build_welcome;
mod workspace_modes;

pub use title_bar::{KnightCodeMode, ShowAccount, ShowBuild, ShowChat, ShowHome, ShowModels};
pub use workspace_modes::{
    KnightCodePage, show_build_in_workspace, show_chat_in_workspace, show_home,
    show_home_in_workspace,
};

use db::kvp::KeyValueStore;
use fs::Fs;
use gpui::{
    Action, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, Image,
    ImageFormat, Render, SharedString, Subscription, Task, WeakEntity, Window, actions, img,
};
use knightcode_engine::report::{self, FirstRunOutcome, FirstRunStep};
use knightcode_models::{ModelPicker, SignInView, State, TelemetrySwitch};
use release_channel::AppVersion;
use settings::{BaseKeymap, Settings as _, update_settings_file};
use std::sync::Arc;
use theme_settings::{ThemeAppearanceMode, ThemeSettings};
use ui::{
    Button, ButtonStyle, Divider, Headline, HeadlineSize, Label, LabelSize, ParentElement as _,
    Styled as _, ToggleButtonGroup, ToggleButtonSimple, h_flex, prelude::*, v_flex,
};
use workspace::{
    AppState, Workspace, WorkspaceId,
    item::{Item, ItemEvent},
    open_new, with_active_or_new_workspace,
};

pub use workspace::welcome::ShowWelcome;

/// The key the stopgap wrote, and the one finishing still writes. Shared with
/// the upstream crate so a machine that ran either does not see first run twice.
pub const FIRST_OPEN: &str = "first_open";

/// The full-colour logo, the same file the About window shows. The in-UI
/// icons are single-colour masks; this is the one place the logo is big
/// enough to be shown as it is on the website.
const LOGO: &[u8] = include_bytes!("../../zed/resources/app-icon.png");

actions!(
    knightcode,
    [
        /// Opens KnightCode's first-run screen.
        ShowFirstRun
    ]
);

pub fn init(cx: &mut App) {
    workspace_modes::init(cx);
    cx.on_action(|_: &ShowFirstRun, cx| {
        with_active_or_new_workspace(cx, |workspace, window, cx| {
            show_in_workspace(workspace, window, cx);
        });
    });
}

/// Opens a workspace on the first-run screen. This is what `main` calls in
/// place of opening an empty buffer.
pub fn show_first_run(app_state: Arc<AppState>, cx: &mut App) -> Task<anyhow::Result<()>> {
    open_new(
        Default::default(),
        app_state,
        cx,
        |workspace, window, cx| {
            show_in_workspace(workspace, window, cx);
        },
    )
}

fn show_in_workspace(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let existing = workspace
        .active_pane()
        .read(cx)
        .items()
        .find_map(|item| item.downcast::<FirstRun>());
    if let Some(existing) = existing {
        workspace.activate_item(&existing, true, true, window, cx);
        return;
    }
    let handle = workspace.weak_handle();
    let item = cx.new(|cx| FirstRun::new(handle, cx));
    workspace.add_item_to_active_pane(Box::new(item), None, true, window, cx);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Welcome,
    SignIn,
    Model,
    Open,
}

impl Step {
    const ALL: [Step; 4] = [Step::Welcome, Step::SignIn, Step::Model, Step::Open];

    fn reported(self) -> FirstRunStep {
        match self {
            Step::Welcome => FirstRunStep::Welcome,
            Step::SignIn => FirstRunStep::SignIn,
            Step::Model => FirstRunStep::Model,
            Step::Open => FirstRunStep::Open,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Step::Welcome => "Welcome",
            Step::SignIn => "Sign in",
            Step::Model => "Pick a model",
            Step::Open => "Open something",
        }
    }
}

pub struct FirstRun {
    workspace: WeakEntity<Workspace>,
    state: Option<Entity<State>>,
    sign_in: Option<Entity<SignInView>>,
    models: Option<Entity<ModelPicker>>,
    telemetry: Option<Entity<TelemetrySwitch>>,
    step: Step,
    /// Set when the user reaches the last step and finishes. An item dropped
    /// before that is an abandonment, and §12.1 wants to know at which step.
    finished: bool,
    /// Held rather than looked up, because `Drop` has no `App` and an abandoned
    /// first run is the half of the signal that matters.
    reporter: Option<Arc<report::Reporter>>,
    logo: Arc<Image>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl FirstRun {
    fn new(workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        // Its own `State` over the same engine. Settings > AI holds another;
        // both subscribe to the engine's `models.changed` and `account.changed`,
        // so they cannot drift — and neither of them holds a credential.
        let state =
            knightcode_engine::try_global(cx).map(|engine| cx.new(|cx| State::new(engine, cx)));

        let mut subscriptions = Vec::new();
        if let Some(state) = &state {
            subscriptions.push(cx.observe(state, |_, _, cx| cx.notify()));
            // A clean machine has not asked the engine anything yet.
            state.update(cx, |state, cx| state.refresh(cx)).detach();
        }
        let telemetry = knightcode_engine::try_global(cx)
            .map(|engine| cx.new(|cx| TelemetrySwitch::new(engine, cx)));

        Self {
            workspace,
            state,
            sign_in: None,
            models: None,
            telemetry,
            step: Step::Welcome,
            finished: false,
            reporter: report::Reporter::try_global(cx),
            logo: Arc::new(Image::from_bytes(ImageFormat::Png, LOGO.to_vec())),
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// §6.3: a machine the CLI has already signed in is already signed in here.
    fn signed_in(&self, cx: &App) -> bool {
        self.state
            .as_ref()
            .is_some_and(|state| !state.read(cx).accounts.is_empty())
    }

    fn has_model(&self, cx: &App) -> bool {
        self.state
            .as_ref()
            .is_some_and(|state| state.read(cx).default_model().is_some())
    }

    fn satisfied(&self, step: Step, cx: &App) -> bool {
        match step {
            Step::Welcome | Step::Open => true,
            Step::SignIn => self.signed_in(cx),
            Step::Model => self.has_model(cx),
        }
    }

    /// Constructs the item without a workspace, for tests that are about which
    /// step is reachable rather than about the pane.
    #[cfg(test)]
    fn for_tests(cx: &mut Context<Self>) -> Self {
        Self::new(WeakEntity::new_invalid(), cx)
    }

    #[cfg(test)]
    fn advance_for_tests(&mut self, cx: &mut Context<Self>) {
        let index = Step::ALL.iter().position(|step| *step == self.step);
        let Some(index) = index else { return };
        self.step = Step::ALL[index + 1..]
            .iter()
            .find(|step| !self.satisfied(**step, cx))
            .copied()
            .unwrap_or(Step::Open);
    }

    /// Whether the user may leave the step they are on. Steps two and three are
    /// gates rather than pages: three of the five surfaces are dead without a
    /// recorded model, so "Continue" that skips it would be a broken install.
    fn can_advance(&self, cx: &App) -> bool {
        self.satisfied(self.step, cx)
    }

    /// Skips anything already satisfied, so a machine with a CLI login and a
    /// recorded model goes welcome → open in one click.
    fn advance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.step;
        let index = Step::ALL.iter().position(|step| *step == self.step);
        let Some(index) = index else { return };
        self.step = Step::ALL[index + 1..]
            .iter()
            .find(|step| !self.satisfied(**step, cx))
            .copied()
            .unwrap_or(Step::Open);
        if self.step == before {
            return;
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn finish(&mut self, cx: &mut Context<Self>) {
        self.finished = true;
        report::report(
            report::Event::FirstRun {
                outcome: FirstRunOutcome::Completed,
                step: Step::Open.reported(),
            },
            cx,
        );
        write_first_open(cx);
        // The pane closes an item that asks to be closed; nothing here needs to
        // know which pane it landed in.
        cx.emit(ItemEvent::CloseItem);
    }
}

/// Written whether first run is completed or abandoned: it exists so the branch
/// is taken once, not to record success.
fn write_first_open(cx: &mut App) {
    let store = KeyValueStore::global(cx);
    db::write_and_log(cx, move || async move {
        store
            .write_kvp(FIRST_OPEN.to_string(), "false".to_string())
            .await
    });
}

impl Drop for FirstRun {
    fn drop(&mut self) {
        // A completed run reports itself in `finish`; anything else is someone
        // closing the tab, and which step they were on is the whole signal.
        if self.finished {
            return;
        }
        if let Some(reporter) = &self.reporter {
            reporter.report(report::Event::FirstRun {
                outcome: FirstRunOutcome::Abandoned,
                step: self.step.reported(),
            });
        }
    }
}

impl FirstRun {
    fn render_welcome(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme_mode = ThemeSettings::get_global(cx)
            .theme
            .mode()
            .unwrap_or(ThemeAppearanceMode::System);
        let base_keymap = *BaseKeymap::get_global(cx);

        v_flex()
            .gap_4()
            .child(
                Label::new("KnightCode is an editor with one agent, one sign-in and one model choice. \
                    The same account serves the agent panel, Cmd+K in a buffer, Cmd+K in the terminal, \
                    commit messages and Tab.")
                    .color(Color::Muted),
            )
            .child(Divider::horizontal())
            .child(Label::new("Appearance").size(LabelSize::Small).color(Color::Muted))
            .child(
                ToggleButtonGroup::single_row(
                    "knightcode-theme-mode",
                    [
                        ToggleButtonSimple::new("Light", |_, _, cx| write_mode(ThemeAppearanceMode::Light, cx)),
                        ToggleButtonSimple::new("Dark", |_, _, cx| write_mode(ThemeAppearanceMode::Dark, cx)),
                        ToggleButtonSimple::new("System", |_, _, cx| write_mode(ThemeAppearanceMode::System, cx)),
                    ],
                )
                .selected_index(match theme_mode {
                    ThemeAppearanceMode::Light => 0,
                    ThemeAppearanceMode::Dark => 1,
                    ThemeAppearanceMode::System => 2,
                })
                .style(ui::ToggleButtonGroupStyle::Outlined),
            )
            .child(Label::new("Keybindings").size(LabelSize::Small).color(Color::Muted))
            .child(
                ToggleButtonGroup::single_row(
                    "knightcode-base-keymap",
                    [
                        ToggleButtonSimple::new("KnightCode", |_, _, cx| write_keymap(BaseKeymap::default(), cx)),
                        ToggleButtonSimple::new("VS Code", |_, _, cx| write_keymap(BaseKeymap::VSCode, cx)),
                        ToggleButtonSimple::new("JetBrains", |_, _, cx| write_keymap(BaseKeymap::JetBrains, cx)),
                        ToggleButtonSimple::new("Sublime Text", |_, _, cx| write_keymap(BaseKeymap::SublimeText, cx)),
                    ],
                )
                .selected_index(match base_keymap {
                    BaseKeymap::VSCode => 1,
                    BaseKeymap::JetBrains => 2,
                    BaseKeymap::SublimeText => 3,
                    _ => 0,
                })
                .style(ui::ToggleButtonGroupStyle::Outlined),
            )
            .child(Divider::horizontal())
            .children(self.telemetry.clone())
    }

    fn render_sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(state) = self.state.clone() else {
            return v_flex().child(Label::new(
                "The engine is not running, so there is nothing to sign in to yet.",
            ));
        };
        if self.signed_in(cx) {
            let who = state
                .read(cx)
                .accounts
                .iter()
                .map(|account| account.provider_name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            return v_flex().gap_2().child(
                Label::new(format!(
                    "Already signed in to {who}. KnightCode shares one login with the CLI, so there is nothing to do here."
                ))
                .color(Color::Muted),
            );
        }
        let view = self
            .sign_in
            .get_or_insert_with(|| cx.new(|cx| SignInView::new(state, window, cx)))
            .clone();
        v_flex()
            .gap_2()
            .child(
                Label::new(
                    "One sign-in serves every AI surface. KnightCode holds no API key: the credential \
                     lives in the shared auth.json the CLI already uses.",
                )
                .color(Color::Muted),
            )
            .child(view)
    }

    fn render_model(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(state) = self.state.clone() else {
            return v_flex().child(Label::new("The engine is not running."));
        };
        let picker = self
            .models
            .get_or_insert_with(|| cx.new(|cx| ModelPicker::new(state, cx)))
            .clone();
        v_flex()
            .gap_2()
            .child(
                Label::new(
                    "Pick the model KnightCode uses for inline assist, commit messages and Tab. \
                     Nothing is chosen for you, and you can change it in Settings > AI.",
                )
                .color(Color::Muted),
            )
            .child(picker)
    }

    fn render_open(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.workspace.clone();
        v_flex()
            .gap_2()
            .child(Label::new("Open a project to get started.").color(Color::Muted))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("open-folder", "Open a folder…")
                            .style(ButtonStyle::Filled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish(cx);
                                window
                                    .dispatch_action(workspace::Open::default().boxed_clone(), cx);
                            })),
                    )
                    .child(
                        Button::new("clone-repo", "Clone a repository…")
                            .style(ButtonStyle::Outlined)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.finish(cx);
                                git_ui::clone::clone_and_open(
                                    SharedString::default(),
                                    workspace.clone(),
                                    window,
                                    cx,
                                    Arc::new(|_, _, _| {}),
                                );
                            })),
                    )
                    .child(
                        Button::new("skip", "Skip for now")
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.finish(cx);
                                window.dispatch_action(ShowHome.boxed_clone(), cx);
                            })),
                    ),
            )
    }
}

fn write_mode(mode: ThemeAppearanceMode, cx: &mut App) {
    let fs = <dyn Fs>::global(cx);
    update_settings_file(fs, cx, move |settings, _cx| {
        theme_settings::set_mode(settings, mode);
    });
}

fn write_keymap(base_keymap: BaseKeymap, cx: &mut App) {
    let fs = <dyn Fs>::global(cx);
    update_settings_file(fs, cx, move |settings, _cx| {
        settings.base_keymap = Some(base_keymap.into());
    });
}

impl Render for FirstRun {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let step = self.step;
        let can_advance = self.can_advance(cx);
        let is_last = step == Step::Open;

        let body = match step {
            Step::Welcome => self.render_welcome(cx).into_any_element(),
            Step::SignIn => self.render_sign_in(window, cx).into_any_element(),
            Step::Model => self.render_model(cx).into_any_element(),
            Step::Open => self.render_open(cx).into_any_element(),
        };

        v_flex()
            .id("knightcode-first-run")
            .size_full()
            .items_center()
            .justify_center()
            .track_focus(&self.focus_handle)
            .child(
                v_flex()
                    .w(ui::rems(34.))
                    .gap_4()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(img(self.logo.clone()).size_10().rounded_md().flex_none())
                            .child(Headline::new("KnightCode").size(HeadlineSize::Large))
                            .child(
                                Label::new(AppVersion::global(cx).to_string())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(h_flex().gap_1().children(Step::ALL.iter().map(|entry| {
                        Label::new(entry.title())
                            .size(LabelSize::Small)
                            .color(if *entry == step {
                                Color::Accent
                            } else {
                                Color::Muted
                            })
                    })))
                    .child(Divider::horizontal())
                    .child(body)
                    .child(Divider::horizontal())
                    .when(!is_last, |this| {
                        this.child(
                            h_flex().justify_end().child(
                                Button::new("continue", "Continue")
                                    .style(ButtonStyle::Filled)
                                    .disabled(!can_advance)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.advance(window, cx)),
                                    ),
                            ),
                        )
                    }),
            )
    }
}

impl EventEmitter<ItemEvent> for FirstRun {}

impl Focusable for FirstRun {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Item for FirstRun {
    type Event = ItemEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        "Welcome to KnightCode".into()
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn can_split(&self) -> bool {
        false
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<WorkspaceId>,
        _: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Task<Option<Entity<Self>>> {
        Task::ready(None)
    }

    fn to_item_events(event: &Self::Event, f: &mut dyn FnMut(ItemEvent)) {
        f(*event)
    }
}

#[cfg(test)]
mod tests;
