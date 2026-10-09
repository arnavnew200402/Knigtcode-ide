//! Build's native empty editor surface. Files, terminals and the agent remain
//! ordinary workspace items and panels; this is not a second Home workspace.
use std::sync::Arc;

use agent_ui::{AgentPanel, AgentThreadSource};
use gpui::{
    Action, App, Context, EventEmitter, FocusHandle, Focusable, Image, ImageFormat, ObjectFit,
    Render, StyledImage, WeakEntity, Window, img, rgb, rgba,
};
use ui::{ButtonLike, KeyBinding, Tooltip, prelude::*};
use util::ResultExt as _;
use workspace::{Item, Workspace, item::ItemEvent};
use zed_actions::assistant::ToggleFocus;

pub(super) struct BuildWelcome {
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    knight: Arc<Image>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl BuildWelcome {
    pub(super) fn new(workspace: WeakEntity<Workspace>, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        if let Some(workspace) = workspace.upgrade() {
            subscriptions.push(cx.observe(&workspace, |_, _, cx| cx.notify()));
            if let Some(panel) = workspace.read(cx).panel::<AgentPanel>(cx) {
                subscriptions.push(cx.observe(&panel, |_, _, cx| cx.notify()));
            }
        }
        Self {
            workspace,
            focus_handle: cx.focus_handle(),
            knight: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../assets/build-knight.png").to_vec(),
            )),
            _subscriptions: subscriptions,
        }
    }

    fn agent_action(&self, action: Option<Box<dyn Action>>, window: &mut Window, cx: &mut App) {
        self.workspace
            .update(cx, |workspace, cx| {
                if let Some(panel) = workspace.focus_panel::<AgentPanel>(window, cx) {
                    panel.update(cx, |panel, cx| {
                        if panel.active_conversation_view().is_none() {
                            panel.activate_new_thread(
                                false,
                                AgentThreadSource::AgentPanel,
                                window,
                                cx,
                            );
                        }
                    });
                    if let Some(action) = action {
                        window.dispatch_action(action, cx);
                    }
                }
            })
            .log_err();
    }
}

impl Render for BuildWelcome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_project = self.workspace.upgrade().is_some_and(|workspace| {
            workspace
                .read(cx)
                .project()
                .read(cx)
                .visible_worktrees(cx)
                .next()
                .is_some()
        });
        let root_thread = self
            .workspace
            .upgrade()
            .and_then(|workspace| workspace.read(cx).panel::<AgentPanel>(cx))
            .and_then(|panel| panel.read(cx).active_conversation_view().cloned())
            .and_then(|conversation| conversation.read(cx).root_thread_view());
        let conversation_ready = root_thread.is_some();
        let supports_tasks =
            root_thread.is_some_and(|view| view.read(cx).supports_task_controls(cx));
        let size = if f32::from(window.viewport_size().width) < 1300. {
            40.
        } else {
            56.
        };
        v_flex().id("knightcode-build-welcome").key_context("KnightCodeBuildWelcome")
            .track_focus(&self.focus_handle).size_full().min_h_0().relative().overflow_hidden()
            .bg(rgb(0x090806)).text_color(rgb(0xf3eee8))
            .child(img(self.knight.clone()).absolute().right_0().top_0().w(px(254.)).max_w(relative(0.45)).h(px(568.)).object_fit(ObjectFit::Contain))
            .child(
                v_flex().relative().flex_1().min_h_0().justify_center().px_8().gap_5()
                    .child(div().text_size(px(13.)).text_color(rgb(0xff9a26)).child("K N I G H T C O D E"))
                    .child(v_flex().max_w(relative(0.78)).gap_1().text_size(px(size)).font_weight(gpui::FontWeight::BOLD)
                        .child(h_flex().flex_wrap().gap_2()
                            .child(div().text_color(rgb(0xff8139)).child("Agentic"))
                            .child("coding,"))
                        .child("close to your repo."))
                    .child(div().max_w(px(460.)).text_size(px(17.)).text_color(rgb(0xd8d2c9))
                        .child(if has_project {
                            "A local coding agent for your terminal. It reads, searches, edits, and runs commands in the folder you've opened, through the provider account you choose."
                        } else {
                            "Open a project folder to explore your files, run a real terminal, and build with KnightCode through the provider account you choose."
                        }))
                    .child(v_flex().w(px(348.)).max_w_full().gap_2()
                        .child(div().rounded_lg().overflow_hidden().border_1().border_color(rgb(0xc97016))
                            .bg(gpui::linear_gradient(130., gpui::linear_color_stop(rgb(0xffa544), 0.), gpui::linear_color_stop(rgb(0x85370d), 1.)))
                            .child(ButtonLike::new("build-code-with-agent").full_width().height(px(52.).into())
                                .style(ButtonStyle::Transparent).disabled(!has_project).aria_label("Code with Agent")
                                .child(h_flex().w_full().px_4().gap_3()
                                    .child(Icon::new(IconName::PlayFilled).color(Color::Custom(rgb(0xffe4b9).into())))
                                    .child(Label::new("Code with Agent").color(Color::Custom(rgb(0xfff4e9).into())))
                                    .child(div().flex_1())
                                    .child(KeyBinding::for_action(&ToggleFocus, cx)))
                                .on_click(cx.listener(|this, _, window, cx| this.agent_action(None, window, cx)))))
                        .child(div().rounded_lg().overflow_hidden().border_1().border_color(rgba(0xa762243f)).bg(rgb(0x14100b))
                            .child(ButtonLike::new("build-open-file").full_width().height(px(52.).into())
                                .style(ButtonStyle::Transparent).aria_label(if has_project { "Open a file" } else { "Open a project" })
                                .child(h_flex().w_full().px_4().gap_3()
                                    .child(Icon::new(IconName::FolderOpen).color(Color::Custom(rgb(0xe8c99e).into())))
                                    .child(Label::new(if has_project { "Open a file" } else { "Open a project" }).color(Color::Custom(rgb(0xf0e8df).into())))
                                    .child(div().flex_1())
                                    .when(has_project, |row| row.child(KeyBinding::for_action(&workspace::ToggleFileFinder::default(), cx))))
                                .on_click(move |_, window, cx| {
                                    if has_project { window.dispatch_action(workspace::ToggleFileFinder::default().boxed_clone(), cx) }
                                    else { window.dispatch_action(workspace::Open::default().boxed_clone(), cx) }
                                }))))
            )
            .child(
                h_flex().relative().w_full().flex_none().px_6().py_5().gap_3().flex_wrap()
                    .children([
                        ("build-context", "Smarter context", IconName::FileMultiple, Some(agent_ui::OpenAddContextMenu.boxed_clone())),
                        ("build-subagents", "Parallel subagents", IconName::BoltOutlined, Some(agent_ui::OpenTaskPanel.boxed_clone())),
                        ("build-models", "Flexible models", IconName::Box, Some(zed_actions::agent::ToggleModelSelector.boxed_clone())),
                        ("build-terminal", "Terminal integrated", IconName::Terminal, None),
                    ].into_iter().map(|(id, label, icon, action)| {
                        ButtonLike::new(id).height(px(56.).into()).style(ButtonStyle::Transparent).aria_label(label)
                            .child(h_flex().gap_2()
                                .child(h_flex().size(px(44.)).justify_center().rounded_full().border_1().border_color(rgb(0x7a3d0e))
                                    .child(Icon::new(icon).size(IconSize::Medium).color(Color::Custom(rgb(0xff9b30).into()))))
                                .child(Label::new(label).size(LabelSize::Small).color(Color::Custom(rgb(0xeee8df).into()))))
                            .disabled(!has_project || (id != "build-terminal" && !conversation_ready) || (id == "build-subagents" && !supports_tasks))
                            .tooltip(Tooltip::text(if id == "build-subagents" && !supports_tasks { "Subagents are unavailable from the current connection" } else { label }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(action) = &action { this.agent_action(Some(action.boxed_clone()), window, cx) }
                                else { window.dispatch_action(terminal_view::terminal_panel::ToggleFocus.boxed_clone(), cx) }
                            }))
                    }))
            )
    }
}

impl EventEmitter<ItemEvent> for BuildWelcome {}
impl Focusable for BuildWelcome {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
impl Item for BuildWelcome {
    type Event = ItemEvent;
    fn tab_content_text(&self, _: usize, _: &App) -> SharedString {
        "KnightCode".into()
    }
    fn show_toolbar(&self) -> bool {
        false
    }
    fn include_in_nav_history() -> bool {
        false
    }
}
