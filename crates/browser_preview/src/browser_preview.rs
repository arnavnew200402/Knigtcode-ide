use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail};
use futures::future::{Either, select};
use gpui::{
    AnyWindowHandle, AppContext, AsyncApp, Bounds, Entity, EventEmitter, FocusHandle, Focusable,
    Global, Hitbox, HitboxBehavior, KeyBinding, Pixels, Subscription, Task, WeakEntity, actions,
    canvas,
};
use serde::Deserialize;
use ui::prelude::*;
use ui_input::InputField;
use workspace::{Item, Workspace, item::ItemEvent};

mod native;
mod screenshot;
use native::{NativeEvent, NativeHost};
use screenshot::png_dimensions;

actions!(
    browser_preview,
    [
        OpenPreview,
        Navigate,
        Back,
        Forward,
        Reload,
        OpenExternal,
        AttachScreenshot,
        FocusPage
    ]
);

#[derive(Clone, Debug)]
pub struct PreviewScreenshot {
    pub png: Arc<[u8]>,
    pub page_url: String,
    pub width: u32,
    pub height: u32,
}

impl PreviewScreenshot {
    pub const MIME_TYPE: &'static str = "image/png";
    pub const FILE_NAME: &'static str = "page-preview.png";
}

type AttachmentHandler = dyn Fn(PreviewScreenshot, &mut Window, &mut App) -> Result<()>;

pub struct PreviewAttachmentService(Rc<AttachmentHandler>);
impl Global for PreviewAttachmentService {}

impl PreviewAttachmentService {
    pub fn install(
        cx: &mut App,
        handler: impl Fn(PreviewScreenshot, &mut Window, &mut App) -> Result<()> + 'static,
    ) {
        cx.set_global(Self(Rc::new(handler)));
    }
}

#[derive(Clone, Debug)]
pub enum PreviewEvent {
    Updated,
    ScreenshotReady(PreviewScreenshot),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewStatus {
    Starting,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Deserialize)]
struct ResourceManifest {
    schema_version: u32,
    architecture: String,
    webview2: RuntimeManifest,
}

#[derive(Deserialize)]
struct RuntimeManifest {
    version: String,
    archive_sha256: String,
}

pub struct BrowserResources {
    pub runtime_directory: PathBuf,
    pub runtime_version: String,
    pub user_data_directory: PathBuf,
}

impl BrowserResources {
    pub fn bundled() -> Result<Self> {
        let executable = std::env::current_exe().context("Locating the IDE executable")?;
        let parent = executable
            .parent()
            .context("Executable has no parent directory")?;
        let directory = parent.join("resources").join("desktop");
        let manifest: ResourceManifest = serde_json::from_slice(
            &std::fs::read(directory.join("manifest.json"))
                .context("Bundled desktop resource manifest is missing; reinstall KnightCode")?,
        )?;
        let architecture = match std::env::consts::ARCH {
            "x86_64" => "x64",
            "aarch64" => "arm64",
            architecture => bail!("Embedded preview does not support {architecture}"),
        };
        if manifest.schema_version != 1 || manifest.architecture != architecture {
            bail!("Bundled browser runtime manifest does not match this IDE");
        }
        if manifest.webview2.version.trim().is_empty()
            || manifest.webview2.archive_sha256.len() != 64
            || !manifest
                .webview2
                .archive_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("Bundled browser runtime manifest is invalid");
        }
        let runtime_directory = directory.join("webview2");
        if !runtime_directory.join("msedgewebview2.exe").is_file() {
            bail!("Bundled fixed WebView2 runtime is missing; reinstall KnightCode");
        }
        Ok(Self {
            runtime_directory,
            runtime_version: manifest.webview2.version,
            user_data_directory: paths::data_dir().join("browser-preview"),
        })
    }
}

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "enter",
        Navigate,
        Some("BrowserPreviewUrl"),
    )]);
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &OpenPreview, window, cx| {
            if let Some(preview) = workspace.active_item_as::<BrowserPreview>(cx) {
                window.focus(&preview.focus_handle(cx), cx);
                return;
            }
            match open_preview(workspace, "http://localhost:3000", window, cx) {
                Ok(_) => {}
                Err(error) => log::error!("Could not open browser preview: {error:#}"),
            }
        });
    })
    .detach();
}

pub fn open_preview(
    workspace: &mut Workspace,
    url: &str,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) -> Result<Entity<BrowserPreview>> {
    if ui_input::ERASED_EDITOR_FACTORY.get().is_none() {
        bail!("Initialize editor before browser_preview");
    }
    let url = normalize_url(url)?;
    let workspace_handle = cx.weak_entity();
    let preview = cx.new(|cx| BrowserPreview::new(workspace_handle, url, window, cx));
    workspace.add_item_to_active_pane(Box::new(preview.clone()), None, true, window, cx);
    Ok(preview)
}

pub struct BrowserPreview {
    workspace: WeakEntity<Workspace>,
    window_handle: AnyWindowHandle,
    item_visible: Cell<bool>,
    native_visible: Cell<Option<bool>>,
    page_hitbox: Option<Hitbox>,
    url_editor: Entity<InputField>,
    focus_handle: FocusHandle,
    page_url: String,
    status: PreviewStatus,
    operation_error: Option<String>,
    host: Option<NativeHost>,
    native_task: Option<Task<()>>,
    startup_task: Option<Task<()>>,
    visibility_task: Option<Task<()>>,
    can_go_back: bool,
    can_go_forward: bool,
    capturing: bool,
    suspended: bool,
    _focus_subscription: Subscription,
}

impl BrowserPreview {
    fn new(
        workspace: WeakEntity<Workspace>,
        page_url: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let url_editor = cx.new(|cx| InputField::new(window, cx, "http://localhost:3000"));
        let editor = url_editor.read(cx).editor().clone();
        editor.set_text(&page_url, window, cx);
        let focus_handle = cx.focus_handle();
        let subscription = cx.on_focus_out(&focus_handle, window, |this, _, _, _| {
            if let Some(host) = &this.host {
                if let Err(error) = host.set_visible(false) {
                    log::error!("Hiding unfocused browser: {error:#}");
                } else {
                    this.native_visible.set(Some(false));
                }
            }
        });
        Self {
            workspace,
            window_handle: window.window_handle(),
            item_visible: Cell::new(false),
            native_visible: Cell::new(None),
            page_hitbox: None,
            url_editor,
            focus_handle,
            page_url,
            status: PreviewStatus::Starting,
            operation_error: None,
            host: None,
            native_task: None,
            startup_task: None,
            visibility_task: None,
            can_go_back: false,
            can_go_forward: false,
            capturing: false,
            suspended: false,
            _focus_subscription: subscription,
        }
    }

    pub fn status(&self) -> &PreviewStatus {
        &self.status
    }

    pub fn page_url(&self) -> &str {
        &self.page_url
    }

    /// Suspend before rendering an overlapping non-modal GPUI menu or tooltip.
    /// HWND children are outside GPUI's GPU scene and cannot share its stacking order.
    pub fn set_overlay_suspended(&mut self, suspended: bool, cx: &mut Context<Self>) {
        self.suspended = suspended;
        if suspended {
            if let Some(host) = &self.host {
                if let Err(error) = host.set_visible(false) {
                    self.operation_error = Some(format!("Hiding embedded browser: {error:#}"));
                }
            }
        }
        self.native_visible.set(None);
        cx.notify();
    }

    pub fn navigate_to(&mut self, url: &str, cx: &mut Context<Self>) -> Result<()> {
        let url = normalize_url(url)?;
        if let Some(host) = &self.host {
            host.navigate(&url)?;
        }
        self.page_url = url;
        self.status = if self.host.is_some() {
            PreviewStatus::Loading
        } else {
            PreviewStatus::Starting
        };
        self.operation_error = None;
        cx.notify();
        Ok(())
    }

    pub fn capture_page(&mut self, cx: &mut Context<Self>) -> Task<Result<PreviewScreenshot>> {
        if self.status != PreviewStatus::Ready {
            return Task::ready(Err(anyhow!(
                "Wait for the page to finish loading before capturing"
            )));
        }
        let receiver = match self
            .host
            .as_ref()
            .context("Embedded browser is not initialized")
            .and_then(|host| host.capture_png())
        {
            Ok(receiver) => receiver,
            Err(error) => return Task::ready(Err(error)),
        };
        let timeout = cx.background_executor().timer(Duration::from_secs(15));
        cx.spawn(async move |_, _| {
            let capture = match select(receiver, timeout).await {
                Either::Left((result, _)) => {
                    result.context("Browser closed during screenshot capture")??
                }
                Either::Right(_) => bail!("Page screenshot capture timed out; reload and retry"),
            };
            let (width, height) = png_dimensions(&capture.png).map_err(|error| anyhow!(error))?;
            Ok(PreviewScreenshot {
                png: capture.png.into(),
                page_url: capture.page_url,
                width,
                height,
            })
        })
    }

    pub fn attach_screenshot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.capturing {
            return;
        }
        if !cx.has_global::<PreviewAttachmentService>() {
            self.operation_error =
                Some("The agent screenshot attachment service is not initialized".into());
            cx.notify();
            return;
        }
        let capture = self.capture_page(cx);
        self.window_handle = window.window_handle();
        self.capturing = true;
        self.operation_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = capture.await;
            if let Err(error) = Self::update_current_window(&this, cx, |this, window, cx| {
                this.capturing = false;
                match result {
                    Ok(screenshot) => {
                        let result = cx
                            .try_global::<PreviewAttachmentService>()
                            .map(|service| service.0.clone())
                            .context("The agent screenshot attachment service was removed")
                            .and_then(|handler| handler(screenshot.clone(), window, cx));
                        match result {
                            Ok(()) => cx.emit(PreviewEvent::ScreenshotReady(screenshot)),
                            Err(error) => {
                                this.operation_error =
                                    Some(format!("Attaching page screenshot: {error:#}"))
                            }
                        }
                    }
                    Err(error) => {
                        this.operation_error = Some(format!("Capturing page screenshot: {error:#}"))
                    }
                }
                cx.notify();
            }) {
                log::debug!("Screenshot target was closed: {error:#}");
                if let Err(update_error) = this.update(cx, |this, cx| {
                    this.capturing = false;
                    this.operation_error = Some(format!(
                        "Screenshot target window is unavailable: {error:#}"
                    ));
                    cx.notify();
                }) {
                    log::debug!("Screenshot preview was closed: {update_error:#}");
                }
            }
        })
        .detach();
    }

    fn start_host(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Result<()> {
        if !cfg!(target_os = "windows") {
            bail!("Embedded browser preview is currently available on Windows only");
        }
        let (host, receiver) = NativeHost::new(window, BrowserResources::bundled()?)?;
        self.host = Some(host);
        self.native_visible.set(None);
        let executor = cx.background_executor().clone();
        // Pane caching can replay the preview without invoking its canvas again.
        // Poll visibility without invalidating or redrawing the GPU scene.
        self.visibility_task = Some(cx.spawn(async move |this, cx| {
            loop {
                executor.timer(Duration::from_millis(50)).await;
                let Ok(visible) = this.read_with(cx, |this, _| this.item_visible.get()) else {
                    break;
                };
                if !visible {
                    continue;
                }
                let result = Self::update_current_window(&this, cx, |this, window, cx| {
                    let visible = !this.overlay_active(window, cx)
                        && !matches!(this.status, PreviewStatus::Failed(_));
                    if this.native_visible.get() != Some(visible) {
                        if let Some(host) = &this.host {
                            if let Err(error) = host.set_visible(visible) {
                                this.native_visible.set(Some(false));
                                this.status = PreviewStatus::Failed(format!(
                                    "Updating browser visibility: {error:#}"
                                ));
                                cx.notify();
                            } else {
                                this.native_visible.set(Some(visible));
                            }
                        }
                    }
                });
                if let Err(error) = result {
                    log::debug!("Browser window is unavailable: {error:#}");
                }
            }
        }));
        let timeout = cx.background_executor().timer(Duration::from_secs(30));
        self.startup_task = Some(cx.spawn(async move |this, cx| {
            timeout.await;
            if let Err(error) = this.update(cx, |this, cx| {
                if this.status == PreviewStatus::Starting {
                    this.status = PreviewStatus::Failed(
                        "Bundled browser startup timed out; reload to retry".into(),
                    );
                    cx.notify();
                }
            }) {
                log::debug!("Browser startup target was closed: {error:#}");
            }
        }));
        self.native_task = Some(cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                let result = Self::update_current_window(&this, cx, |this, window, cx| {
                    let result = this.handle_native_event(event, window, cx);
                    if let Err(error) = result {
                        this.status = PreviewStatus::Failed(format!("Embedded browser: {error:#}"));
                    }
                    cx.notify();
                });
                if let Err(error) = result {
                    log::debug!("Embedded preview was closed: {error:#}");
                    break;
                }
            }
        }));
        Ok(())
    }

    fn update_current_window<R>(
        this: &WeakEntity<Self>,
        cx: &mut AsyncApp,
        update: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) -> R,
    ) -> Result<R> {
        let window_handle = this.read_with(cx, |this, _| this.window_handle)?;
        window_handle.update(cx, |_, window, cx| {
            this.update(cx, |this, cx| update(this, window, cx))
        })?
    }

    fn handle_native_event(
        &mut self,
        event: NativeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        match event {
            NativeEvent::Ready => {
                self.startup_task.take();
                self.host
                    .as_ref()
                    .context("Browser host disappeared")?
                    .navigate(&self.page_url)?;
                self.status = PreviewStatus::Loading;
            }
            NativeEvent::Loading => self.status = PreviewStatus::Loading,
            NativeEvent::Loaded => self.status = PreviewStatus::Ready,
            NativeEvent::Failed(error) => self.status = PreviewStatus::Failed(error),
            NativeEvent::History { url, back, forward } => {
                self.page_url = url;
                self.can_go_back = back;
                self.can_go_forward = forward;
                let editor = self.url_editor.read(cx).editor().clone();
                editor.set_text(&self.page_url, window, cx);
                cx.emit(PreviewEvent::Updated);
            }
            NativeEvent::Focused => {
                if !self.overlay_active(window, cx) && self.item_visible.get() {
                    window.focus(&self.focus_handle, cx);
                } else if let Some(host) = &self.host {
                    host.set_visible(false)?;
                    self.native_visible.set(Some(false));
                }
            }
            NativeEvent::FocusAddress => {
                if let Some(host) = &self.host {
                    host.return_focus_to_parent()?;
                }
                window.focus(&self.url_editor.focus_handle(cx), cx);
                let editor = self.url_editor.read(cx).editor().clone();
                editor.select_all(window, cx);
            }
        }
        Ok(())
    }

    fn perform(
        &mut self,
        operation: impl FnOnce(&NativeHost) -> Result<()>,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self
            .host
            .as_ref()
            .context("Embedded browser is not initialized")
            .and_then(operation)
        {
            self.operation_error = Some(format!("{error:#}"));
        } else {
            self.operation_error = None;
        }
        cx.notify();
    }

    fn go(&mut self, _: &Navigate, _: &mut Window, cx: &mut Context<Self>) {
        let url = self.url_editor.read(cx).text(cx);
        if let Err(error) = self.navigate_to(&url, cx) {
            self.operation_error = Some(format!("{error:#}"));
            cx.notify();
        }
    }

    fn reload_page(&mut self, cx: &mut Context<Self>) {
        if self.host.is_none() || matches!(self.status, PreviewStatus::Failed(_)) {
            self.stop_host();
            self.status = PreviewStatus::Starting;
            self.operation_error = None;
            cx.notify();
        } else {
            self.perform(NativeHost::reload, cx);
        }
    }

    fn open_external(&mut self, cx: &mut Context<Self>) {
        cx.open_url(&self.page_url);
    }

    fn stop_host(&mut self) {
        self.native_task.take();
        self.startup_task.take();
        self.visibility_task.take();
        self.host.take();
        self.native_visible.set(None);
    }

    fn place_host(
        &mut self,
        bounds: Bounds<Pixels>,
        hitbox: Hitbox,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.window_handle = window.window_handle();
        self.item_visible.set(true);
        self.page_hitbox = Some(hitbox);
        if self.host.is_none() && self.status == PreviewStatus::Starting {
            if let Err(error) = self.start_host(window, cx) {
                self.status = PreviewStatus::Failed(format!("{error:#}"));
                cx.notify();
                return;
            }
        }
        let visible =
            !self.overlay_active(window, cx) && !matches!(self.status, PreviewStatus::Failed(_));
        if let Some(host) = &self.host {
            if let Err(error) = host.set_bounds(window, bounds, visible) {
                self.stop_host();
                self.status = PreviewStatus::Failed(format!("Sizing embedded browser: {error:#}"));
                cx.notify();
            } else {
                self.native_visible.set(Some(visible));
            }
        }
    }

    fn overlay_active(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let modal = self.workspace.upgrade().is_some_and(|workspace| {
            workspace.update(cx, |workspace, cx| workspace.has_active_modal(window, cx))
        });
        let menu = window
            .context_stack()
            .iter()
            .any(|context| context.contains("menu") || context.contains("ContextMenu"));
        let page_occluded = self
            .page_hitbox
            .as_ref()
            .is_none_or(|hitbox| !hitbox.is_hovered_at(hitbox.bounds.center(), window));
        self.suspended
            || modal
            || menu
            || page_occluded
            || window.has_active_prompt()
            || cx.has_active_drag()
    }
}

impl EventEmitter<PreviewEvent> for BrowserPreview {}
impl Focusable for BrowserPreview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
impl Item for BrowserPreview {
    type Event = PreviewEvent;

    fn tab_content_text(&self, _: usize, _: &App) -> SharedString {
        "Browser Preview".into()
    }
    fn tab_tooltip_text(&self, _: &App) -> Option<SharedString> {
        Some(self.page_url.clone().into())
    }
    fn to_item_events(event: &PreviewEvent, emit: &mut dyn FnMut(ItemEvent)) {
        if matches!(event, PreviewEvent::Updated) {
            emit(ItemEvent::UpdateTab);
        }
    }
    fn deactivated(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.item_visible.set(false);
        self.native_visible.set(Some(false));
        if let Some(host) = &self.host {
            if let Err(error) = host.set_visible(false) {
                log::error!("Hiding inactive preview: {error:#}");
            }
        }
    }
    fn on_removed(&self, _: &mut Context<Self>) {
        self.item_visible.set(false);
        self.native_visible.set(Some(false));
        if let Some(host) = &self.host {
            if let Err(error) = host.set_visible(false) {
                log::error!("Hiding removed preview: {error:#}");
            }
        }
    }

    fn added_to_workspace(
        &mut self,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace = workspace.weak_handle();
        self.window_handle = window.window_handle();
        self.item_visible.set(false);
        self.page_hitbox = None;
        self.native_visible.set(None);
        if let Some(host) = &self.host {
            if let Err(error) = host.set_visible(false) {
                log::error!("Hiding browser during workspace move: {error:#}");
            }
        }
        self._focus_subscription = cx.on_focus_out(&self.focus_handle, window, |this, _, _, _| {
            if let Some(host) = &this.host {
                if let Err(error) = host.set_visible(false) {
                    log::error!("Hiding unfocused browser: {error:#}");
                } else {
                    this.native_visible.set(Some(false));
                }
            }
        });
    }
}

impl Render for BrowserPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.weak_entity();
        let status = match &self.status {
            PreviewStatus::Starting => "Starting bundled browser…".into(),
            PreviewStatus::Loading => "Loading…".into(),
            PreviewStatus::Ready => self.page_url.clone(),
            PreviewStatus::Failed(error) => error.clone(),
        };
        let error = self.operation_error.clone();
        let address_focus = self.url_editor.focus_handle(cx);
        v_flex()
            .id("browser-preview")
            .key_context("BrowserPreview")
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .bg(cx.theme().colors().editor_background)
            .on_action(cx.listener(Self::go))
            .on_action(cx.listener(|this, _: &Back, _, cx| this.perform(NativeHost::back, cx)))
            .on_action(
                cx.listener(|this, _: &Forward, _, cx| this.perform(NativeHost::forward, cx)),
            )
            .on_action(cx.listener(|this, _: &Reload, _, cx| this.reload_page(cx)))
            .on_action(cx.listener(|this, _: &OpenExternal, _, cx| this.open_external(cx)))
            .on_action(cx.listener(|this, _: &AttachScreenshot, window, cx| {
                this.attach_screenshot(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &FocusPage, _, cx| this.perform(NativeHost::focus, cx)),
            )
            .child(
                h_flex()
                    .flex_none()
                    .flex_wrap()
                    .gap_1()
                    .p_2()
                    .child(
                        Button::new("browser-back", "Back")
                            .disabled(!self.can_go_back)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.perform(NativeHost::back, cx)),
                            ),
                    )
                    .child(
                        Button::new("browser-forward", "Forward")
                            .disabled(!self.can_go_forward)
                            .on_click(
                                cx.listener(|this, _, _, cx| this.perform(NativeHost::forward, cx)),
                            ),
                    )
                    .child(
                        Button::new("browser-reload", "Reload")
                            .on_click(cx.listener(|this, _, _, cx| this.reload_page(cx))),
                    )
                    .child(
                        div()
                            .key_context("BrowserPreviewUrl")
                            .track_focus(&address_focus)
                            .flex_1()
                            .min_w(px(192.))
                            .child(self.url_editor.clone()),
                    )
                    .child(Button::new("browser-go", "Go").on_click(
                        cx.listener(|this, _, window, cx| this.go(&Navigate, window, cx)),
                    ))
                    .child(
                        Button::new("browser-external", "Open External")
                            .on_click(cx.listener(|this, _, _, cx| this.open_external(cx))),
                    )
                    .child(
                        Button::new(
                            "browser-screenshot",
                            if self.capturing {
                                "Capturing…"
                            } else {
                                "Attach Screenshot"
                            },
                        )
                        .disabled(self.capturing || self.status != PreviewStatus::Ready)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.attach_screenshot(window, cx)),
                        ),
                    ),
            )
            .child(Label::new(status).size(LabelSize::Small).color(
                if matches!(self.status, PreviewStatus::Failed(_)) {
                    Color::Error
                } else {
                    Color::Muted
                },
            ))
            .when_some(error, |element, error| {
                element.child(Label::new(error).color(Color::Error))
            })
            .child(
                canvas(
                    |bounds, window, _| {
                        let bounds = bounds.intersect(&window.content_mask().bounds);
                        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
                        (bounds, hitbox)
                    },
                    move |_, (bounds, hitbox), window, cx| {
                        let view = view.clone();
                        // Deferring avoids updating an entity from inside its own render pass.
                        window.defer(cx, move |window, cx| {
                            if let Err(error) = view
                                .update(cx, |this, cx| this.place_host(bounds, hitbox, window, cx))
                            {
                                log::debug!("Preview layout target was closed: {error:#}");
                            }
                        });
                    },
                )
                .w_full()
                .flex_1(),
            )
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    if this.focus_handle.is_focused(window) {
                        this.perform(NativeHost::focus, cx);
                    }
                }),
            )
    }
}

fn normalize_url(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() || input.chars().any(char::is_control) {
        bail!("Enter an HTTP or HTTPS page URL");
    }
    let input = if input.contains("://") {
        input.to_owned()
    } else {
        if let Some((prefix, remainder)) = input.split_once(':') {
            let scheme_like = prefix.starts_with(|character: char| character.is_ascii_alphabetic())
                && prefix.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
                });
            let port = remainder
                .split('/')
                .next()
                .and_then(|port| port.parse::<u16>().ok());
            if scheme_like && port.is_none() {
                bail!("Preview supports HTTP and HTTPS page URLs");
            }
        }
        format!("http://{input}")
    };
    let url = url::Url::parse(&input).context("Invalid preview URL")?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("Preview supports HTTP and HTTPS page URLs");
    }
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_urls_and_unsupported_schemes() -> Result<()> {
        assert_eq!(
            normalize_url(" localhost:3000/path ")?,
            "http://localhost:3000/path"
        );
        assert_eq!(
            normalize_url("https://example.com")?,
            "https://example.com/"
        );
        assert!(normalize_url("file:///C:/secret").is_err());
        assert!(normalize_url("javascript://alert(1)").is_err());
        assert!(normalize_url("javascript:alert(1)").is_err());
        assert!(normalize_url("mailto:user@example.com").is_err());
        assert!(normalize_url("localhost\n:3000").is_err());
        Ok(())
    }
}
