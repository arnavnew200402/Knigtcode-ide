//! Viewport-fit Home sizing in logical pixels (GPUI already accounts for DPI).
//! The hero consumes remaining space; the action/recent rows never wrap or scroll.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HomeLayout {
    pub left_inset: f32,
    pub right_inset: f32,
    pub bottom_padding: f32,
    pub row_gap: f32,
    pub card_padding: f32,
    pub card_gap: f32,
    pub card_header_height: f32,
    pub button_height: f32,
    pub action_height: f32,
    pub recent_height: f32,
    pub recent_top_gap: f32,
    pub recent_bottom_gap: f32,
    pub recent_header_height: f32,
    pub recent_count: usize,
    pub show_recent_cards: bool,
    pub show_descriptions: bool,
    pub show_hero: bool,
    pub show_tagline: bool,
    pub show_subtitle: bool,
    pub hero_width: f32,
    pub heading_size: f32,
    pub subtitle_size: f32,
    pub hero_gap: f32,
}

impl HomeLayout {
    pub fn new(width: f32, body_height: f32) -> Self {
        let compact = body_height < 560.;
        let very_short = body_height < 400.;
        let narrow = width < 850.;
        let left_inset = if width < 1100. { 0.05 } else { 0.094 };
        let right_inset = if width < 1100. { 0.05 } else { 0.068 };
        let content_width = width * (1. - left_inset - right_inset);
        let card_padding = if compact { 12. } else { 20. };
        let card_gap = if compact { 8. } else { 16. };
        let card_header_height = if very_short {
            40.
        } else if compact {
            48.
        } else {
            72.
        };
        let button_height = if very_short {
            32.
        } else if compact {
            36.
        } else {
            50.
        };
        let action_height = card_padding * 2. + card_header_height + card_gap + button_height + 2.;
        let recent_height = if very_short {
            80.
        } else if compact {
            90.
        } else {
            114.
        };
        let recent_top_gap = if compact { 12. } else { 24. };
        let recent_bottom_gap = if compact { 8. } else { 12. };
        let recent_header_height = 28.;
        let bottom_padding = if compact { 12. } else { 24. };
        let show_recent_cards = body_height >= 270.;
        let fixed_height = action_height
            + recent_top_gap
            + recent_header_height
            + if show_recent_cards {
                recent_bottom_gap + recent_height
            } else {
                0.
            }
            + bottom_padding;
        let hero_height = (body_height - fixed_height).max(0.);
        let hero_width = if narrow {
            content_width
        } else {
            (content_width * 0.62).min(700.)
        };
        let hero_gap = if compact { 10. } else { 16. };
        let subtitle_size = if compact || narrow { 16. } else { 20. };
        let show_hero = hero_height >= 64.;
        let show_tagline = hero_height >= 110.;
        let show_subtitle = hero_height >= 170. && width >= 600.;
        let ancillary_height = if show_tagline { 18. + hero_gap } else { 0. }
            + if show_subtitle {
                subtitle_size * 2.8 + hero_gap
            } else {
                0.
            };
        let heading_size = ((hero_height - ancillary_height) / 2.2)
            .min(hero_width / 7.8)
            .clamp(22., 64.);
        Self {
            left_inset,
            right_inset,
            bottom_padding,
            row_gap: if compact || narrow { 12. } else { 20. },
            card_padding,
            card_gap,
            card_header_height,
            button_height,
            action_height,
            recent_height,
            recent_top_gap,
            recent_bottom_gap,
            recent_header_height,
            recent_count: if content_width >= 840. {
                3
            } else if content_width >= 540. {
                2
            } else {
                1
            },
            show_recent_cards,
            show_descriptions: !very_short && width >= 1000.,
            show_hero,
            show_tagline,
            show_subtitle,
            hero_width,
            heading_size,
            subtitle_size,
            hero_gap,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_home_viewport_fit_at_common_sizes_and_dpi_scales() {
        for (width, height) in [
            (1916., 1017.),
            (1920., 1080.),
            (1584., 993.),
            (1366., 768.),
            (1024., 768.),
            (800., 600.),
        ] {
            for dpi in [1., 1.25, 1.5, 2.] {
                let width = width / dpi;
                let body_height = height / dpi - 50.; // Slim Home header + window border.
                let layout = HomeLayout::new(width, body_height);
                assert!(layout.left_inset + layout.right_inset < 0.2);
                assert!(layout.row_gap >= 12. && layout.hero_width <= width);
                assert!(layout.button_height >= 32.);
                assert_eq!(
                    layout.action_height,
                    layout.card_padding * 2.
                        + layout.card_header_height
                        + layout.card_gap
                        + layout.button_height
                        + 2.
                );
                let fixed_height = layout.action_height
                    + layout.recent_top_gap
                    + layout.recent_header_height
                    + if layout.show_recent_cards {
                        layout.recent_bottom_gap + layout.recent_height
                    } else {
                        0.
                    }
                    + layout.bottom_padding;
                assert!(
                    fixed_height <= body_height,
                    "{width} x {body_height}: {layout:?}"
                );
                if layout.show_hero {
                    let hero_text_height = layout.heading_size * 2.2
                        + if layout.show_tagline {
                            18. + layout.hero_gap
                        } else {
                            0.
                        }
                        + if layout.show_subtitle {
                            layout.subtitle_size * 2.8 + layout.hero_gap
                        } else {
                            0.
                        };
                    assert!(
                        hero_text_height <= body_height - fixed_height + 0.01,
                        "Hero overflows: {layout:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_home_screenshot_at_windows_125_percent_keeps_all_recent_cards() {
        let layout = HomeLayout::new(1916. / 1.25, 1017. / 1.25 - 50.);
        assert_eq!(layout.recent_count, 3);
        assert!(layout.show_recent_cards && layout.show_hero && layout.show_subtitle);
    }

    #[test]
    fn test_home_narrow_windows_limit_recent_cards_instead_of_wrapping() {
        assert_eq!(HomeLayout::new(500., 500.).recent_count, 1);
        assert_eq!(HomeLayout::new(750., 500.).recent_count, 2);
        assert_eq!(HomeLayout::new(1280., 700.).recent_count, 3);
        assert!(!HomeLayout::new(500., 500.).show_descriptions);
    }
}
