use agent_ui::{AgentPanel, AgentThreadSource};
use gpui::{Action, App, Context, Window, actions};
use project_panel::ProjectPanel;
use terminal_view::terminal_panel::TerminalPanel;
use ui::prelude::*;
use workspace::{
    WorkbenchActivity, Workspace,
    dock::{DockPosition, Panel, PanelSizeState},
};

use super::build_welcome::BuildWelcome;

actions!(knightcode, [OpenBuildTimeline]);

pub(super) fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|workspace, _: &OpenBuildTimeline, window, cx| {
            if workspace
                .focus_panel::<git_ui::git_panel::GitPanel>(window, cx)
                .is_some()
            {
                window.dispatch_action(git_ui::git_panel::ActivateHistoryTab.boxed_clone(), cx);
            }
        });
    })
    .detach();
}

fn activities() -> Vec<WorkbenchActivity> {
    [
        (
            "explorer",
            "Explorer",
            IconName::FileMultiple,
            zed_actions::project_panel::ToggleFocus.boxed_clone(),
        ),
        (
            "search",
            "Search",
            IconName::MagnifyingGlass,
            search::project_search::ToggleFocus.boxed_clone(),
        ),
        (
            "source-control",
            "Source Control",
            IconName::GitBranch,
            zed_actions::git_panel::ToggleFocus.boxed_clone(),
        ),
        (
            "terminal",
            "Terminal",
            IconName::Terminal,
            terminal_view::terminal_panel::ToggleFocus.boxed_clone(),
        ),
        (
            "tasks",
            "Run Tasks",
            IconName::PlayOutlined,
            zed_actions::Spawn::modal().boxed_clone(),
        ),
        (
            "extensions",
            "Extensions",
            IconName::Blocks,
            zed_actions::Extensions::default().boxed_clone(),
        ),
        (
            "outline",
            "Outline",
            IconName::ListTree,
            outline_panel::ToggleFocus.boxed_clone(),
        ),
        (
            "timeline",
            "Timeline",
            IconName::Clock,
            OpenBuildTimeline.boxed_clone(),
        ),
    ]
    .into_iter()
    .map(|(id, label, icon, action)| WorkbenchActivity {
        id: id.into(),
        label: label.into(),
        icon,
        action,
    })
    .collect()
}

pub(super) fn panel_mask(workspace: &Workspace, cx: &App) -> u8 {
    u8::from(workspace.panel::<ProjectPanel>(cx).is_some())
        | (u8::from(workspace.panel::<TerminalPanel>(cx).is_some()) << 1)
        | (u8::from(workspace.panel::<AgentPanel>(cx).is_some()) << 2)
}

pub(super) fn enable(
    workspace: &mut Workspace,
    first_layout: bool,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    workspace.set_knightcode_workbench(activities(), cx);
    let pane = workspace.active_pane().clone();
    pane.update(cx, |pane, cx| {
        // Only the empty Build welcome surface is tabless. Real files retain
        // standard Zed tabs, splits, previews and navigation.
        pane.set_should_display_tab_bar_with_pane(|pane, _, _| {
            pane.active_item()
                .is_some_and(|item| item.downcast::<BuildWelcome>().is_none())
        });
        cx.notify();
    });
    if first_layout {
        if let Some(panel) = workspace.panel::<ProjectPanel>(cx) {
            panel.update(cx, |panel, cx| {
                if panel.position(window, cx) != DockPosition::Left {
                    panel.set_position(DockPosition::Left, window, cx);
                }
            });
            workspace.left_dock().update(cx, |dock, cx| {
                dock.set_panel_size_state(
                    &panel,
                    PanelSizeState {
                        size: Some(px(244.)),
                        flex: Some(0.16),
                    },
                    cx,
                );
            });
            workspace.reveal_panel::<ProjectPanel>(window, cx);
        }
        if let Some(panel) = workspace.panel::<TerminalPanel>(cx) {
            panel.update(cx, |panel, cx| {
                if panel.position(window, cx) != DockPosition::Bottom {
                    panel.set_position(DockPosition::Bottom, window, cx);
                }
            });
            workspace.bottom_dock().update(cx, |dock, cx| {
                dock.set_panel_size_state(
                    &panel,
                    PanelSizeState {
                        size: Some(px(
                            (f32::from(window.viewport_size().height) * 0.22).clamp(160., 260.)
                        )),
                        flex: None,
                    },
                    cx,
                );
            });
            workspace.reveal_panel::<TerminalPanel>(window, cx);
        }
        if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
            panel.update(cx, |panel, cx| {
                if panel.position(window, cx) != DockPosition::Right {
                    panel.set_position(DockPosition::Right, window, cx);
                }
            });
            workspace.right_dock().update(cx, |dock, cx| {
                dock.set_panel_size_state(
                    &panel,
                    PanelSizeState {
                        size: Some(px(
                            (f32::from(window.viewport_size().width) * 0.30).clamp(350., 480.)
                        )),
                        flex: Some(0.30),
                    },
                    cx,
                );
            });
        }
    }
    if let Some(panel) = workspace.panel::<ProjectPanel>(cx) {
        panel.update(cx, |_, cx| cx.notify());
    }
    if let Some(panel) = workspace.panel::<TerminalPanel>(cx) {
        panel.update(cx, |_, cx| cx.notify());
    }
    if workspace
        .project()
        .read(cx)
        .visible_worktrees(cx)
        .next()
        .is_some()
    {
        TerminalPanel::ensure_default_terminal(workspace, window, cx);
    }
    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
        panel.update(cx, |panel, cx| {
            panel.set_build_presentation(true, window, cx);
            if panel.active_conversation_view().is_none() {
                panel.activate_new_thread(false, AgentThreadSource::AgentPanel, window, cx);
            }
        });
        workspace.reveal_panel::<AgentPanel>(window, cx);
    }
}
