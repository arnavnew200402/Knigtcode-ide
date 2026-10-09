//! Native blue Home surface. Only the scenery is raster artwork; every
//! heading, action, recent-project name, path and timestamp is live GPUI UI.
use std::sync::{Arc, OnceLock};

use gpui::{
    Action, ClipboardItem, Context, Image, ImageFormat, ObjectFit, StyledImage, Window, img, rgb,
    rgba,
};
use ui::{ButtonLike, ContextMenu, PopoverMenu, Tooltip, prelude::*};
use workspace::{RecentWorkspace, SerializedWorkspaceLocation};

use super::KnightCodePage;

struct HomeArtwork {
    landscape: Arc<Image>,
    texture: Arc<Image>,
}
impl HomeArtwork {
    fn get() -> &'static Self {
        static ART: OnceLock<HomeArtwork> = OnceLock::new();
        ART.get_or_init(|| Self {
            landscape: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../assets/home-landscape.png").to_vec(),
            )),
            texture: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../assets/home-card-texture.png").to_vec(),
            )),
        })
    }
}

fn icon_tile(icon: IconName, size: f32) -> impl IntoElement {
    h_flex()
        .size(px(size))
        .flex_none()
        .justify_center()
        .rounded_lg()
        .border_1()
        .border_color(rgba(0x74baff80))
        .bg(gpui::linear_gradient(
            135.,
            gpui::linear_color_stop(rgb(0x285b9c), 0.),
            gpui::linear_color_stop(rgb(0x070f20), 1.),
        ))
        .child(
            Icon::new(icon)
                .size(IconSize::Custom(rems(2.)))
                .color(Color::Custom(rgb(0xdcf4ff).into())),
        )
}

fn action_card(
    id: &'static str,
    title: &'static str,
    description: &'static str,
    caption: &'static str,
    icon: IconName,
    action: Box<dyn Action>,
) -> impl IntoElement {
    let header_action = action.boxed_clone();
    v_flex()
        .relative()
        .flex_1()
        .min_w(px(300.))
        .p_5()
        .gap_4()
        .rounded_xl()
        .border_1()
        .border_color(rgba(0x64c4ff99))
        .bg(rgba(0x061327df))
        .overflow_hidden()
        .child(
            img(HomeArtwork::get().texture.clone())
                .absolute()
                .size_full()
                .object_fit(ObjectFit::Cover)
                .opacity(0.16),
        )
        .child(
            ButtonLike::new(format!("{id}-heading"))
                .full_width()
                .height(px(72.).into())
                .style(ButtonStyle::Transparent)
                .aria_label(title)
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_5()
                        .child(icon_tile(icon, 72.))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(px(22.))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                                .child(
                                    Label::new(description)
                                        .size(LabelSize::Small)
                                        .color(Color::Custom(rgb(0xc7e3f7).into())),
                                ),
                        )
                        .child(
                            Icon::new(IconName::ChevronRight)
                                .color(Color::Custom(rgb(0xe1ecff).into())),
                        ),
                )
                .on_click(move |_, window, cx| {
                    window.dispatch_action(header_action.boxed_clone(), cx)
                }),
        )
        .child(
            div()
                .relative()
                .w_full()
                .rounded_lg()
                .overflow_hidden()
                .border_1()
                .border_color(rgb(0x74caff))
                .bg(gpui::linear_gradient(
                    110.,
                    gpui::linear_color_stop(rgb(0x42c8ff), 0.),
                    gpui::linear_color_stop(rgb(0x153769), 0.5),
                ))
                .child(
                    img(HomeArtwork::get().texture.clone())
                        .absolute()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .opacity(0.32),
                )
                .child(
                    ButtonLike::new(id)
                        .full_width()
                        .height(px(50.).into())
                        .style(ButtonStyle::Transparent)
                        .aria_label(caption)
                        .child(
                            h_flex()
                                .w_full()
                                .justify_center()
                                .gap_3()
                                .child(
                                    Label::new(caption).color(Color::Custom(rgb(0xf2f8ff).into())),
                                )
                                .child(
                                    Icon::new(IconName::ArrowRight)
                                        .color(Color::Custom(rgb(0xf2f8ff).into())),
                                ),
                        )
                        .on_click(move |_, window, cx| {
                            window.dispatch_action(action.boxed_clone(), cx)
                        }),
                ),
        )
}

fn recent_card(recent: &RecentWorkspace, cx: &mut Context<KnightCodePage>) -> impl IntoElement {
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
        .with_timezone(&chrono::Local)
        .format("%d %b %Y, %H:%M")
        .to_string();
    let icon = match recent.location {
        SerializedWorkspaceLocation::Local => IconName::Folder,
        SerializedWorkspaceLocation::Remote(_) => IconName::Server,
    };
    let open = recent.clone();
    let menu_recent = recent.clone();
    let weak = cx.weak_entity();
    let copy_paths = paths.clone();
    // The entire card opens a real project; the ellipsis has a separate native
    // menu rather than a decorative/unconnected screenshot control.
    div()
        .relative()
        .flex_1()
        .min_w(px(260.))
        .rounded_lg()
        .border_1()
        .border_color(rgba(0x4299e066))
        .overflow_hidden()
        .bg(rgb(0x081426))
        .child(
            img(HomeArtwork::get().texture.clone())
                .absolute()
                .size_full()
                .object_fit(ObjectFit::Cover)
                .opacity(0.48),
        )
        .child(div().absolute().size_full().bg(gpui::linear_gradient(
            0.,
            gpui::linear_color_stop(rgba(0x060d1cef), 0.),
            gpui::linear_color_stop(rgba(0x08142633), 1.),
        )))
        .child(
            ButtonLike::new(format!("home-recent-{}", i64::from(recent.workspace_id)))
                .full_width()
                .height(px(112.).into())
                .style(ButtonStyle::Transparent)
                .aria_label(format!("Open {name}"))
                .child(
                    h_flex()
                        .relative()
                        .w_full()
                        .min_w_0()
                        .p_4()
                        .gap_4()
                        .items_start()
                        .child(icon_tile(icon, 64.))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .pr_5()
                                .child(
                                    Label::new(name)
                                        .truncate()
                                        .color(Color::Custom(rgb(0xf1f5ff).into())),
                                )
                                .child(
                                    Label::new(paths)
                                        .size(LabelSize::Small)
                                        .truncate()
                                        .color(Color::Custom(rgb(0xc6dff5).into())),
                                )
                                .child(
                                    Label::new(format!("Last opened · {timestamp}"))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Custom(rgb(0xc6dff5).into())),
                                ),
                        ),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_recent(open.clone(), window, cx)
                })),
        )
        .child(
            div().absolute().top_2().right_2().child(
                PopoverMenu::new(format!(
                    "home-recent-menu-{}",
                    i64::from(recent.workspace_id)
                ))
                .trigger_with_tooltip(
                    IconButton::new("recent-project-options", IconName::Ellipsis)
                        .icon_color(Color::Custom(rgb(0xd0eaff).into())),
                    Tooltip::text("Project options"),
                )
                .menu(move |window, cx| {
                    Some(ContextMenu::build(window, cx, |menu, _, _| {
                        let weak = weak.clone();
                        let recent = menu_recent.clone();
                        let paths = copy_paths.clone();
                        menu.entry("Open project", None, move |window, cx| {
                            weak.update(cx, |this, cx| {
                                this.open_recent(recent.clone(), window, cx)
                            })
                            .ok();
                        })
                        .entry("Copy project path", None, move |_, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(paths.clone()))
                        })
                    }))
                }),
            ),
        )
}

pub(super) fn render(
    page: &KnightCodePage,
    window: &Window,
    cx: &mut Context<KnightCodePage>,
) -> AnyElement {
    let compact = f32::from(window.viewport_size().width) < 1200.
        || f32::from(window.viewport_size().height) < 760.;
    let hero_height = if compact { 330. } else { 438. };
    let heading_size = if compact { 44. } else { 64. };
    let recent_cards = page
        .recent
        .as_ref()
        .into_iter()
        .flatten()
        .take(3)
        .map(|recent| recent_card(recent, cx).into_any_element())
        .collect::<Vec<_>>();
    v_flex().id("home-content").size_full().relative().overflow_hidden().bg(rgb(0x030913)).text_color(rgb(0xf6f8ff))
        .child(img(HomeArtwork::get().landscape.clone()).absolute().size_full().object_fit(ObjectFit::Fill))
        .child(v_flex().id("home-scroll").relative().size_full().overflow_y_scroll()
            .pl(relative(if compact { 0.05 } else { 0.094 })).pr(relative(if compact { 0.05 } else { 0.068 })).pb_8()
            .child(v_flex().h(px(hero_height)).pt(px(if compact { 24. } else { 72. })).flex_none().justify_center().gap_5().max_w(px(700.))
                .child(div().text_size(px(13.)).text_color(rgb(0x62d8ff)).child("T H I N K  •  B U I L D  •  B E Y O N D"))
                .child(v_flex().text_size(px(heading_size)).line_height(relative(1.10)).font_weight(gpui::FontWeight::BOLD)
                    .child("Turn your ideas")
                    .child(h_flex().flex_wrap().gap_3().child("into")
                        .child(div().text_color(rgb(0x91c9fa)).child("real"))
                        .child(div().text_color(rgb(0x949cff)).child("impact."))))
                .child(v_flex().gap_1().text_size(px(if compact { 17. } else { 20. })).text_color(rgb(0xbfe5fa))
                    .child("Chat with AI, or open a project and start building.")
                    .child("Same intelligence. More possibilities.")))
            .child(h_flex().w_full().gap_5().flex_wrap()
                .child(action_card("home-start-chat", "Chat", "Ask, explore, brainstorm and get instant answers.", "Start chatting", IconName::Chat, title_bar::ShowChat.boxed_clone()))
                .child(action_card("home-open-project", "Build", "Open a project, write code, run commands and build with AI.", "Open project", IconName::Code, workspace::Open::default().boxed_clone())))
            .child(h_flex().w_full().mt_6().mb_3().justify_between()
                .child(h_flex().gap_3().child(Icon::new(IconName::Clock).color(Color::Custom(rgb(0xb8eaff).into())))
                    .child(Label::new("Recent projects").color(Color::Custom(rgb(0xf3f7ff).into()))))
                .child(Button::new("home-all-projects", "View all").end_icon(Icon::new(IconName::ArrowRight)).color(Color::Custom(rgb(0xc8eaff).into()))
                    .on_click(|_, window, cx| window.dispatch_action(zed_actions::OpenRecent::default().boxed_clone(), cx))))
            .when_some(page.recent_error.clone(), |view, error| view.child(Label::new(error).color(Color::Error)))
            .child(h_flex().w_full().gap_3().flex_wrap()
                .when(page.recent.is_none() && page.recent_error.is_none(), |view| view.child(Label::new("Loading recent projects…").color(Color::Custom(rgb(0xbfe5fa).into()))))
                .when(page.recent.as_ref().is_some_and(Vec::is_empty), |view| view.child(Label::new("Your recent projects will appear here after you open a folder.").color(Color::Custom(rgb(0xbfe5fa).into()))))
                .children(recent_cards)))
        .into_any_element()
}
