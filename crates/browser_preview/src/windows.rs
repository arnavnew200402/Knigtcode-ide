use std::{cell::RefCell, rc::Rc};

use anyhow::{Context as _, Result, anyhow, bail};
use async_channel::{Receiver, Sender};
use futures::channel::oneshot;
use gpui::{Bounds, Pixels, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use webview2_com::{
    CapturePreviewCompletedHandler, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, FocusChangedEventHandler,
    HistoryChangedEventHandler, Microsoft::Web::WebView2::Win32::*, MoveFocusRequestedEventHandler,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, ProcessFailedEventHandler, SourceChangedEventHandler,
    take_pwstr,
};
use windows::{
    Win32::{
        Foundation::{HGLOBAL, HWND, RECT},
        System::Com::{
            STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET, StructuredStorage::CreateStreamOnHGlobal,
        },
        UI::{
            Input::KeyboardAndMouse::{GetFocus, SetFocus},
            WindowsAndMessaging::IsChild,
        },
    },
    core::{BOOL, Interface, PCWSTR, PWSTR},
};

use crate::{
    BrowserResources,
    native::{NativeCapture, NativeEvent},
};

struct Controller {
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
    _environment: ICoreWebView2Environment,
}

impl Drop for Controller {
    fn drop(&mut self) {
        if let Err(error) = unsafe { self.controller.Close() } {
            log::error!("Closing WebView2 controller: {error}");
        }
    }
}

pub struct NativeHost {
    controller: Rc<RefCell<Option<Controller>>>,
    parent: RefCell<HWND>,
}

impl NativeHost {
    pub fn new(
        window: &Window,
        resources: BrowserResources,
    ) -> Result<(Self, Receiver<NativeEvent>)> {
        let parent = hwnd(window)?;
        // A null browser folder selects Microsoft's shared Evergreen runtime.
        // Do not pin its version: Windows/Microsoft updates it independently.
        let mut version = PWSTR::null();
        unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) }
            .context("Microsoft WebView2 Runtime is missing or unavailable. Install it from https://developer.microsoft.com/microsoft-edge/webview2/consumer/ and reload browser preview")?;
        let version = take_pwstr(version);
        if version.is_empty() {
            bail!("Microsoft WebView2 Runtime returned no version; repair the system runtime");
        }
        log::info!("Using system WebView2 runtime {version}");
        std::fs::create_dir_all(&resources.user_data_directory)
            .context("Creating browser profile directory")?;
        let user_data = resources.user_data_directory.as_os_str().to_string_lossy();
        let user_data: Vec<u16> = user_data.encode_utf16().chain(Some(0)).collect();
        let controller = Rc::new(RefCell::new(None));
        let weak_controller = Rc::downgrade(&controller);
        let (sender, receiver) = async_channel::unbounded();
        let environment_sender = sender.clone();
        let environment_handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment| {
                let result = (|| -> Result<()> {
                    result.context("Creating system WebView2 environment; install or repair Microsoft WebView2 Runtime if unavailable")?;
                    let environment = environment.context("WebView2 returned no environment")?;
                    let mut version = PWSTR::null();
                    unsafe {
                        environment.BrowserVersionString(&mut version)?;
                    }
                    let version = take_pwstr(version);
                    log::debug!("System WebView2 environment version: {version}");
                    if weak_controller.upgrade().is_none() {
                        return Ok(());
                    }
                    let controller_sender = environment_sender.clone();
                    let callback_environment = environment.clone();
                    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                        move |result, controller| {
                            let result = (|| -> Result<()> {
                                result.context("Creating native WebView2 controller")?;
                                let controller =
                                    controller.context("WebView2 returned no controller")?;
                                let webview = unsafe { controller.CoreWebView2()? };
                                let owned = Controller {
                                    controller,
                                    webview,
                                    _environment: callback_environment,
                                };
                                let Some(target) = weak_controller.upgrade() else {
                                    return Ok(());
                                };
                                unsafe {
                                    owned.controller.SetIsVisible(false)?;
                                }
                                register_events(&owned, &controller_sender)?;
                                *target.borrow_mut() = Some(owned);
                                send(&controller_sender, NativeEvent::Ready);
                                Ok(())
                            })();
                            if let Err(error) = result {
                                send(
                                    &controller_sender,
                                    NativeEvent::Failed(format!("{error:#}")),
                                );
                            }
                            Ok(())
                        },
                    ));
                    unsafe {
                        environment.CreateCoreWebView2Controller(parent, &handler)?;
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    send(
                        &environment_sender,
                        NativeEvent::Failed(format!("{error:#}")),
                    );
                }
                Ok(())
            },
        ));
        // GPUI initializes OLE on its foreground thread. Never pump a nested message loop here:
        // completion handlers must run through GPUI's existing STA loop.
        unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                PCWSTR::null(),
                PCWSTR(user_data.as_ptr()),
                None::<&ICoreWebView2EnvironmentOptions>,
                &environment_handler,
            )
            .context("Starting system WebView2; install or repair Microsoft WebView2 Runtime if unavailable")?;
        }
        Ok((
            Self {
                controller,
                parent: RefCell::new(parent),
            },
            receiver,
        ))
    }

    fn with_controller<T>(&self, operation: impl FnOnce(&Controller) -> Result<T>) -> Result<T> {
        let controller = self.controller.borrow();
        operation(controller.as_ref().context("Browser is still starting")?)
    }

    pub fn navigate(&self, url: &str) -> Result<()> {
        let encoded: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        self.with_controller(|controller| {
            unsafe {
                controller.webview.Navigate(PCWSTR(encoded.as_ptr()))?;
            }
            Ok(())
        })
    }
    pub fn reload(&self) -> Result<()> {
        self.with_controller(|controller| Ok(unsafe { controller.webview.Reload()? }))
    }
    pub fn back(&self) -> Result<()> {
        self.with_controller(|controller| Ok(unsafe { controller.webview.GoBack()? }))
    }
    pub fn forward(&self) -> Result<()> {
        self.with_controller(|controller| Ok(unsafe { controller.webview.GoForward()? }))
    }
    pub fn focus(&self) -> Result<()> {
        self.with_controller(|controller| {
            unsafe {
                controller
                    .controller
                    .MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC)?;
            }
            Ok(())
        })
    }
    pub fn return_focus_to_parent(&self) -> Result<()> {
        let parent = *self.parent.borrow();
        unsafe {
            let focused = GetFocus();
            if IsChild(parent, focused).as_bool() {
                SetFocus(Some(parent))?;
            }
        }
        Ok(())
    }
    pub fn set_visible(&self, visible: bool) -> Result<()> {
        if let Some(controller) = self.controller.borrow().as_ref() {
            if !visible {
                self.return_focus_to_parent()?;
            }
            unsafe {
                controller.controller.SetIsVisible(visible)?;
            }
        }
        Ok(())
    }
    pub fn set_bounds(&self, window: &Window, bounds: Bounds<Pixels>, visible: bool) -> Result<()> {
        let parent = hwnd(window)?;
        if let Some(controller) = self.controller.borrow().as_ref() {
            if parent != *self.parent.borrow() {
                unsafe {
                    controller.controller.SetParentWindow(parent)?;
                }
                *self.parent.borrow_mut() = parent;
            }
            let scale = window.scale_factor();
            let left = (f32::from(bounds.origin.x) * scale).round() as i32;
            let top = (f32::from(bounds.origin.y) * scale).round() as i32;
            let right = (f32::from(bounds.right()) * scale).round() as i32;
            let bottom = (f32::from(bounds.bottom()) * scale).round() as i32;
            if !visible {
                self.return_focus_to_parent()?;
            }
            unsafe {
                let scaled: ICoreWebView2Controller3 = controller.controller.cast()?;
                scaled.SetBoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS)?;
                scaled.SetShouldDetectMonitorScaleChanges(false)?;
                scaled.SetRasterizationScale(f64::from(scale))?;
                controller.controller.SetBounds(RECT {
                    left,
                    top,
                    right,
                    bottom,
                })?;
                controller.controller.NotifyParentWindowPositionChanged()?;
                controller
                    .controller
                    .SetIsVisible(visible && right > left && bottom > top)?;
            }
        }
        Ok(())
    }

    pub fn capture_png(&self) -> Result<oneshot::Receiver<Result<NativeCapture>>> {
        self.with_controller(|controller| {
            let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true)? };
            let capture_stream = stream.clone();
            let webview = controller.webview.clone();
            let source = source(&webview)?;
            let (sender, receiver) = oneshot::channel();
            let callback = CapturePreviewCompletedHandler::create(Box::new(move |result| {
                let result = (|| -> Result<NativeCapture> {
                    result.context("WebView2 page-only CapturePreview")?;
                    if source != self::source(&webview)? {
                        bail!("The page navigated while capturing; capture again");
                    }
                    let mut statistics = STATSTG::default();
                    unsafe {
                        capture_stream.Stat(&mut statistics, STATFLAG_NONAME)?;
                        capture_stream.Seek(0, STREAM_SEEK_SET, None)?;
                    }
                    let length = usize::try_from(statistics.cbSize)?;
                    if length == 0 || length > 50 * 1024 * 1024 {
                        bail!("Screenshot is empty or exceeds 50 MiB");
                    }
                    let mut bytes = vec![0; length];
                    let mut read = 0;
                    unsafe {
                        capture_stream
                            .Read(
                                bytes.as_mut_ptr().cast(),
                                u32::try_from(length)?,
                                Some(&mut read),
                            )
                            .ok()?;
                    }
                    if read as usize != length {
                        bail!("WebView2 returned a truncated screenshot stream");
                    }
                    Ok(NativeCapture {
                        png: bytes,
                        page_url: source,
                    })
                })();
                if sender.send(result).is_err() {
                    log::debug!("Screenshot receiver was cancelled");
                }
                Ok(())
            }));
            unsafe {
                controller.webview.CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &callback,
                )?;
            }
            Ok(receiver)
        })
    }
}

fn hwnd(window: &Window) -> Result<HWND> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow!("Getting the native window handle: {error}"))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(HWND(handle.hwnd.get() as *mut _)),
        _ => Err(anyhow!("Embedded preview needs a native Win32 window")),
    }
}

fn send(sender: &Sender<NativeEvent>, event: NativeEvent) {
    if let Err(error) = sender.try_send(event) {
        log::debug!("WebView2 event receiver closed: {error}");
    }
}

fn source(webview: &ICoreWebView2) -> Result<String> {
    let mut value = PWSTR::null();
    unsafe {
        webview.Source(&mut value)?;
    }
    Ok(take_pwstr(value))
}

fn history(webview: &ICoreWebView2, sender: &Sender<NativeEvent>) -> Result<()> {
    let mut back = BOOL::default();
    let mut forward = BOOL::default();
    unsafe {
        webview.CanGoBack(&mut back)?;
        webview.CanGoForward(&mut forward)?;
    }
    let url = source(webview)?;
    if url == "about:blank" {
        return Ok(());
    }
    send(
        sender,
        NativeEvent::History {
            url,
            back: back.as_bool(),
            forward: forward.as_bool(),
        },
    );
    Ok(())
}

fn register_events(controller: &Controller, sender: &Sender<NativeEvent>) -> Result<()> {
    let webview = &controller.webview;
    let mut token = 0;
    unsafe {
        let settings = webview.Settings()?;
        settings.SetIsStatusBarEnabled(false)?;
        let settings: ICoreWebView2Settings3 = settings.cast()?;
        settings.SetAreBrowserAcceleratorKeysEnabled(true)?;
        webview.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new({
                let sender = sender.clone();
                move |_, arguments| {
                    if let Some(arguments) = arguments {
                        let mut value = PWSTR::null();
                        arguments.Uri(&mut value)?;
                        let url = take_pwstr(value);
                        match crate::normalize_url(&url) {
                            Ok(_) => send(&sender, NativeEvent::Loading),
                            Err(error) => {
                                arguments.SetCancel(true)?;
                                send(&sender, NativeEvent::Failed(format!("{error:#}")));
                            }
                        }
                    }
                    Ok(())
                }
            })),
            &mut token,
        )?;
        webview.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new({
                let sender = sender.clone();
                move |webview, arguments| {
                    if let Some(arguments) = arguments {
                        let mut success = BOOL::default();
                        arguments.IsSuccess(&mut success)?;
                        if success.as_bool() {
                            send(&sender, NativeEvent::Loaded);
                        } else {
                            let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                            arguments.WebErrorStatus(&mut status)?;
                            send(
                                &sender,
                                NativeEvent::Failed(format!(
                                    "Page navigation failed (WebView2 status {})",
                                    status.0
                                )),
                            );
                        }
                    }
                    if let Some(webview) = webview {
                        if let Err(error) = history(&webview, &sender) {
                            send(&sender, NativeEvent::Failed(format!("{error:#}")));
                        }
                    }
                    Ok(())
                }
            })),
            &mut token,
        )?;
        webview.add_SourceChanged(
            &SourceChangedEventHandler::create(Box::new({
                let sender = sender.clone();
                move |webview, _| {
                    if let Some(webview) = webview {
                        if let Err(error) = history(&webview, &sender) {
                            send(&sender, NativeEvent::Failed(format!("{error:#}")));
                        }
                    }
                    Ok(())
                }
            })),
            &mut token,
        )?;
        webview.add_HistoryChanged(
            &HistoryChangedEventHandler::create(Box::new({
                let sender = sender.clone();
                move |webview, _| {
                    if let Some(webview) = webview {
                        if let Err(error) = history(&webview, &sender) {
                            send(&sender, NativeEvent::Failed(format!("{error:#}")));
                        }
                    }
                    Ok(())
                }
            })),
            &mut token,
        )?;
        webview.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new({
                let sender = sender.clone();
                move |_, _| {
                    send(
                        &sender,
                        NativeEvent::Failed(
                            "The embedded browser process exited; reload to restart".into(),
                        ),
                    );
                    Ok(())
                }
            })),
            &mut token,
        )?;
        webview.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new({
                let sender = sender.clone();
                move |webview, arguments| {
                    if let (Some(webview), Some(arguments)) = (webview, arguments) {
                        arguments.SetHandled(true)?;
                        let mut value = PWSTR::null();
                        arguments.Uri(&mut value)?;
                        let url = take_pwstr(value);
                        match crate::normalize_url(&url) {
                            Ok(url) => {
                                let encoded: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
                                webview.Navigate(PCWSTR(encoded.as_ptr()))?;
                            }
                            Err(error) => send(&sender, NativeEvent::Failed(format!("{error:#}"))),
                        }
                    }
                    Ok(())
                }
            })),
            &mut token,
        )?;
        controller.controller.add_GotFocus(
            &FocusChangedEventHandler::create(Box::new({
                let sender = sender.clone();
                move |_, _| {
                    send(&sender, NativeEvent::Focused);
                    Ok(())
                }
            })),
            &mut token,
        )?;
        controller.controller.add_MoveFocusRequested(
            &MoveFocusRequestedEventHandler::create(Box::new({
                let sender = sender.clone();
                move |_, arguments| {
                    if let Some(arguments) = arguments {
                        arguments.SetHandled(true)?;
                    }
                    send(&sender, NativeEvent::FocusAddress);
                    Ok(())
                }
            })),
            &mut token,
        )?;
    }
    Ok(())
}
