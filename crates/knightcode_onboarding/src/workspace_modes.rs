mod home_presentation;

use std::{collections::HashMap, sync::Arc};

use agent_ui::{
    Agent, AgentPanel, AgentThreadSource, ConversationView, NewThread,
    thread_metadata_store::{ThreadId, ThreadMetadata, ThreadMetadataStore, WorktreePaths},
};
use chrono::{Local, Utc};
use editor::Editor;
use gpui::{
    Action, AnyElement, App, AppContext as _, Context, Entity, EntityId, EventEmitter, FocusHandle,
    Focusable, Global, Image, ImageFormat, KeyContext, PathBuilder, Render, Subscription, Task,
    WeakEntity, Window, canvas, div, img, linear_color_stop, linear_gradient, point, rgb, rgba,
};
use knightcode_models::{LocalModelsView, ModelPicker, SignInView, State};
use language_model::AuthenticateError;
use remote::RemoteConnectionOptions;
use settings::{DefaultOpenBehavior, Settings as _, SettingsStore};
use title_bar::{KnightCodeMode, ShowAccount, ShowBuild, ShowChat, ShowHome, ShowModels, TitleBar};
use ui::{ButtonLike, Divider, TintColor, prelude::*, utils::WithRemSize};
use util::ResultExt as _;
use workspace::{
    AppState, DockStructure, OpenMode, OpenOptions, Pane, RecentWorkspace,
    SerializedWorkspaceLocation, Workspace, WorkspaceDb, WorkspaceSettings, ZoomIn, ZoomOut,
    item::{Item, ItemEvent, WeakItemHandle},
    notifications::DetachAndPromptErr as _,
    open_new, with_active_or_new_workspace,
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
    super::build_layout::init(cx);
    cx.bind_keys([gpui::KeyBinding::new(
        if cfg!(target_os = "macos") {
            "cmd-k"
        } else {
            "ctrl-k"
        },
        zed_actions::command_palette::Toggle,
        Some("KnightCodeHome"),
    )]);
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
            cx.subscribe_in(
                &workspace,
                window,
                |_this: &mut WorkspaceModes, workspace, event, window, cx| {
                    if matches!(event, workspace::Event::ActiveItemChanged) {
                        let workspace = workspace.downgrade();
                        cx.defer_in(window, move |this, window, cx| {
                            workspace
                                .update(cx, |workspace, cx| this.reconcile(workspace, window, cx))
                                .log_err();
                        });
                    }
                },
            )
        });
        let layout_subscription = workspace_handle.upgrade().map(|workspace| {
            cx.observe_in(
                &workspace,
                window,
                |modes: &mut WorkspaceModes, workspace, window, cx| {
                    if !workspace.read(cx).is_knightcode_build() {
                        return;
                    }
                    let mask = super::build_layout::panel_mask(workspace.read(cx), cx);
                    if mask != modes.last_build_panel_mask {
                        modes.last_build_panel_mask = mask;
                        let workspace = workspace.downgrade();
                        cx.defer_in(window, move |modes, window, cx| {
                            workspace
                                .update(cx, |workspace, cx| {
                                    if workspace.active_item_as::<KnightCodePage>(cx).is_none() {
                                        modes.enable_workbench(workspace, window, cx);
                                    }
                                })
                                .log_err();
                        });
                    }
                },
            )
        });
        WorkspaceModes {
            workspace: workspace_handle.clone(),
            chat_page: None,
            cached_pages: Vec::new(),
            build_item: None,
            build_docks: None,
            build_pane: None,
            page_pane: None,
            pane_was_zoomed: false,
            build_layout_initialized: false,
            last_build_panel_mask: 0,
            _layout_subscription: layout_subscription,
            previous_build_theme: None,
            build_theme_selection: None,
            _theme_subscription: cx.observe_global::<SettingsStore>(
                |modes: &mut WorkspaceModes, cx| {
                    if modes.previous_build_theme.is_some()
                        && modes.build_theme_selection.as_ref()
                            == Some(&theme_settings::ThemeSettings::get_global(cx).theme)
                        && modes
                            .workspace
                            .upgrade()
                            .is_some_and(|workspace| workspace.read(cx).is_knightcode_build())
                        && theme::GlobalTheme::theme(cx).name.as_ref() != "KnightCode Workbench"
                        && let Ok(theme) =
                            theme::ThemeRegistry::global(cx).get("KnightCode Workbench")
                    {
                        // Model/reasoning changes also update SettingsStore. Keep the
                        // temporary workbench palette unless the user changes themes.
                        let theme = theme_settings::ThemeSettings::get_global(cx)
                            .apply_theme_overrides(theme);
                        theme::GlobalTheme::update_theme(cx, theme);
                    }
                },
            ),
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
    let initial_modes = modes.downgrade();
    cx.defer_in(window, move |workspace, window, cx| {
        if workspace.active_item_as::<KnightCodePage>(cx).is_none()
            && workspace
                .project()
                .read(cx)
                .visible_worktrees(cx)
                .next()
                .is_some()
        {
            initial_modes
                .update(cx, |modes, cx| {
                    if !modes.build_layout_initialized {
                        modes.build_ready(workspace, window, cx);
                    }
                })
                .log_err();
        }
    });
    modes
}

struct WorkspaceModes {
    workspace: WeakEntity<Workspace>,
    chat_page: Option<Entity<KnightCodePage>>,
    cached_pages: Vec<Entity<KnightCodePage>>,
    build_item: Option<Box<dyn WeakItemHandle>>,
    build_docks: Option<DockStructure>,
    build_pane: Option<WeakEntity<Pane>>,
    page_pane: Option<WeakEntity<Pane>>,
    pane_was_zoomed: bool,
    build_layout_initialized: bool,
    last_build_panel_mask: u8,
    _layout_subscription: Option<Subscription>,
    previous_build_theme: Option<Arc<theme::Theme>>,
    build_theme_selection: Option<theme_settings::ThemeSelection>,
    _theme_subscription: Subscription,
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
        set_title_bar_operation(workspace, None, false, cx);
        self.show_ready(workspace, page, window, cx);
    }

    fn show_ready(
        &mut self,
        workspace: &mut Workspace,
        page: Page,
        window: &mut Window,
        cx: &mut App,
    ) {
        workspace.set_knightcode_front_page(true, cx);
        if let Some(previous) = self.previous_build_theme.take() {
            // The workbench theme is temporary; never rewrite the user's
            // persisted theme or undo a theme they selected while in Build.
            if theme::GlobalTheme::theme(cx).name.as_ref() == "KnightCode Workbench"
                && self.build_theme_selection.as_ref()
                    == Some(&theme_settings::ThemeSettings::get_global(cx).theme)
            {
                theme::GlobalTheme::update_theme(cx, previous);
                window.refresh();
            }
        }
        self.build_theme_selection = None;
        if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
            panel.update(cx, |panel, cx| {
                panel.set_build_presentation(false, window, cx)
            });
        }
        let workspace_handle = self.workspace.clone();
        window.defer(cx, move |_, cx| {
            workspace_handle
                .update(cx, |workspace, cx| {
                    if workspace.active_item_as::<KnightCodePage>(cx).is_some() {
                        workspace.set_knightcode_workbench(Vec::new(), cx);
                        if let Some(panel) = workspace.panel::<project_panel::ProjectPanel>(cx) {
                            panel.update(cx, |_, cx| cx.notify());
                        }
                        if let Some(panel) =
                            workspace.panel::<terminal_view::terminal_panel::TerminalPanel>(cx)
                        {
                            panel.update(cx, |_, cx| cx.notify());
                        }
                    }
                })
                .log_err();
        });
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
            .find(|item| item.read(cx).page == page)
            .or_else(|| {
                self.cached_pages
                    .iter()
                    .find(|item| item.read(cx).page == page)
                    .cloned()
            });
        let item = if let Some(item) = existing {
            if workspace.pane_for(&item).is_some() {
                workspace.activate_item(&item, true, true, window, cx);
            } else {
                workspace.add_item_to_active_pane(Box::new(item.clone()), None, true, window, cx);
            }
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

    fn restore(&mut self, workspace: &mut Workspace, window: &mut Window, cx: &mut App) {
        workspace.set_knightcode_front_page(false, cx);
        // This also runs when an editor is activated outside the mode switch.
        // Do not leave the full-page Chat layout inside Build's agent dock.
        if let Some(conversation) = workspace
            .panel::<AgentPanel>(cx)
            .and_then(|panel| panel.read(cx).active_conversation_view().cloned())
        {
            conversation.update(cx, |conversation, cx| {
                conversation.set_full_page_chat(false, window, cx);
            });
        }
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
                pane.set_should_display_tab_bar_with_pane(|pane, _, _| {
                    pane.active_item().is_some_and(|item| {
                        item.downcast::<super::build_welcome::BuildWelcome>()
                            .is_none()
                    })
                });
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
        if self.build_docks.is_some() {
            self.restore(workspace, window, cx);
        }
        self.build_ready(workspace, window, cx);
    }

    fn enable_workbench(&mut self, _workspace: &Workspace, window: &mut Window, cx: &mut App) {
        if self.previous_build_theme.is_none()
            && let Ok(theme) = theme::ThemeRegistry::global(cx).get("KnightCode Workbench")
        {
            self.previous_build_theme = Some(theme::GlobalTheme::theme(cx).clone());
            self.build_theme_selection =
                Some(theme_settings::ThemeSettings::get_global(cx).theme.clone());
            let theme = theme_settings::ThemeSettings::get_global(cx).apply_theme_overrides(theme);
            theme::GlobalTheme::update_theme(cx, theme);
            window.refresh();
        }
        let workspace_handle = self.workspace.clone();
        let modes = cx
            .global::<WorkspaceModesRegistry>()
            .0
            .get(&self.workspace.entity_id())
            .cloned();
        window.defer(cx, move |window, cx| {
            workspace_handle
                .update(cx, |workspace, cx| {
                    // Dock restoration is queued first. A newer Home/Chat navigation
                    // must win over this deferred Build request.
                    if workspace.active_item_as::<KnightCodePage>(cx).is_none() {
                        let first_layout = modes
                            .as_ref()
                            .and_then(|modes| modes.upgrade())
                            .is_none_or(|modes| !modes.read(cx).build_layout_initialized);
                        let pages = workspace
                            .items_of_type::<KnightCodePage>(cx)
                            .collect::<Vec<_>>();
                        if let Some(modes) = modes.as_ref() {
                            modes
                                .update(cx, |modes, _| {
                                    for page in &pages {
                                        if !modes
                                            .cached_pages
                                            .iter()
                                            .any(|cached| cached.entity_id() == page.entity_id())
                                        {
                                            modes.cached_pages.push(page.clone());
                                        }
                                    }
                                })
                                .log_err();
                        }
                        // Keep front-page state/drafts alive, but never expose
                        // Home, Chat, Models or Account as ordinary editor tabs.
                        for pane in workspace.panes() {
                            pane.update(cx, |pane, cx| {
                                for page in &pages {
                                    pane.remove_item(page.entity_id(), false, false, window, cx);
                                }
                            });
                        }
                        super::build_layout::enable(workspace, first_layout, window, cx);
                        if let Some(modes) = modes.as_ref() {
                            modes
                                .update(cx, |modes, cx| {
                                    let mask = super::build_layout::panel_mask(workspace, cx);
                                    modes.build_layout_initialized = mask == 7;
                                    modes.last_build_panel_mask = mask;
                                })
                                .log_err();
                        }
                    }
                })
                .log_err();
        });
    }

    fn build_ready(&mut self, workspace: &mut Workspace, window: &mut Window, cx: &mut App) {
        set_title_bar_mode(workspace, KnightCodeMode::Build, cx);
        self.enable_workbench(workspace, window, cx);
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
            let welcome =
                cx.new(|cx| super::build_welcome::BuildWelcome::new(self.workspace.clone(), cx));
            workspace.add_item_to_active_pane(Box::new(welcome), None, true, window, cx);
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
            if !workspace.is_knightcode_build() {
                self.enable_workbench(workspace, window, cx);
            }
        }
    }
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
            // The engine's fallback cwd is the home directory, but scratch
            // conversations are persisted with no project worktree paths.
            let work_dirs = project.worktree_paths(cx).folder_path_list().clone();
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
            .when(page == Page::Chat, |navigation| {
                navigation
                    .w(px(260.))
                    .p_4()
                    .gap_3()
                    .bg(rgb(0x090d17))
                    .border_color(rgb(0x202130))
            })
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
            .when(page != Page::Chat, |navigation| {
                navigation.child(
                    v_flex()
                        .p_3()
                        .gap_1()
                        .child(
                            Label::new("Ideas into Impact.")
                                .color(Color::Custom(rgb(0xb19bff).into())),
                        )
                        .child(div().h(px(2.)).w_8().bg(rgb(0x8b7cff))),
                )
            })
    }

    fn render_history(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search.read(cx).text(cx).to_lowercase();
        let workspace = self.workspace.upgrade();
        let project = workspace
            .as_ref()
            .map(|workspace| workspace.read(cx).project().clone());
        let mut threads = project
            .as_ref()
            .and_then(|project| {
                let store = ThreadMetadataStore::try_global(cx)?;
                Some(
                    store
                        .read(cx)
                        .entries_for_chat(project.read(cx), cx)
                        .filter(|metadata| {
                            metadata.display_title().to_lowercase().contains(&query)
                                || (metadata.is_draft() && "new chat".contains(&query))
                        })
                        .cloned()
                        .collect::<Vec<_>>(),
                )
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
                2..=6 => "This Week",
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
            let title = if metadata.is_draft() {
                "New Chat".into()
            } else {
                metadata.display_title()
            };
            entries.push(
                div()
                    .w_full()
                    .rounded_lg()
                    .overflow_hidden()
                    .when(selected, |row| row.bg(rgba(0x6b4bc630)))
                    .child(
                        ButtonLike::new(metadata.thread_id.to_key_string())
                            .full_width()
                            .height(px(40.).into())
                            .style(ButtonStyle::Transparent)
                            .aria_label(title.clone())
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .px_2()
                                    .gap_3()
                                    .child(
                                        Icon::new(IconName::Chat)
                                            .size(IconSize::Medium)
                                            .color(Color::Custom(rgb(0xb8b5d3).into())),
                                    )
                                    .child(div().flex_1().min_w_0().text_left().child(
                                        Label::new(title).truncate().color(Color::Custom(
                                            rgb(if selected { 0xd5c8ff } else { 0xc5c0dc }).into(),
                                        )),
                                    )),
                            )
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
                            })),
                    )
                    .into_any_element(),
            );
        }
        v_flex()
            .gap_3()
            .min_h_0()
            .flex_1()
            .child(
                div()
                    .w_full()
                    .rounded_lg()
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(0x493090))
                    .bg(linear_gradient(
                        110.,
                        linear_color_stop(rgb(0x302074), 0.),
                        linear_color_stop(rgb(0x211659), 1.),
                    ))
                    .child(
                        ButtonLike::new("new-chat")
                            .aria_label("New Chat")
                            .full_width()
                            .height(px(56.).into())
                            .style(ButtonStyle::Transparent)
                            .child(
                                h_flex()
                                    .w_full()
                                    .px_3()
                                    .gap_3()
                                    .child(
                                        Icon::new(IconName::Plus)
                                            .size(IconSize::Medium)
                                            .color(Color::Custom(rgb(0xe6e0ff).into())),
                                    )
                                    .child(
                                        Label::new("New Chat")
                                            .color(Color::Custom(rgb(0xf5f1ff).into())),
                                    ),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.new_chat(window, cx))),
                    ),
            )
            .child(
                h_flex()
                    .h(px(48.))
                    .px_3()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(0x202432))
                    .bg(rgb(0x10141e))
                    .child(
                        Icon::new(IconName::MagnifyingGlass)
                            .color(Color::Custom(rgb(0xb8b5d3).into())),
                    )
                    .child(
                        div().min_w_0().flex_1().child(editor::EditorElement::new(
                            &self.search,
                            editor::EditorStyle {
                                background: rgb(0x10141e).into(),
                                local_player: cx.theme().players().local(),
                                text: gpui::TextStyle {
                                    color: rgb(0xddd8ee).into(),
                                    font_family: theme_settings::ThemeSettings::get_global(cx)
                                        .agent_ui_font_family()
                                        .clone(),
                                    font_size: theme_settings::ThemeSettings::get_global(cx)
                                        .agent_ui_font_size(cx)
                                        .into(),
                                    ..Default::default()
                                },
                                syntax: cx.theme().syntax().clone(),
                                ..Default::default()
                            },
                        )),
                    ),
            )
            .child(
                v_flex()
                    .id("chat-history")
                    .min_h_0()
                    .flex_1()
                    .gap_2()
                    .overflow_y_scroll()
                    .when(entries.is_empty(), |history| {
                        history.child(
                            Label::new(if query.is_empty() {
                                "Your conversations will appear here"
                            } else {
                                "No matching conversations"
                            })
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                        )
                    })
                    .children(entries),
            )
    }

    fn render_chat(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        if let Some(conversation) = &conversation {
            conversation.update(cx, |conversation, cx| {
                conversation.set_full_page_chat(true, window, cx);
            });
        }
        let show_loading = is_active && !panel_visible && conversation.is_none();
        let theme_settings = theme_settings::ThemeSettings::get_global(cx);
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(rgb(0x070a13))
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
            Page::Home => home_presentation::render(self, window, cx),
            Page::Chat => self.render_chat(window, cx).into_any_element(),
            Page::Models | Page::Account => {
                self.render_configuration(window, cx).into_any_element()
            }
        };
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("KnightCodePage");
        if self.page == Page::Home {
            key_context.add("KnightCodeHome");
        }
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
            .when(matches!(self.page, Page::Models | Page::Account), |page| {
                page.child(scenic_background())
            })
            .when(self.page != Page::Home, |page| {
                page.child(self.render_navigation(cx))
            })
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
