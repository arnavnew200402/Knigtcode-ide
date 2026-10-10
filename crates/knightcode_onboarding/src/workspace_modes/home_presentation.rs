//! Native, viewport-fit Home. One scenic background; all text and controls
//! are live GPUI elements, with no page scroll container or raster UI overlays.
use std::sync::{Arc, OnceLock};

use gpui::{
    Action, ClipboardItem, Context, Image, ImageFormat, ObjectFit, StyledImage, Window, img, rgb,
    rgba,
};
use ui::{ButtonLike, ContextMenu, PopoverMenu, Tooltip, prelude::*};
use workspace::{RecentWorkspace, SerializedWorkspaceLocation};

use super::{KnightCodePage, home_layout::HomeLayout};

struct HomeArtwork {
    landscape: Arc<Image>,
}
impl HomeArtwork {
    fn get() -> &'static Self {
        static ART: OnceLock<HomeArtwork> = OnceLock::new();
        ART.get_or_init(|| Self {
            landscape: Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../../assets/home-landscape.png").to_vec(),
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
    layout: HomeLayout,
) -> impl IntoElement {
    let header_action = action.boxed_clone();
    v_flex()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .p(px(layout.card_padding))
        .gap(px(layout.card_gap))
        .rounded_xl()
        .border_1()
        .border_color(rgba(0x64c4ff99))
        .bg(rgba(0x061327df))
        .overflow_hidden()
        .child(
            ButtonLike::new(format!("{id}-heading"))
                .full_width()
                .height(px(layout.card_header_height).into())
                .style(ButtonStyle::Transparent)
                .aria_label(title)
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .text_left()
                        .gap(px(layout.card_gap))
                        .child(icon_tile(icon, layout.card_header_height))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .items_start()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(px(if layout.show_descriptions {
                                            22.
                                        } else {
                                            20.
                                        }))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child(title),
                                )
                                .when(layout.show_descriptions, |column| {
                                    column.child(
                                        div().w_full().overflow_hidden().child(
                                            Label::new(description)
                                                .truncate()
                                                .size(LabelSize::Small)
                                                .color(Color::Custom(rgb(0xc7e3f7).into())),
                                        ),
                                    )
                                }),
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
                .h(px(layout.button_height))
                .flex_none()
                .rounded_lg()
                .overflow_hidden()
                .border_1()
                .border_color(rgb(0x74caff))
                .bg(gpui::linear_gradient(
                    110.,
                    gpui::linear_color_stop(rgb(0x239bd2), 0.),
                    gpui::linear_color_stop(rgb(0x153769), 1.),
                ))
                .child(
                    ButtonLike::new(id)
                        .full_width()
                        .height(px(layout.button_height - 2.).into())
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

fn recent_card(
    recent: &RecentWorkspace,
    layout: HomeLayout,
    cx: &mut Context<KnightCodePage>,
) -> impl IntoElement {
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
    let padding = if layout.recent_height < 100. {
        10.
    } else {
        16.
    };
    let icon_size = if layout.recent_height < 100. {
        44.
    } else {
        64.
    };
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .rounded_lg()
        .border_1()
        .border_color(rgba(0x4299e066))
        .overflow_hidden()
        .bg(rgba(0x081426df))
        .child(
            ButtonLike::new(format!("home-recent-{}", i64::from(recent.workspace_id)))
                .full_width()
                .height(px(layout.recent_height - 2.).into())
                .style(ButtonStyle::Transparent)
                .aria_label(format!("Open {name}"))
                .child(
                    h_flex()
                        .relative()
                        .w_full()
                        .min_w_0()
                        .h_full()
                        .p(px(padding))
                        .gap(px(layout.card_gap))
                        .text_left()
                        .child(icon_tile(icon, icon_size))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .items_start()
                                .gap_1()
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
                                        .truncate()
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
    // Use logical client pixels, excluding Home chrome. Flex sizing still uses
    // the actual parent bounds, so frame rounding cannot introduce scrolling.
    let body_height = f32::from(window.viewport_size().height)
        - title_bar::platform_title_bar::KNIGHTCODE_HOME_TITLE_BAR_HEIGHT
        - 2.;
    // Only actual, successfully loaded project history belongs on Home.
    // Loading, a fresh install, and empty history show no recent-project UI.
    let has_recents = page.recent_error.is_none()
        && page
            .recent
            .as_ref()
            .is_some_and(|recent| !recent.is_empty());
    let layout = HomeLayout::new(
        f32::from(window.viewport_size().width),
        body_height,
        has_recents,
    );
    let recent_cards = page
        .recent
        .as_ref()
        .into_iter()
        .flatten()
        .take(layout.recent_count)
        .map(|recent| recent_card(recent, layout, cx).into_any_element())
        .collect::<Vec<_>>();
    v_flex()
        .id("home-content")
        .size_full()
        .min_h_0()
        .min_w_0()
        .relative()
        .overflow_hidden()
        .bg(rgb(0x030913))
        .text_color(rgb(0xf6f8ff))
        .child(
            img(HomeArtwork::get().landscape.clone())
                .absolute()
                .size_full()
                .object_fit(ObjectFit::Cover),
        )
        .child(
            v_flex()
                .id("home-layout")
                .relative()
                .size_full()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .pl(relative(layout.left_inset))
                .pr(relative(layout.right_inset))
                .pb(px(layout.bottom_padding))
                .child(
                    v_flex()
                        .id("home-hero")
                        .flex_1()
                        .min_h_0()
                        .max_w(px(layout.hero_width))
                        .justify_center()
                        .gap(px(layout.hero_gap))
                        .overflow_hidden()
                        .when(layout.show_hero, |hero| {
                            hero.when(layout.show_tagline, |hero| {
                                hero.child(
                                    div()
                                        .text_size(px(13.))
                                        .line_height(relative(1.2))
                                        .text_color(rgb(0x62d8ff))
                                        .child("T H I N K  •  B U I L D  •  B E Y O N D"),
                                )
                            })
                            .child(
                                v_flex()
                                    .text_size(px(layout.heading_size))
                                    .line_height(relative(1.10))
                                    .font_weight(gpui::FontWeight::BOLD)
                                    .child("Turn your ideas")
                                    .child(
                                        h_flex()
                                            .gap_3()
                                            .child("into")
                                            .child(div().text_color(rgb(0x91c9fa)).child("real"))
                                            .child(
                                                div().text_color(rgb(0x949cff)).child("impact."),
                                            ),
                                    ),
                            )
                            .when(layout.show_subtitle, |hero| {
                                hero.child(
                                    v_flex()
                                        .gap(px(4.))
                                        .text_size(px(layout.subtitle_size))
                                        .line_height(relative(1.2))
                                        .text_color(rgb(0xbfe5fa))
                                        .child(
                                            "Chat with AI, or open a project and start building.",
                                        )
                                        .child("Same intelligence. More possibilities."),
                                )
                            })
                        }),
                )
                .child(
                    h_flex()
                        .id("home-actions")
                        .w_full()
                        .h(px(layout.action_height))
                        .flex_none()
                        .gap(px(layout.row_gap))
                        .child(action_card(
                            "home-start-chat",
                            "Chat",
                            "Ask, explore, brainstorm and get instant answers.",
                            "Start chatting",
                            IconName::Chat,
                            title_bar::ShowChat.boxed_clone(),
                            layout,
                        ))
                        .child(action_card(
                            "home-open-project",
                            "Build",
                            "Open a project, write code, run commands and build with AI.",
                            "Open project",
                            IconName::Code,
                            workspace::Open::default().boxed_clone(),
                            layout,
                        )),
                )
                .when(has_recents, |view| {
                    view.child(
                        h_flex()
                            .w_full()
                            .flex_none()
                            .h(px(layout.recent_header_height))
                            .mt(px(layout.recent_top_gap))
                            .justify_between()
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(
                                        Icon::new(IconName::Clock)
                                            .color(Color::Custom(rgb(0xb8eaff).into())),
                                    )
                                    .child(
                                        Label::new("Recent projects")
                                            .color(Color::Custom(rgb(0xf3f7ff).into())),
                                    ),
                            )
                            .child(
                                Button::new("home-all-projects", "View all")
                                    .end_icon(Icon::new(IconName::ArrowRight))
                                    .color(Color::Custom(rgb(0xc8eaff).into()))
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(
                                            zed_actions::OpenRecent::default().boxed_clone(),
                                            cx,
                                        )
                                    }),
                            ),
                    )
                    .when(layout.show_recent_cards, |view| {
                        view.child(
                            h_flex()
                                .id("home-recents")
                                .w_full()
                                .h(px(layout.recent_height))
                                .flex_none()
                                .mt(px(layout.recent_bottom_gap))
                                .gap_3()
                                .children(recent_cards),
                        )
                    })
                }),
        )
        .into_any_element()
}
