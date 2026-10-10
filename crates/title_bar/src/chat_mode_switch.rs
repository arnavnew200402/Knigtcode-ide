use super::{KnightCodeMode, ShowBuild, ShowChat};
use gpui::{Action, Div, MouseButton};
use ui::{ButtonLike, prelude::*};

/// Navigation is independent of agent loading/account operations. Occlusion
/// keeps these controls out of the native Windows caption hit-test region.
pub(super) fn render(mode: KnightCodeMode) -> Div {
    h_flex()
        .w(px(224.))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .p_1()
        .gap_1()
        .rounded_full()
        .border_1()
        .border_color(gpui::rgba(0x8c7fc926))
        .bg(gpui::rgb(0x080b14))
        .children(
            [
                (
                    "full-page-chat",
                    "Chat",
                    IconName::Chat,
                    KnightCodeMode::Chat,
                    ShowChat.boxed_clone(),
                ),
                (
                    "full-page-build",
                    "Build",
                    IconName::Code,
                    KnightCodeMode::Build,
                    ShowBuild.boxed_clone(),
                ),
            ]
            .into_iter()
            .map(|(id, label, icon, target, action)| {
                div()
                    .flex_1()
                    .rounded_full()
                    .overflow_hidden()
                    .debug_selector(move || id.to_owned())
                    .when(mode == target, |tab| {
                        tab.bg(gpui::linear_gradient(
                            120.,
                            gpui::linear_color_stop(gpui::rgb(0x6540d7), 0.),
                            gpui::linear_color_stop(gpui::rgb(0x40258c), 1.),
                        ))
                    })
                    .child(
                        ButtonLike::new(id)
                            .full_width()
                            .height(px(32.).into())
                            .style(ButtonStyle::Transparent)
                            .child(
                                h_flex()
                                    .gap_2()
                                    .justify_center()
                                    .child(
                                        Icon::new(icon)
                                            .color(Color::Custom(gpui::rgb(0xe1dcf5).into())),
                                    )
                                    .child(
                                        Label::new(label)
                                            .color(Color::Custom(gpui::rgb(0xe1dcf5).into())),
                                    ),
                            )
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(action.boxed_clone(), cx)
                            }),
                    )
            }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Context, FocusHandle, Modifiers, Render, TestAppContext, Window, WindowControlArea,
    };

    struct ModeSwitchView {
        focus: FocusHandle,
        mode: KnightCodeMode,
        build_clicks: usize,
        chat_clicks: usize,
        caption_mouse_downs: usize,
    }

    impl Render for ModeSwitchView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            h_flex()
                .w(px(400.))
                .h(px(48.))
                .track_focus(&self.focus)
                .window_control_area(WindowControlArea::Drag)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|view, _, _, _| view.caption_mouse_downs += 1),
                )
                .on_action(cx.listener(|view, _: &ShowBuild, _, cx| {
                    view.mode = KnightCodeMode::Build;
                    view.build_clicks += 1;
                    cx.notify();
                }))
                .on_action(cx.listener(|view, _: &ShowChat, _, cx| {
                    view.mode = KnightCodeMode::Chat;
                    view.chat_clicks += 1;
                    cx.notify();
                }))
                .child(super::render(self.mode))
        }
    }

    #[gpui::test(iterations = 5)]
    fn test_knightcode_mode_switch_clicks_do_not_arm_caption_drag(cx: &mut TestAppContext) {
        let _app_state = cx.update(workspace::AppState::test);
        let (view, cx) = cx.add_window_view(|window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus);
            ModeSwitchView {
                focus,
                mode: KnightCodeMode::Chat,
                build_clicks: 0,
                chat_clicks: 0,
                caption_mouse_downs: 0,
            }
        });
        cx.run_until_parked();
        let build = cx
            .debug_bounds("full-page-build")
            .expect("Build must be visible");
        cx.simulate_mouse_move(build.center(), None, Modifiers::default());
        cx.simulate_click(build.center(), Modifiers::default());
        cx.run_until_parked();
        view.update(cx, |view, _| {
            assert_eq!(view.mode, KnightCodeMode::Build);
            assert_eq!(view.build_clicks, 1);
            assert_eq!(view.caption_mouse_downs, 0);
        });
        let chat = cx
            .debug_bounds("full-page-chat")
            .expect("Chat must remain visible");
        cx.simulate_mouse_move(chat.center(), None, Modifiers::default());
        cx.simulate_click(chat.center(), Modifiers::default());
        cx.run_until_parked();
        view.update(cx, |view, _| {
            assert_eq!(view.mode, KnightCodeMode::Chat);
            assert_eq!(view.chat_clicks, 1);
            assert_eq!(view.caption_mouse_downs, 0);
        });
        // Only controls are excluded; the remaining caption area still receives
        // the mouse-down that initiates normal title-bar window dragging.
        let caption = gpui::point(px(350.), px(24.));
        cx.simulate_mouse_move(caption, None, Modifiers::default());
        cx.simulate_click(caption, Modifiers::default());
        view.update(cx, |view, _| assert_eq!(view.caption_mouse_downs, 1));
    }
}
