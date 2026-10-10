#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use {
    crate::BrowserResources,
    anyhow::{Result, bail},
    async_channel::Receiver,
    futures::channel::oneshot,
    gpui::{Bounds, Pixels, Window},
};

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod windows_host;
#[cfg(target_os = "windows")]
pub use windows_host::NativeHost;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod macos_host;
#[cfg(target_os = "macos")]
pub use macos_host::NativeHost;

pub struct NativeCapture {
    pub png: Vec<u8>,
    pub page_url: String,
}

pub enum NativeEvent {
    Ready,
    Loading,
    Loaded,
    Failed(String),
    History {
        url: String,
        back: bool,
        forward: bool,
    },
    Focused,
    FocusAddress,
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub struct NativeHost;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl NativeHost {
    pub fn new(_: &Window, _: BrowserResources) -> Result<(Self, Receiver<NativeEvent>)> {
        bail!("Embedded browser preview requires Windows WebView2 or macOS WebKit")
    }
    pub fn navigate(&self, _: &str) -> Result<()> {
        bail!("WebView2 requires Windows")
    }
    pub fn reload(&self) -> Result<()> {
        bail!("WebView2 requires Windows")
    }
    pub fn back(&self) -> Result<()> {
        bail!("WebView2 requires Windows")
    }
    pub fn forward(&self) -> Result<()> {
        bail!("WebView2 requires Windows")
    }
    pub fn focus(&self) -> Result<()> {
        bail!("WebView2 requires Windows")
    }
    pub fn return_focus_to_parent(&self) -> Result<()> {
        bail!("WebView2 requires Windows")
    }
    pub fn set_visible(&self, _: bool) -> Result<()> {
        bail!("WebView2 requires Windows")
    }
    pub fn set_bounds(&self, _: &Window, _: Bounds<Pixels>, _: bool) -> Result<()> {
        bail!("WebView2 requires Windows")
    }
    pub fn capture_png(&self) -> Result<oneshot::Receiver<Result<NativeCapture>>> {
        bail!("WebView2 requires Windows")
    }
}
