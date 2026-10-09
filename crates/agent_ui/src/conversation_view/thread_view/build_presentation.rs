use super::*;
use gpui::{rgb, rgba};

impl ThreadView {
    pub fn supports_task_controls(&self, cx: &App) -> bool {
        self.knightcode_tasks
            .as_ref()
            .is_some_and(|tasks| tasks.read(cx).is_available())
    }

    pub(super) fn render_build_welcome(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("build-agent-welcome")
            .w_full()
            .h_full()
            .min_h_0()
            .justify_center()
            .items_center()
            .px_5()
            .gap_4()
            .overflow_y_scroll()
            .child(img(crate::build_artwork::knight()).w(px(78.)).h(px(99.)))
            .child(
                div()
                    .text_size(px(28.))
                    .text_color(rgb(0xffc482))
                    .text_center()
                    .child("What should we build?"),
            )
            .child(
                div()
                    .text_size(px(17.))
                    .text_color(rgb(0xf0e8df))
                    .text_center()
                    .child("Ask, code, debug, or explain."),
            )
    }

    pub(super) fn render_build_composer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let supports_files = self.session_capabilities.read().supports_embedded_context();
        v_flex().w_full().flex_none().gap_3().px_4().pt_3().pb_3()
            .child(v_flex().w_full().min_w_0().rounded_lg().overflow_hidden().border_1()
                .border_color(rgb(0xb1641d)).bg(rgb(0x141009))
                .child(div().px_4().pt_3().pb_1().min_h(px(74.)).child(self.message_editor.clone()))
                .child(h_flex().w_full().justify_between().px_3().pb_2()
                    .child(h_flex().gap_2()
                        .child(div().w(px(54.)).h(px(52.)).rounded_lg().border_1().border_color(rgba(0x81511c80)).bg(rgb(0x211306))
                            .child(self.render_add_context_button(cx)))
                        .child(div().w(px(54.)).h(px(52.)).rounded_lg().border_1().border_color(rgba(0x81511c80)).bg(rgb(0x211306))
                            .child(ButtonLike::new("build-attach-file").full_width().height(px(52.).into())
                                .style(ButtonStyle::Transparent).disabled(!supports_files).aria_label("Attach a file")
                                .tooltip(Tooltip::text("Attach files from the current project; images are available in Add Context"))
                                .child(Icon::new(IconName::Paperclip).size(IconSize::Medium).color(Color::Custom(rgb(0xffd09b).into())))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.message_editor.update(cx, |editor, cx| editor.insert_context_type("file", window, cx));
                                    this.message_editor.focus_handle(cx).focus(window, cx);
                                }))))
                        .child(div().w(px(54.)).h(px(52.)).rounded_lg().border_1().border_color(rgba(0x81511c80)).bg(rgb(0x211306))
                            .child(self.voice_input.clone())))
                    .child(self.render_connected_send_button(cx))))
            .child(h_flex().w_full().min_w_0().flex_wrap().gap_2()
                .map(|row| match self.config_options_view.clone() {
                    Some(config) => row.child(config),
                    None => row.children(self.model_selector.clone()).children(self.render_thinking_control(cx)),
                }))
            .children(self.knightcode_tasks.clone())
            .into_any_element()
    }
}
