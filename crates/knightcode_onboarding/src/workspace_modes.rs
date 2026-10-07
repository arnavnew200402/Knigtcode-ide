use std::{collections::HashMap, sync::Arc};

use acp_thread::AcpThread;
use agent_client_protocol::schema::v1 as acp;
use agent_ui::{
    Agent, AgentPanel, AgentThreadSource, ConversationView, NewThread,
    thread_metadata_store::{ThreadId, ThreadMetadata, ThreadMetadataStore, WorktreePaths},
};
use anyhow::{Context as _, Result, anyhow};
use chrono::{Local, Utc};
use editor::Editor;
use gpui::{
    Action, AnyElement, App, AppContext as _, Context, Entity, EntityId, EventEmitter, FocusHandle,
    Focusable, Global, Image, ImageFormat, KeyContext, PathBuilder, Render, Subscription, Task,
    WeakEntity, Window, canvas, div, img, linear_color_stop, linear_gradient, point, rgb, rgba,
};
use knightcode_engine::session_controls::SessionMode;
use knightcode_models::{LocalModelsView, ModelPicker, SignInView, State};
use language_model::AuthenticateError;
use remote::RemoteConnectionOptions;
use settings::{DefaultOpenBehavior, Settings as _};
use title_bar::{KnightCodeMode, ShowAccount, ShowBuild, ShowChat, ShowHome, ShowModels, TitleBar};
use ui::{ButtonLike, Divider, TintColor, prelude::*, utils::WithRemSize};
use util::ResultExt as _;
use workspace::{
    AppState, OpenMode, OpenOptions, Pane, RecentWorkspace, SerializedWorkspaceLocation, Workspace,
    WorkspaceDb, WorkspaceSettings, ZoomIn, ZoomOut,
    item::{Item, ItemEvent, WeakItemHandle},
    notifications::DetachAndPromptErr as _,
    open_new,
    DockStructure,
    with_active_or_new_workspace,
};

#[derive(Default)]
struct WorkspaceModesRegistry(HashMap<EntityId, WeakEntity<WorkspaceModes>>);
impl Global for WorkspaceModesRegistry {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Chat,
    Models,
    Account,
}

impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Chat => "Chat",
            Self::Models => "Models",
            Self::Account => "Account",
        }
    }

    fn mode(self) -> KnightCodeMode {
        match self {
            Self::Chat => KnightCodeMode::Chat,
            _ => KnightCodeMode::Home,
        }
    }
}

pub(super) fn init(cx: &mut App) {
    cx.default_global::<WorkspaceModesRegistry>();
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        if let Some(window) = window {
            install(workspace, window, cx);
        }
    })
    .detach();
    cx.on_action(|_: &ShowHome, cx| {
        with_active_or_new_workspace(cx, show_home_in_workspace);
    });
    cx.on_action(|_: &ShowChat, cx| {
        with_active_or_new_workspace(cx, show_chat_in_workspace);
    });
    cx.on_action(|_: &ShowBuild, cx| {
        with_active_or_new_workspace(cx, show_build_in_workspace);
    });
    cx.on_action(|_: &ShowModels, cx| {
        with_active_or_new_workspace(cx, |workspace, window, cx| {
            show_page(workspace, Page::Models, window, cx);
        });
    });
    cx.on_action(|_: &ShowAccount, cx| {
        with_active_or_new_workspace(cx, |workspace, window, cx| {
            show_page(workspace, Page::Account, window, cx);
        });
    });
}

pub fn show_home(app_state: Arc<AppState>, cx: &mut App) -> Task<anyhow::Result<()>> {
    open_new(Default::default(), app_state, cx, show_home_in_workspace)
}

pub fn show_home_in_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    show_page(workspace, Page::Home, window, cx);
}

pub fn show_chat_in_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    show_page(workspace, Page::Chat, window, cx);
}

pub fn show_build_in_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let modes = install(workspace, window, cx);
    modes.update(cx, |modes, cx| modes.build(workspace, window, cx));
}

fn show_page(
    workspace: &mut Workspace,
    page: Page,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let modes = install(workspace, window, cx);
    modes.update(cx, |modes, cx| modes.show(workspace, page, window, cx));
}

fn install(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Entity<WorkspaceModes> {
    let workspace_id = cx.entity_id();
    if let Some(modes) = cx
        .default_global::<WorkspaceModesRegistry>()
        .0
        .get(&workspace_id)
        .and_then(WeakEntity::upgrade)
    {
        return modes;
    }
        let workspace_handle = workspace.weak_handle();
        let modes = cx.new(|cx| {
            let subscription = workspace_handle.upgrade().map(|workspace| {
            cx.subscribe_in(&workspace, window, |_this: &mut WorkspaceModes, workspace, event, window, cx| {
                if matches!(event, workspace::Event::ActiveItemChanged) {
                    let workspace = workspace.downgrade();
                    cx.defer_in(window, move |this, window, cx| {
                        workspace
                            .update(cx, |workspace, cx| this.reconcile(workspace, window, cx))
                            .log_err();
                    });
                }
            })
        });
        WorkspaceModes {
            workspace: workspace_handle.clone(),
            requested_mode: None,
            chat_page: None,
            panel: None,
            policy_task: None,
            policy_subscription: None,
            build_item: None,
            build_docks: None,
            build_pane: None,
            page_pane: None,
            pane_was_zoomed: false,
            _subscription: subscription,
        }
    });
    cx.default_global::<WorkspaceModesRegistry>()
        .0
        .insert(workspace_id, modes.downgrade());
    cx.on_release(move |_, cx| {
        cx.default_global::<WorkspaceModesRegistry>()
            .0
            .remove(&workspace_id);
    })
    .detach();

    workspace.register_action({
        let modes = modes.clone();
        move |workspace, _: &ShowHome, window, cx| {
            modes.update(cx, |modes, cx| {
                modes.show(workspace, Page::Home, window, cx)
            });
        }
    });
    workspace.register_action({
        let modes = modes.clone();
        move |workspace, _: &ShowChat, window, cx| {
            modes.update(cx, |modes, cx| {
                modes.show(workspace, Page::Chat, window, cx)
            });
        }
    });
    workspace.register_action({
        let modes = modes.clone();
        move |workspace, _: &ShowModels, window, cx| {
            modes.update(cx, |modes, cx| {
                modes.show(workspace, Page::Models, window, cx)
            });
        }
    });
    workspace.register_action({
        let modes = modes.clone();
        move |workspace, _: &ShowAccount, window, cx| {
            modes.update(cx, |modes, cx| {
                modes.show(workspace, Page::Account, window, cx)
            });
        }
    });
    workspace.register_action({
        let modes = modes.clone();
        move |workspace, _: &ShowBuild, window, cx| {
            modes.update(cx, |modes, cx| modes.build(workspace, window, cx));
        }
    });
    modes
}

struct WorkspaceModes {
    workspace: WeakEntity<Workspace>,
    requested_mode: Option<KnightCodeMode>,
    chat_page: Option<Entity<KnightCodePage>>,
    panel: Option<Entity<AgentPanel>>,
    policy_task: Option<Task<()>>,
    policy_subscription: Option<Subscription>,
    build_item: Option<Box<dyn WeakItemHandle>>,
    build_docks: Option<DockStructure>,
    build_pane: Option<WeakEntity<Pane>>,
    page_pane: Option<WeakEntity<Pane>>,
    pane_was_zoomed: bool,
    _subscription: Option<Subscription>,
}

impl WorkspaceModes {
    fn show(
        &mut self,
        workspace: &mut Workspace,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Navigation must not depend on an ACP session being available. The
        // conversation is loaded after the page is visible; otherwise a
        // missing agent, an empty draft, or an agent without session-mode
        // support leaves the mode switch permanently stuck on the previous
        // page.
        self.requested_mode = None;
        self.policy_task = None;
        self.policy_subscription = None;
        set_title_bar_operation(workspace, None, false, cx);
        self.show_ready(workspace, page, window, cx);
    }

    fn request_policy(
        &mut self,
        workspace: &mut Workspace,
        mode: KnightCodeMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.requested_mode.is_some() {
            return;
        }
        let panel = workspace.panel::<AgentPanel>(cx);
        let has_conversation = panel
            .as_ref()
            .is_some_and(|panel| panel.read(cx).active_conversation_view().is_some());
        if mode == KnightCodeMode::Build && !has_conversation {
            self.build_ready(workspace, window, cx);
            return;
        }
        let Some(panel) = panel else {
            set_title_bar_operation(
                workspace,
                Some("The agent panel is loading. Retry shortly.".into()),
                false,
                cx,
            );
            return;
        };
        if mode == KnightCodeMode::Chat {
            let existing_chat = workspace
                .items_of_type::<KnightCodePage>(cx)
                .find(|item| item.read(cx).page == Page::Chat);
            let item = existing_chat
                .or_else(|| self.chat_page.clone())
                .unwrap_or_else(|| {
                    cx.new(|cx| KnightCodePage::new(self.workspace.clone(), Page::Chat, window, cx))
                });
            item.update(cx, |item, cx| {
                item.attach_panel(Some(panel.clone()), cx);
                item.prepare_chat(window, cx);
            });
            self.chat_page = Some(item);
        }
        self.requested_mode = Some(mode);
        self.panel = Some(panel.clone());
        self.policy_subscription = Some(cx.observe_in(&panel, window, |this, _, window, cx| {
            this.advance_policy(window, cx);
        }));
        set_title_bar_operation(
            workspace,
            Some(format!("Opening {mode:?}…").into()),
            true,
            cx,
        );
        self.advance_policy(window, cx);
    }

    fn advance_policy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.policy_task.is_some() {
            return;
        }
        let Some(mode) = self.requested_mode else {
            return;
        };
        let thread = self
            .panel
            .as_ref()
            .and_then(|panel| panel.read(cx).active_conversation_view())
            .and_then(|conversation| conversation.read(cx).root_thread_view())
            .map(|view| view.read(cx).thread.clone());
        let Some(thread) = thread else {
            return;
        };
        let mode_task = set_session_mode(
            &thread,
            match mode {
                KnightCodeMode::Chat => SessionMode::Chat,
                _ => SessionMode::Build,
            },
            cx,
        );
        let workspace = self.workspace.clone();
        self.policy_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = mode_task.await;
            this.update_in(cx, |this, window, cx| {
                this.policy_task = None;
                this.policy_subscription = None;
                this.requested_mode = None;
                workspace
                    .update(cx, |workspace, cx| match result {
                        Ok(()) => {
                            set_title_bar_operation(workspace, None, false, cx);
                            if mode == KnightCodeMode::Chat {
                                this.show_ready(workspace, Page::Chat, window, cx);
                            } else {
                                this.build_ready(workspace, window, cx);
                            }
                        }
                        Err(error) => {
                            set_title_bar_operation(
                                workspace,
                                Some(format!("Could not enter {mode:?}: {error:#}").into()),
                                false,
                                cx,
                            );
                        }
                    })
                    .log_err();
                cx.notify();
            })
            .log_err();
        }));
        cx.notify();
    }

    fn show_ready(
        &mut self,
        workspace: &mut Workspace,
        page: Page,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.build_docks.is_none() {
            self.build_docks = Some(workspace.capture_dock_state(window, cx));
            if let Some(item) = workspace
                .active_item(cx)
                .filter(|item| item.downcast::<KnightCodePage>().is_none())
            {
                self.build_item = Some(item.downgrade_item());
            }
            let pane = workspace.active_pane();
            self.pane_was_zoomed = pane.read(cx).is_zoomed();
            self.build_pane = Some(pane.downgrade());
        }
        let existing = workspace
            .items_of_type::<KnightCodePage>(cx)
            .find(|item| item.read(cx).page == page);
        let item = if let Some(item) = existing {
            workspace.activate_item(&item, true, true, window, cx);
            item
        } else {
            let item = self
                .chat_page
                .clone()
                .filter(|_| page == Page::Chat)
                .unwrap_or_else(|| {
                    cx.new(|cx| KnightCodePage::new(workspace.weak_handle(), page, window, cx))
                });
            workspace.add_item_to_active_pane(Box::new(item.clone()), None, true, window, cx);
            item
        };
        if page == Page::Chat {
            self.chat_page = Some(item.clone());
        }
        for dock in workspace.all_docks() {
            dock.update(cx, |dock, cx| dock.set_open(false, window, cx));
        }
        let pane = workspace.active_pane().clone();
        self.page_pane = Some(pane.downgrade());
        pane.update(cx, |pane, cx| {
            // KnightCode's front pages should not look like a Zed editor tab.
            // Build mode restores the standard tab bar in restore().
            pane.set_should_display_tab_bar(|_, _| false);
            cx.notify();
            pane.zoom_in(&ZoomIn, window, cx);
        });
        set_title_bar_mode(workspace, page.mode(), cx);
        item.update(cx, |item, cx| {
            if page == Page::Home {
                cx.defer_in(window, |item, _, cx| item.refresh_recent(cx));
            } else if page == Page::Chat {
                item.attach_panel(workspace.panel::<AgentPanel>(cx), cx);
                cx.defer_in(window, |item, window, cx| item.prepare_chat(window, cx));
            }
        });
        window.focus(&item.focus_handle(cx), cx);
    }

    fn restore(
        &mut self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut App,
    ) {
        if let Some(docks) = self.build_docks.take() {
            let workspace_handle = self.workspace.clone();
            window.defer(cx, move |window, cx| {
                workspace_handle
                    .update(cx, |workspace, cx| {
                        workspace.set_dock_structure(docks, window, cx);
                    })
                    .log_err();
            });
        }
        if let Some(pane) = self.page_pane.take().and_then(|pane| pane.upgrade()) {
            pane.update(cx, |pane, cx| {
                pane.set_should_display_tab_bar(|_, _| true);
                cx.notify();
                pane.zoom_out(&ZoomOut, window, cx);
            });
        }
        if self.pane_was_zoomed {
            if let Some(pane) = self.build_pane.take().and_then(|pane| pane.upgrade()) {
                pane.update(cx, |pane, cx| pane.zoom_in(&ZoomIn, window, cx));
            }
        }
        self.build_pane = None;
        self.pane_was_zoomed = false;
        set_title_bar_mode(workspace, KnightCodeMode::Build, cx);
    }

    fn build(&mut self, workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Self>) {
        // Build is the normal Zed workspace. Restore the docks and the
        // pre-existing editor before activating the item, rather than waiting
        // for a Chat conversation or an ACP session-mode request.
        self.requested_mode = None;
        self.policy_task = None;
        self.policy_subscription = None;
        if self.build_docks.is_some() {
            self.restore(workspace, window, cx);
        }
        self.build_ready(workspace, window, cx);
    }

    fn build_ready(&mut self, workspace: &mut Workspace, window: &mut Window, cx: &mut App) {
        set_title_bar_mode(workspace, KnightCodeMode::Build, cx);
        // Build mode is the editor plus the right-side KnightCode agent dock,
        // matching the normal Zed workspace layout shown in the reference.
        workspace.reveal_panel::<AgentPanel>(window, cx);
        let item = self
            .build_item
            .as_ref()
            .and_then(|item| item.upgrade())
            .filter(|item| workspace.pane_for(item.as_ref()).is_some())
            .or_else(|| {
                workspace
                    .items(cx)
                    .find(|item| item.downcast::<KnightCodePage>().is_none())
                    .map(|item| item.boxed_clone())
            });
        if let Some(item) = item {
            workspace.activate_item(item.as_ref(), true, true, window, cx);
        } else {
            window.dispatch_action(workspace::NewFile.boxed_clone(), cx);
        }
    }

    fn reconcile(
        &mut self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        let page = workspace
            .active_item_as::<KnightCodePage>(cx)
            .map(|item| item.read(cx).page);
        if let Some(page) = page {
            set_title_bar_mode(workspace, page.mode(), cx);
            if self.build_docks.is_some() {
                for dock in workspace.all_docks() {
                    dock.update(cx, |dock, cx| dock.set_open(false, window, cx));
                }
            }
        } else {
            // Normally build() restores this before activating the editor.
            // Keep this fallback for a workspace restored with an editor
            // already active, and never route back through Chat here.
            if self.build_docks.is_some() {
                self.restore(workspace, window, cx);
            }
            self.build_item = workspace.active_item(cx).map(|item| item.downgrade_item());
        }
    }
}

fn root_thread(workspace: &Workspace, cx: &App) -> Option<Entity<AcpThread>> {
    workspace
        .panel::<AgentPanel>(cx)
        .and_then(|panel| panel.read(cx).active_conversation_view().cloned())
        .and_then(|conversation| conversation.read(cx).root_thread_view())
        .map(|view| view.read(cx).thread.clone())
}

fn session_policy_value(thread: &Entity<AcpThread>, config_id: &str, cx: &App) -> Option<String> {
    let thread = thread.read(cx);
    if config_id == "mode" {
        return thread
            .connection()
            .session_modes(thread.session_id(), cx)
            .map(|modes| modes.current_mode().0.to_string());
    }
    thread
        .connection()
        .session_config_options(thread.session_id(), cx)
        .and_then(|options| {
            options.config_options().into_iter().find_map(|option| {
                if option.id.0.as_ref() == config_id
                    && let acp::SessionConfigKind::Select(select) = option.kind
                {
                    Some(select.current_value.0.to_string())
                } else {
                    None
                }
            })
        })
}

fn set_session_mode(
    thread: &Entity<AcpThread>,
    mode: SessionMode,
    cx: &mut App,
) -> Task<Result<()>> {
    let current = session_policy_value(thread, "mode", cx);
    let thread = thread.read(cx);
    if thread.parent_session_id().is_some()
        || thread.connection().agent_id() != Agent::NativeAgent.id()
    {
        return Task::ready(Err(anyhow!(
            "Mode controls require the KnightCode parent conversation"
        )));
    }
    if current.as_deref() == Some(mode.as_str()) {
        return Task::ready(Ok(()));
    }
    if let Some(modes) = thread.connection().session_modes(thread.session_id(), cx) {
        return modes.set_mode(acp::SessionModeId::new(mode.as_str()), cx);
    }
    Task::ready(Err(anyhow!(
        "The KnightCode engine does not offer session mode controls"
    )))
}

fn set_title_bar_operation(
    workspace: &Workspace,
    message: Option<SharedString>,
    busy: bool,
    cx: &mut App,
) {
    if let Some(title_bar) = workspace
        .titlebar_item()
        .and_then(|item| item.downcast::<TitleBar>().ok())
    {
        title_bar.update(cx, |title_bar, cx| {
            title_bar.set_knightcode_operation(message, busy, cx)
        });
    }
}

fn set_title_bar_mode(workspace: &Workspace, mode: KnightCodeMode, cx: &mut App) {
    if let Some(title_bar) = workspace
        .titlebar_item()
        .and_then(|item| item.downcast::<TitleBar>().ok())
    {
        title_bar.update(cx, |title_bar, cx| title_bar.set_knightcode_mode(mode, cx));
    }
}

pub struct KnightCodePage {
    workspace: WeakEntity<Workspace>,
    page: Page,
    focus_handle: FocusHandle,
    logo: Arc<Image>,
    state: Option<Entity<State>>,
    models: Option<Entity<ModelPicker>>,
    local_models: Option<Entity<LocalModelsView>>,
    account: Option<Entity<SignInView>>,
    panel: Option<Entity<AgentPanel>>,
    panel_subscription: Option<Subscription>,
    search: Entity<Editor>,
    recent: Option<Vec<RecentWorkspace>>,
    recent_error: Option<SharedString>,
    state_error: Option<SharedString>,
    chat_error: Option<SharedString>,
    recent_task: Option<Task<()>>,
    state_task: Option<Task<()>>,
    chat_task: Option<Task<()>>,
    handoff_intent: Entity<Editor>,
    handoff_plan: Entity<Editor>,
    handoff_project: Option<std::path::PathBuf>,
    handoff_task: Option<Task<()>>,
    handoff_busy: bool,
    _subscriptions: Vec<Subscription>,
}

impl KnightCodePage {
    fn new(
        workspace: WeakEntity<Workspace>,
        page: Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Search chats…", window, cx);
            editor
        });
        let handoff_intent = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_placeholder_text("What should be built?", window, cx);
            editor.set_mode(editor::EditorMode::AutoHeight {
                min_lines: 2,
                max_lines: Some(6),
            });
            editor
        });
        let handoff_plan = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_placeholder_text("Optional plan or constraints", window, cx);
            editor.set_mode(editor::EditorMode::AutoHeight {
                min_lines: 2,
                max_lines: Some(6),
            });
            editor
        });
        let mut subscriptions = vec![cx.observe(&search, |_, _, cx| cx.notify())];
        if let Some(store) = ThreadMetadataStore::try_global(cx) {
            subscriptions.push(cx.observe(&store, |_, _, cx| cx.notify()));
        }
        if let Some(workspace) = workspace.upgrade() {
            subscriptions.push(cx.observe(&workspace, |_, _, cx| cx.notify()));
        }
        let state =
            knightcode_engine::try_global(cx).map(|engine| cx.new(|cx| State::new(engine, cx)));
        if let Some(state) = &state {
            subscriptions.push(cx.observe(state, |_, _, cx| cx.notify()));
        }
        let mut this = Self {
            workspace,
            page,
            focus_handle: cx.focus_handle(),
            logo: Arc::new(Image::from_bytes(ImageFormat::Png, super::LOGO.to_vec())),
            state,
            models: None,
            local_models: None,
            account: None,
            panel: None,
            panel_subscription: None,
            search,
            recent: None,
            recent_error: None,
            state_error: None,
            chat_error: None,
            recent_task: None,
            state_task: None,
            chat_task: None,
            handoff_intent,
            handoff_plan,
            handoff_project: None,
            handoff_task: None,
            handoff_busy: false,
            _subscriptions: subscriptions,
        };
        this.refresh_state(cx);
        this
    }

    fn refresh_state(&mut self, cx: &mut Context<Self>) {
        if self.state.is_none() {
            if let Some(engine) = knightcode_engine::try_global(cx) {
                let state = cx.new(|cx| State::new(engine, cx));
                self._subscriptions
                    .push(cx.observe(&state, |_, _, cx| cx.notify()));
                self.state = Some(state);
            }
        }
        let Some(state) = self.state.clone() else {
            return;
        };
        self.state_error = None;
        let refresh = state.update(cx, |state, cx| state.refresh(cx));
        self.state_task = Some(cx.spawn(async move |this, cx| {
            let result = refresh.await;
            this.update(cx, |this, cx| {
                this.state_error = match result {
                    Ok(()) | Err(AuthenticateError::CredentialsNotFound) => None,
                    Err(error) => Some(
                        format!("Could not refresh KnightCode accounts and models: {error:#}")
                            .into(),
                    ),
                };
                cx.notify();
            })
            .log_err();
        }));
    }

    fn refresh_recent(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let fs = workspace.read(cx).app_state().fs.clone();
        let database = WorkspaceDb::global(cx);
        self.recent_error = None;
        self.recent_task = Some(cx.spawn(async move |this, cx| {
            let result = database.recent_project_workspaces(fs.as_ref()).await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(recent) => this.recent = Some(recent),
                    Err(error) => {
                        this.recent_error =
                            Some(format!("Could not load recent projects: {error:#}").into())
                    }
                }
                cx.notify();
            })
            .log_err();
        }));
    }

    fn attach_panel(&mut self, panel: Option<Entity<AgentPanel>>, cx: &mut Context<Self>) {
        if self.panel.as_ref().map(Entity::entity_id) != panel.as_ref().map(Entity::entity_id) {
            self.panel_subscription = panel
                .as_ref()
                .map(|panel| cx.observe(panel, |_, _, cx| cx.notify()));
            self.panel = panel;
        }
    }

    fn ensure_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .panel
            .as_ref()
            .is_some_and(|panel| panel.read(cx).active_conversation_view().is_some())
        {
            return;
        }
        let metadata = self.workspace.upgrade().and_then(|workspace| {
            let project = workspace.read(cx).project().read(cx);
            let work_dirs = project.default_path_list(cx);
            let remote_connection = project.remote_connection_options(cx);
            let store = ThreadMetadataStore::try_global(cx)?;
            store
                .read(cx)
                .entries()
                .filter(|metadata| {
                    !metadata.archived
                        && metadata.folder_paths() == &work_dirs
                        && metadata.matches_remote_connection(remote_connection.as_ref())
                })
                .max_by_key(|metadata| metadata.updated_at)
                .cloned()
        });
        if let (Some(panel), Some(metadata)) = (&self.panel, metadata) {
            panel.update(cx, |panel, cx| {
                panel.load_agent_thread(
                    Agent::from(metadata.agent_id.clone()),
                    metadata.thread_id,
                    Some(metadata.folder_paths().clone()),
                    metadata.title(),
                    true,
                    AgentThreadSource::Sidebar,
                    window,
                    cx,
                )
            });
            self.chat_error = None;
            return;
        }
        self.new_chat(window, cx);
    }

    fn prepare_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reload = ThreadMetadataStore::try_global(cx).map(|store| store.read(cx).reload_task());
        self.chat_task = Some(cx.spawn_in(window, async move |this, cx| {
            if let Some(reload) = reload {
                reload.await;
            }
            this.update_in(cx, |this, window, cx| this.ensure_conversation(window, cx))
                .log_err();
        }));
    }

    pub fn mode(&self) -> KnightCodeMode {
        self.page.mode()
    }

    pub fn conversation_view(&self, cx: &App) -> Option<Entity<ConversationView>> {
        self.panel
            .as_ref()
            .and_then(|panel| panel.read(cx).active_conversation_view().cloned())
    }

    fn new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(panel) = self.panel.clone() else {
            self.chat_error =
                Some("The native agent panel is still loading. Retry when it is ready.".into());
            cx.notify();
            return;
        };
        if panel.read(cx).active_thread_is_draft(cx)
            && panel
                .read(cx)
                .active_thread_id(cx)
                .is_some_and(|thread_id| {
                    panel
                        .read(cx)
                        .editor_text(thread_id, cx)
                        .is_some_and(|text| text.trim().is_empty())
                })
        {
            if let Some(conversation) = panel.read(cx).active_conversation_view() {
                window.focus(&conversation.focus_handle(cx), cx);
            }
            return;
        }
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let Some(store) = ThreadMetadataStore::try_global(cx) else {
            self.chat_error = Some(
                "The native conversation store is not available. Restart KnightCode and retry."
                    .into(),
            );
            cx.notify();
            return;
        };
        let project = workspace.read(cx).project().read(cx);
        let work_dirs = project.default_path_list(cx);
        let remote_connection = project.remote_connection_options(cx);
        let thread_id = ThreadId::new();
        let now = Utc::now();
        // A scratch workspace has no worktree. Persist a real native draft first:
        // its stable ID lets the same loader restore it in Chat or the Build panel.
        store.update(cx, |store, cx| {
            store.save(
                ThreadMetadata {
                    thread_id,
                    session_id: None,
                    agent_id: Agent::NativeAgent.id(),
                    title: None,
                    title_override: None,
                    updated_at: now,
                    created_at: Some(now),
                    interacted_at: None,
                    worktree_paths: WorktreePaths::from_folder_paths(&work_dirs),
                    remote_connection,
                    archived: false,
                },
                cx,
            )
        });
        panel.update(cx, |panel, cx| {
            panel.load_agent_thread(
                Agent::NativeAgent,
                thread_id,
                Some(work_dirs),
                None,
                true,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            )
        });
        self.chat_error = None;
        cx.notify();
    }

    fn choose_handoff_project(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a project folder".into()),
        });
        self.handoff_task = Some(cx.spawn(async move |this, cx| {
            let selected = receiver.await.ok().and_then(Result::ok).flatten();
            if let Some(path) = selected.and_then(|paths| paths.into_iter().next()) {
                this.update(cx, |this, cx| {
                    this.handoff_project = Some(path);
                    cx.notify();
                })
                .ok();
            }
        }));
    }

    fn handoff_to_build(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.handoff_busy {
            return;
        }
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let Some(conversation) = self.conversation_view(cx) else {
            self.chat_error = Some("The conversation is still loading.".into());
            cx.notify();
            return;
        };
        let current_cwd = workspace.read(cx).project().read(cx).default_path_list(cx);
        let cwd = self
            .handoff_project
            .clone()
            .or_else(|| current_cwd.ordered_paths().next().cloned());
        let Some(cwd) = cwd else {
            self.chat_error = Some("Choose a project folder before continuing in Build.".into());
            cx.notify();
            return;
        };
        let text = nonempty_editor_text(&self.handoff_intent, cx);
        let plan = nonempty_editor_text(&self.handoff_plan, cx);
        let connection = conversation.read(cx).connection();
        let Some(connection) = connection else {
            self.chat_error = Some("The ACP connection is unavailable.".into());
            cx.notify();
            return;
        };
        let thread_id = conversation.read(cx).parent_id();
        self.handoff_busy = true;
        self.chat_error = None;
        let source_workspace = workspace.clone();
        let old_conversation = conversation.clone();
        self.handoff_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let target_workspace = source_workspace
                    .update_in(cx, |workspace, window, cx| {
                        workspace.open_workspace_for_paths(
                            OpenMode::NewWindow,
                            vec![cwd.clone()],
                            window,
                            cx,
                        )
                    })?
                    .await?;
                let (target_project, work_dirs, worktree_paths) = target_workspace.read_with(cx, |workspace, cx| {
                    let target_project = workspace.project().clone();
                    let work_dirs = target_project.read(cx).default_path_list(cx);
                    let worktree_paths = target_project.read(cx).worktree_paths(cx);
                    (target_project, work_dirs, worktree_paths)
                });
                let target_panel =
                    target_workspace.update_in(cx, |workspace, _window, cx| {
                        workspace
                            .panel::<AgentPanel>(cx)
                            .context("The target workspace agent panel is unavailable")
                    })??;
                let handoff = conversation.update_in(cx, |conversation, _window, cx| {
                    conversation.handoff_to_project(
                        cwd.clone(),
                        text.clone(),
                        plan.clone(),
                        target_project.clone(),
                        cx,
                    )
                })??;
                let _thread = handoff.await?;
                target_workspace.update_in(cx, |workspace, window, cx| {
                    target_panel.update(cx, |panel, cx| {
                        panel.adopt_connection(Agent::NativeAgent, connection.clone(), cx);
                        panel.load_agent_thread(
                            Agent::NativeAgent,
                            thread_id,
                            Some(work_dirs.clone()),
                            None,
                            true,
                            AgentThreadSource::AgentPanel,
                            window,
                            cx,
                        );
                        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                            store.update_worktree_paths(&[thread_id], worktree_paths.clone(), cx);
                            store.update_working_directories(thread_id, work_dirs.clone(), cx);
                        });
                        Ok::<(), anyhow::Error>(())
                    })?;
                    show_build_in_workspace(workspace, window, cx);
                    Ok::<(), anyhow::Error>(())
                })??;
                Ok::<(), anyhow::Error>(())
            }
            .await;
            this.update_in(cx, |this, _window, cx| {
                this.handoff_busy = false;
                if let Err(error) = result {
                    old_conversation
                        .update(cx, |conversation, cx| {
                            conversation
                                .set_preserve_session_on_release(false, cx)
                                .log_err();
                        });
                    this.chat_error =
                        Some(format!("Could not continue in Build: {error:#}").into());
                }
                cx.notify();
            })
            .log_err();
        }));
    }

    fn open_recent(
        &mut self,
        recent: RecentWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace
            .update(cx, |workspace, cx| {
                let paths = recent.paths.paths().to_vec();
                let open_mode = match WorkspaceSettings::get_global(cx).default_open_behavior {
                    DefaultOpenBehavior::ExistingWindow => OpenMode::Activate,
                    DefaultOpenBehavior::NewWindow => OpenMode::NewWindow,
                };
                match recent.location {
                    SerializedWorkspaceLocation::Local => {
                        let open_task =
                            workspace.open_workspace_for_paths(open_mode, paths, window, cx);
                        cx.spawn_in(window, async move |_, cx| {
                            let target_workspace = open_task.await?;
                            target_workspace.update_in(cx, |workspace, window, cx| {
                                show_build_in_workspace(workspace, window, cx);
                                Ok::<(), anyhow::Error>(())
                            })??;
                            Ok::<(), anyhow::Error>(())
                        })
                        .detach_and_prompt_err(
                            "Failed to open project",
                            window,
                            cx,
                            |_, _, _| None,
                        );
                    }
                    SerializedWorkspaceLocation::Remote(mut connection) => {
                        if let RemoteConnectionOptions::Ssh(connection) = &mut connection {
                            recent_projects::RemoteSettings::get_global(cx)
                                .fill_connection_options_from_settings(connection);
                        }
                        let app_state = workspace.app_state().clone();
                        let open_options = OpenOptions {
                            requesting_window: if matches!(open_mode, OpenMode::Activate) {
                                window
                                    .window_handle()
                                    .downcast::<workspace::MultiWorkspace>()
                            } else {
                                None
                            },
                            ..Default::default()
                        };
                        let open_task = cx.spawn_in(window, async move |_, cx| {
                            recent_projects::open_remote_project(
                                connection,
                                paths,
                                app_state,
                                open_options,
                                cx,
                            )
                            .await
                        });
                        cx.spawn_in(window, async move |_, cx| {
                            let target_window = open_task.await?;
                            target_window.update(cx, |multi_workspace, window, cx| {
                                multi_workspace.workspace().update(cx, |workspace, cx| {
                                    show_build_in_workspace(workspace, window, cx);
                                });
                            })?;
                            Ok::<(), anyhow::Error>(())
                        })
                        .detach_and_prompt_err(
                            "Failed to open project",
                            window,
                            cx,
                            |_, _, _| None,
                        );
                    }
                }
            })
            .log_err();
    }

    fn render_navigation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.page;
        v_flex()
            .relative()
            .flex_none()
            .w(px(220.))
            .h_full()
            .p_3()
            .gap_1()
            .border_r_1()
            .border_color(rgba(0xb4a0ff24))
            .bg(rgba(0x08061cd9))
            .when(page != Page::Chat, |navigation| {
                navigation
                    .children(
                        [
                            (
                                "home",
                                "Home",
                                IconName::Screen,
                                Some(Page::Home),
                                ShowHome.boxed_clone(),
                            ),
                            (
                                "chat",
                                "Chat",
                                IconName::Chat,
                                Some(Page::Chat),
                                ShowChat.boxed_clone(),
                            ),
                            (
                                "build",
                                "Build",
                                IconName::Code,
                                None,
                                ShowBuild.boxed_clone(),
                            ),
                            (
                                "projects",
                                "Projects",
                                IconName::Folder,
                                None,
                                zed_actions::OpenRecent::default().boxed_clone(),
                            ),
                            (
                                "models",
                                "Models",
                                IconName::Box,
                                Some(Page::Models),
                                ShowModels.boxed_clone(),
                            ),
                        ]
                        .into_iter()
                        .map(|(id, label, icon, target, action)| {
                            Button::new(id, label)
                                .full_width()
                                .start_icon(
                                    Icon::new(icon).color(Color::Custom(rgb(0xc4c8ea).into())),
                                )
                                .toggle_state(Some(page) == target)
                                .selected_label_color(Color::Custom(rgb(0xb19bff).into()))
                                .selected_style(ButtonStyle::Tinted(TintColor::Accent))
                                .on_click(move |_, window, cx| {
                                    window.dispatch_action(action.boxed_clone(), cx)
                                })
                        }),
                    )
                    .child(
                        Button::new("settings", "Settings")
                            .full_width()
                            .start_icon(Icon::new(IconName::Settings))
                            .on_click(|_, window, cx| {
                                window.dispatch_action(zed_actions::OpenSettings.boxed_clone(), cx)
                            }),
                    )
                    .child(div().flex_1())
            })
            .when(page == Page::Chat, |navigation| {
                navigation.child(self.render_history(cx))
            })
            .child(
                v_flex()
                    .p_3()
                    .gap_1()
                    .child(
                        Label::new("Ideas into Impact.").color(Color::Custom(rgb(0xb19bff).into())),
                    )
                    .child(div().h(px(2.)).w_8().bg(rgb(0x8b7cff))),
            )
    }

    fn render_history(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).text(cx).to_lowercase();
        let workspace = self.workspace.upgrade();
        let project = workspace
            .as_ref()
            .map(|workspace| workspace.read(cx).project().clone());
        let work_dirs = project
            .as_ref()
            .map(|project| project.read(cx).default_path_list(cx));
        let remote_connection = project
            .as_ref()
            .and_then(|project| project.read(cx).remote_connection_options(cx));
        let mut threads = ThreadMetadataStore::try_global(cx)
            .map(|store| {
                store
                    .read(cx)
                    .entries()
                    .filter(|metadata| {
                        !metadata.archived
                            && metadata.matches_remote_connection(remote_connection.as_ref())
                            && work_dirs
                                .as_ref()
                                .is_some_and(|paths| metadata.folder_paths() == paths)
                            && metadata.display_title().to_lowercase().contains(&query)
                    })
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        threads.sort_by_key(|metadata| std::cmp::Reverse(metadata.updated_at));
        let active_thread = self
            .panel
            .as_ref()
            .and_then(|panel| panel.read(cx).active_thread_id(cx));
        let today = Local::now().date_naive();
        let mut last_group = None;
        let mut entries = Vec::<AnyElement>::new();
        for metadata in threads {
            let age = today
                .signed_duration_since(metadata.updated_at.with_timezone(&Local).date_naive())
                .num_days();
            let group = match age {
                i64::MIN..=0 => "Today",
                1 => "Yesterday",
                2..=6 => "This week",
                _ => "Earlier",
            };
            if last_group != Some(group) {
                entries.push(
                    Label::new(group)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted)
                        .mt_3()
                        .into_any_element(),
                );
                last_group = Some(group);
            }
            let selected = active_thread == Some(metadata.thread_id);
            let title = metadata.display_title();
            entries.push(
                Button::new(metadata.thread_id.to_key_string(), title)
                    .full_width()
                    .toggle_state(selected)
                    .label_size(LabelSize::Small)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(panel) = &this.panel {
                            panel.update(cx, |panel, cx| {
                                panel.load_agent_thread(
                                    Agent::from(metadata.agent_id.clone()),
                                    metadata.thread_id,
                                    Some(metadata.folder_paths().clone()),
                                    metadata.title(),
                                    true,
                                    AgentThreadSource::Sidebar,
                                    window,
                                    cx,
                                )
                            });
                        }
                    }))
                    .into_any_element(),
            );
        }
        v_flex()
            .mt_3()
            .gap_2()
            .min_h_0()
            .flex_1()
            .child(
                Button::new("new-chat", "New Chat")
                    .full_width()
                    .start_icon(Icon::new(IconName::Plus))
                    .on_click(cx.listener(|this, _, window, cx| this.new_chat(window, cx))),
            )
            .child(
                div()
                    .p_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgba(0x8c78ff29))
                    .bg(rgba(0x121033bb))
                    .child(self.search.clone()),
            )
            .child(
                v_flex()
                    .id("chat-history")
                    .min_h_0()
                    .flex_1()
                    .gap_1()
                    .overflow_y_scroll()
                    .when(entries.is_empty(), |history| {
                        history.child(
                            Label::new("No matching conversations")
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    })
                    .children(entries),
            )
    }

    fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut recent_cards = Vec::<AnyElement>::new();
        for recent in self.recent.as_ref().into_iter().flatten().take(3) {
            let name = recent
                .identity_paths
                .paths()
                .iter()
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(", ");
            let paths = recent
                .paths
                .paths()
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let timestamp = recent
                .timestamp
                .with_timezone(&Local)
                .format("%b %-d, %Y · %H:%M")
                .to_string();
            let icon = match recent.location {
                SerializedWorkspaceLocation::Local => IconName::Folder,
                SerializedWorkspaceLocation::Remote(_) => IconName::Server,
            };
            let recent = recent.clone();
            recent_cards.push(
                div()
                    .flex_1()
                    .min_w(px(220.))
                    .rounded_xl()
                    .border_1()
                    .border_color(rgba(0x826eff33))
                    .bg(rgba(0x100c2ac7))
                    .child(
                        ButtonLike::new(format!("recent-{}", i64::from(recent.workspace_id)))
                            .full_width()
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .gap_3()
                                    .p_3()
                                    .child(
                                        Icon::new(icon).color(Color::Custom(rgb(0xb19bff).into())),
                                    )
                                    .child(
                                        v_flex()
                                            .min_w_0()
                                            .gap_1()
                                            .child(Label::new(name).truncate())
                                            .child(
                                                Label::new(paths)
                                                    .truncate()
                                                    .size(LabelSize::Small)
                                                    .color(Color::Muted),
                                            )
                                            .child(
                                                Label::new(timestamp)
                                                    .size(LabelSize::XSmall)
                                                    .color(Color::Muted),
                                            ),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_recent(recent.clone(), window, cx)
                            })),
                    )
                    .into_any_element(),
            );
        }
        v_flex().id("home-content").size_full().p_8().gap_6().overflow_y_scroll()
            .child(h_flex().w_full().gap_6().justify_between()
                .child(v_flex().flex_1().gap_3().justify_center()
                    .child(Label::new("THINK › BUILD › BEYOND").size(LabelSize::Small).color(Color::Custom(rgb(0x9aa0d6).into())))
                    .child(div().text_size(px(44.)).font_weight(gpui::FontWeight::BOLD).child("Turn your ideas"))
                    .child(div().text_size(px(44.)).font_weight(gpui::FontWeight::BOLD).text_color(rgb(0xb19bff)).child("into real impact."))
                    .child(Label::new("Chat with AI, or open a project and start building.").color(Color::Custom(rgb(0xb7bad8).into())))
                    .child(Label::new("Same intelligence. More possibilities.").color(Color::Custom(rgb(0xb7bad8).into()))))
                .child(img(self.logo.clone()).size(px(300.)).flex_none()))
            .child(h_flex().w_full().flex_wrap().gap_4()
                .child(v_flex().flex_1().min_w(px(240.)).p_5().gap_3().rounded_xl().border_1().border_color(rgba(0x826eff33)).bg(rgba(0x100c2ac7))
                    .child(Icon::new(IconName::Chat).color(Color::Custom(rgb(0xb19bff).into())))
                    .child(Headline::new("Chat"))
                    .child(Label::new("Ask, explore, brainstorm and get instant answers.").color(Color::Muted))
                    .child(Button::new("start-chat", "Start Chatting →").full_width().style(ButtonStyle::Tinted(TintColor::Accent))
                        .on_click(|_, window, cx| window.dispatch_action(ShowChat.boxed_clone(), cx))))
                .child(v_flex().flex_1().min_w(px(240.)).p_5().gap_3().rounded_xl().border_1().border_color(rgba(0x826eff33)).bg(rgba(0x100c2ac7))
                    .child(Icon::new(IconName::Code).color(Color::Custom(rgb(0xd9956c).into())))
                    .child(Headline::new("Build"))
                    .child(Label::new("Open a project, write code, run commands and build with AI.").color(Color::Muted))
                    .child(Button::new("open-project", "Open Project →").full_width().style(ButtonStyle::Tinted(TintColor::Accent))
                        .on_click(|_, window, cx| window.dispatch_action(workspace::Open::default().boxed_clone(), cx)))))
            .child(h_flex().justify_between()
                .child(h_flex().gap_2().child(Icon::new(IconName::Clock)).child(Label::new("Recent")))
                .child(Button::new("all-projects", "View all →").on_click(|_, window, cx| {
                    window.dispatch_action(zed_actions::OpenRecent::default().boxed_clone(), cx);
                })))
            .when_some(self.recent_error.clone(), |content, error| content.child(Label::new(error).color(Color::Error)))
            .child(h_flex().w_full().flex_wrap().gap_3()
                .when(self.recent.is_none() && self.recent_error.is_none(), |recent| recent.child(div().p_3().child(Label::new("Loading recent projects…").color(Color::Muted))))
                .when(self.recent.as_ref().is_some_and(Vec::is_empty), |recent| recent.child(div().p_3().child(Label::new("Your recent projects will appear here after you open a folder.").color(Color::Muted))))
                .children(recent_cards))
            .child(h_flex().gap_2()
                .child(Button::new("refresh-recents", "Refresh Recent Projects").on_click(cx.listener(|this, _, _, cx| this.refresh_recent(cx))))
                .child(Button::new("clone-project", "Clone Repository…").on_click(cx.listener(|this, _, window, cx| {
                    git_ui::clone::clone_and_open(SharedString::default(), this.workspace.clone(), window, cx, Arc::new(|_, _, _| {}));
                }))))
    }

    fn render_chat(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.workspace.upgrade().is_some_and(|workspace| {
            workspace
                .read(cx)
                .active_item_as::<KnightCodePage>(cx)
                .is_some_and(|item| item.entity_id() == cx.entity_id())
        });
        let panel_visible = self
            .workspace
            .upgrade()
            .is_some_and(|workspace| AgentPanel::is_visible(&workspace, cx));
        let conversation = (is_active && !panel_visible)
            .then(|| self.conversation_view(cx))
            .flatten();
        let show_loading = is_active && !panel_visible && conversation.is_none();
        let theme_settings = theme_settings::ThemeSettings::get_global(cx);
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .p_6()
            .gap_3()
            .when_some(self.chat_error.clone(), |stage, error| {
                stage.child(Label::new(error).color(Color::Error))
            })
            .when(show_loading, |stage| {
                stage.child(Label::new("Starting a new conversation…").color(Color::Muted))
            })
            .child(
                WithRemSize::new(theme_settings.agent_ui_font_size(cx))
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .font_family(theme_settings.agent_ui_font_family().clone())
                    .child(
                        v_flex()
                            .flex_1()
                            .size_full()
                            .min_h_0()
                            .min_w_0()
                            .when_some(conversation, |stage, conversation| {
                                stage.child(conversation)
                            })
                            .when(!is_active, |stage| {
                                stage.child(Button::new("activate-chat", "Open Chat").on_click(
                                    |_, window, cx| {
                                        window.dispatch_action(ShowChat.boxed_clone(), cx)
                                    },
                                ))
                            })
                            .when(is_active && panel_visible, |stage| {
                                stage.child(
                                    Button::new("return-to-chat", "Return to full-page Chat")
                                        .on_click(|_, window, cx| {
                                            window.dispatch_action(ShowChat.boxed_clone(), cx)
                                        }),
                                )
                            })
                            .when(self.panel.is_none(), |stage| {
                                stage.child(Button::new("retry-chat", "Retry Chat").on_click(
                                    cx.listener(|this, _, window, cx| {
                                        this.ensure_conversation(window, cx)
                                    }),
                                ))
                            }),
                    ),
            )
    }

    fn render_configuration(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.state.clone();
        let body = match (self.page, state) {
            (Page::Models, Some(state)) => {
                let models = self
                    .models
                    .get_or_insert_with(|| cx.new(|cx| ModelPicker::new(state.clone(), cx)))
                    .clone();
                let local_models = self
                    .local_models
                    .get_or_insert_with(|| cx.new(|cx| LocalModelsView::new(state, window, cx)))
                    .clone();
                v_flex()
                    .gap_4()
                    .child(local_models)
                    .child(Divider::horizontal())
                    .child(models)
                    .into_any_element()
            }
            (Page::Account, Some(state)) => self
                .account
                .get_or_insert_with(|| cx.new(|cx| SignInView::new(state, window, cx)))
                .clone()
                .into_any_element(),
            _ => Label::new("The KnightCode engine is not available yet.")
                .color(Color::Muted)
                .into_any_element(),
        };
        v_flex()
            .id("configuration-content")
            .size_full()
            .p_8()
            .gap_4()
            .overflow_y_scroll()
            .child(Headline::new(self.page.title()).size(HeadlineSize::Large))
            .child(
                Label::new(
                    "The same accounts and model choice are shared with the KnightCode CLI.",
                )
                .color(Color::Muted),
            )
            .when_some(self.state_error.clone(), |content, error| {
                content.child(Label::new(error).color(Color::Error))
            })
            .child(body)
            .child(
                Button::new("refresh-model-state", "Refresh")
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_state(cx))),
            )
    }
}

fn nonempty_editor_text(editor: &Entity<Editor>, cx: &App) -> Option<String> {
    let text = editor.read(cx).text(cx);
    (!text.trim().is_empty()).then_some(text)
}

impl Render for KnightCodePage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.page == Page::Chat {
            let panel = self
                .workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).panel::<AgentPanel>(cx));
            let attach = self.panel.is_none() && panel.is_some();
            self.attach_panel(panel, cx);
            if attach {
                cx.defer_in(window, |this, window, cx| this.prepare_chat(window, cx));
            }
        }
        let content = match self.page {
            Page::Home => self.render_home(cx).into_any_element(),
            Page::Chat => self.render_chat(cx).into_any_element(),
            Page::Models | Page::Account => {
                self.render_configuration(window, cx).into_any_element()
            }
        };
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("KnightCodePage");
        if self.page == Page::Chat {
            key_context.add("KnightCodeChat");
            key_context.add("AgentPanel");
        }
        h_flex()
            .id("knightcode-page")
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .overflow_hidden()
            .text_color(rgb(0xeef0ff))
            .bg(linear_gradient(
                180.,
                linear_color_stop(rgb(0x0a0720), 0.),
                linear_color_stop(rgb(0x1a0c38), 1.),
            ))
            .on_action(cx.listener(|this, _: &NewThread, window, cx| {
                if this.page == Page::Chat {
                    this.new_chat(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(
                |this, action: &zed_actions::IncreaseBufferFontSize, window, cx| {
                    if this.page == Page::Chat {
                        if let Some(panel) = &this.panel {
                            panel.update(cx, |panel, cx| {
                                panel.increase_font_size(action, window, cx)
                            });
                        }
                    } else {
                        cx.propagate();
                    }
                },
            ))
            .on_action(cx.listener(
                |this, action: &zed_actions::DecreaseBufferFontSize, window, cx| {
                    if this.page == Page::Chat {
                        if let Some(panel) = &this.panel {
                            panel.update(cx, |panel, cx| {
                                panel.decrease_font_size(action, window, cx)
                            });
                        }
                    } else {
                        cx.propagate();
                    }
                },
            ))
            .on_action(cx.listener(
                |this, action: &zed_actions::ResetBufferFontSize, window, cx| {
                    if this.page == Page::Chat {
                        if let Some(panel) = &this.panel {
                            panel.update(cx, |panel, cx| panel.reset_font_size(action, window, cx));
                        }
                    } else {
                        cx.propagate();
                    }
                },
            ))
            .child(scenic_background())
            .child(self.render_navigation(cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .min_h_0()
                    .child(content),
            )
    }
}

fn scenic_background() -> impl IntoElement {
    div()
        .absolute()
        .size_full()
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .top(relative(0.08))
                .right(relative(0.14))
                .size(px(240.))
                .rounded_full()
                .bg(linear_gradient(
                    135.,
                    linear_color_stop(rgba(0xf2edff70), 0.),
                    linear_color_stop(rgba(0x4a2fa038), 1.),
                )),
        )
        .child(
            canvas(
                |_, _, _| {},
                |bounds, _, window, _| {
                    for (color, vertices) in [
                        (
                            0x140a32,
                            vec![
                                (0., 0.74),
                                (0.12, 0.59),
                                (0.22, 0.70),
                                (0.33, 0.53),
                                (0.47, 0.74),
                                (0.59, 0.66),
                                (0.72, 0.81),
                                (0.86, 0.72),
                                (1., 0.82),
                            ],
                        ),
                        (
                            0x0c0624,
                            vec![
                                (0., 0.83),
                                (0.15, 0.88),
                                (0.28, 0.73),
                                (0.42, 0.84),
                                (0.55, 0.78),
                                (0.67, 0.88),
                                (0.83, 0.82),
                                (1., 0.91),
                            ],
                        ),
                    ] {
                        let mut path = PathBuilder::fill();
                        path.move_to(point(bounds.origin.x, bounds.bottom()));
                        for (x, y) in vertices {
                            path.line_to(point(
                                bounds.origin.x + bounds.size.width * x,
                                bounds.origin.y + bounds.size.height * y,
                            ));
                        }
                        path.line_to(point(bounds.right(), bounds.bottom()));
                        path.close();
                        if let Some(path) = path.build().log_err() {
                            window.paint_path(path, rgb(color));
                        }
                    }
                },
            )
            .size_full(),
        )
        .children(
            [
                (0.12, 0.18),
                (0.28, 0.08),
                (0.63, 0.14),
                (0.81, 0.22),
                (0.44, 0.28),
                (0.90, 0.40),
            ]
            .into_iter()
            .map(|(x, y)| {
                div()
                    .absolute()
                    .left(relative(x))
                    .top(relative(y))
                    .size(px(2.))
                    .rounded_full()
                    .bg(rgba(0xcbb6ffb3))
            }),
        )
}

impl EventEmitter<ItemEvent> for KnightCodePage {}

impl Focusable for KnightCodePage {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.page == Page::Chat {
            if let Some(conversation) = self
                .panel
                .as_ref()
                .and_then(|panel| panel.read(cx).active_conversation_view())
            {
                return conversation.focus_handle(cx);
            }
        }
        self.focus_handle.clone()
    }
}

impl Item for KnightCodePage {
    type Event = ItemEvent;

    fn tab_content_text(&self, _: usize, _: &App) -> SharedString {
        self.page.title().into()
    }

    fn show_toolbar(&self) -> bool {
        false
    }

    fn include_in_nav_history() -> bool {
        false
    }

    fn to_item_events(event: &Self::Event, emit: &mut dyn FnMut(ItemEvent)) {
        emit(*event);
    }
}
