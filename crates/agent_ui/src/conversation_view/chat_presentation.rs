//! Artwork and layout for the native, full-page Chat surface.
//! Conversation state, permissions and the composer remain in ThreadView.
use std::sync::Arc;

use gpui::{Image, ImageFormat, ObjectFit, StyledImage, Window, img, rgb};
use ui::prelude::*;

pub(super) struct ChatArtwork {
    landscape: Arc<Image>,
    knight: Arc<Image>,
}

impl ChatArtwork {
    pub(super) fn new() -> Self {
        Self {
            landscape: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../assets/chat-landscape.png").to_vec(),
            )),
            knight: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../assets/chat-knight.png").to_vec(),
            )),
        }
    }

    pub(super) fn background(&self) -> impl IntoElement {
        img(self.landscape.clone())
            .absolute()
            .size_full()
            .object_fit(ObjectFit::Cover)
    }

    pub(super) fn welcome(&self, name: Option<String>, window: &Window) -> impl IntoElement {
        // Keep the bottom composer visible on shorter windows too.
        let height = f32::from(window.viewport_size().height);
        let hero_height = (height * 0.31).clamp(120., 309.);
        let headline_size = if height < 700. { 40. } else { 64. };
        v_flex()
            .relative()
            .flex_1()
            .size_full()
            .min_h_0()
            .px_6()
            .gap_3()
            .items_center()
            .justify_center()
            .child(
                img(self.knight.clone())
                    .w(px(hero_height * 420. / 309.))
                    .h(px(hero_height))
                    .flex_none(),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(rgb(0xa394ed))
                    .child("W E L C O M E  B A C K"),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_center()
                    .gap_3()
                    .text_size(px(headline_size))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(0xf5f4ff))
                    .child(if name.is_some() {
                        "Hello,"
                    } else {
                        "Hello there"
                    })
                    .when_some(name, |greeting, name| {
                        greeting.child(div().text_color(rgb(0xa17bff)).child(name))
                    }),
            )
            .child(
                div()
                    .text_size(px(if height < 700. { 18. } else { 26. }))
                    .text_color(rgb(0xb3b1dd))
                    .child("What shall we explore today?"),
            )
    }
}
