use gpui::{Image, ImageFormat};
use std::sync::{Arc, OnceLock};

/// Shared decoded-image identity across all native Build conversations.
pub(crate) fn knight() -> Arc<Image> {
    static KNIGHT: OnceLock<Arc<Image>> = OnceLock::new();
    KNIGHT
        .get_or_init(|| {
            Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!("../assets/build-agent-knight.png").to_vec(),
            ))
        })
        .clone()
}
