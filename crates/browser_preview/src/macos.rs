//! Browser preview backed by macOS's built-in WebKit. No browser runtime is shipped.
use std::{
    ffi::{CStr, c_void},
    rc::{Rc, Weak},
    sync::OnceLock,
};

use crate::{
    BrowserResources,
    native::{NativeCapture, NativeEvent},
};
use anyhow::{Result, bail};
use async_channel::{Receiver, Sender};
use block::ConcreteBlock;
use cocoa::{
    base::{BOOL, NO, YES, id, nil},
    foundation::{NSPoint, NSRect, NSSize, NSString},
};
use futures::channel::oneshot;
use gpui::{Bounds, Pixels, Window};
use objc::{
    class,
    declare::ClassDecl,
    msg_send,
    runtime::{Class, Object, Sel},
    sel, sel_impl,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[link(name = "WebKit", kind = "framework")]
unsafe extern "C" {}

struct State {
    view: id,
    delegate: id,
    sender: Sender<NativeEvent>,
}

impl Drop for State {
    fn drop(&mut self) {
        // SAFETY: all objects are owned by this main-thread host. Clear WebKit's
        // weak delegate references before releasing either retained object.
        unsafe {
            for key in ["URL", "canGoBack", "canGoForward"] {
                let name = NSString::alloc(nil).init_str(key);
                let _: () = msg_send![self.view, removeObserver: self.delegate forKeyPath: name];
                let _: () = msg_send![name, release];
            }
            let _: () = msg_send![self.view, setNavigationDelegate: nil];
            let _: () = msg_send![self.view, setUIDelegate: nil];
            let _: () = msg_send![self.view, stopLoading];
            let _: () = msg_send![self.view, removeFromSuperview];
            let _: () = msg_send![self.view, release];
            let _: () = msg_send![self.delegate, release];
        }
    }
}

pub struct NativeHost {
    state: Rc<State>,
}

fn parent_view(window: &Window) -> Result<id> {
    match window.window_handle()?.as_raw() {
        RawWindowHandle::AppKit(handle) => Ok(handle.ns_view.as_ptr().cast()),
        _ => bail!("WKWebView requires an AppKit window"),
    }
}

// SAFETY: callers pass a live NSString owned by an Objective-C object or the
// current autorelease pool. Copy the UTF-8 bytes before returning to Rust.
unsafe fn string(value: id) -> String {
    if value == nil {
        return String::new();
    }
    unsafe {
        let bytes: *const std::ffi::c_char = msg_send![value, UTF8String];
        if bytes.is_null() {
            String::new()
        } else {
            CStr::from_ptr(bytes).to_string_lossy().into_owned()
        }
    }
}

unsafe fn delegate_state(delegate: &Object) -> Option<Rc<State>> {
    // SAFETY: only our registered delegate class reaches these callbacks; its
    // ivar contains an owned Weak<State> until delegate_dealloc runs.
    unsafe {
        let pointer = *delegate.get_ivar::<*mut c_void>("state");
        if pointer.is_null() {
            None
        } else {
            (*(pointer.cast::<Weak<State>>())).upgrade()
        }
    }
}

fn send(state: &State, event: NativeEvent) {
    let _ = state.sender.try_send(event);
}

fn history(state: &State) {
    // SAFETY: WKWebView and NSURL are accessed on GPUI's AppKit foreground thread.
    unsafe {
        let url: id = msg_send![state.view, URL];
        let text: id = if url == nil {
            nil
        } else {
            msg_send![url, absoluteString]
        };
        let back: BOOL = msg_send![state.view, canGoBack];
        let forward: BOOL = msg_send![state.view, canGoForward];
        send(
            state,
            NativeEvent::History {
                url: string(text),
                back: back == YES,
                forward: forward == YES,
            },
        );
    }
}

extern "C" fn observed(this: &Object, _: Sel, _: id, _: id, _: id, _: *mut c_void) {
    // SAFETY: WebKit's documented observable URL/history properties are updated
    // on the main thread; this also handles SPA pushState/fragment navigation.
    if let Some(state) = unsafe { delegate_state(this) } {
        history(&state);
    }
}
extern "C" fn started(this: &Object, _: Sel, _: id, _: id) {
    // SAFETY: WebKit invokes the registered delegate on the main thread.
    if let Some(state) = unsafe { delegate_state(this) } {
        send(&state, NativeEvent::Loading);
        history(&state);
    }
}
extern "C" fn finished(this: &Object, _: Sel, _: id, _: id) {
    // SAFETY: WebKit invokes the registered delegate on the main thread.
    if let Some(state) = unsafe { delegate_state(this) } {
        history(&state);
        send(&state, NativeEvent::Loaded);
    }
}
extern "C" fn failed(this: &Object, _: Sel, _: id, _: id, error: id) {
    // SAFETY: the callback's NSError and our delegate are alive for this call.
    unsafe {
        let Some(state) = delegate_state(this) else {
            return;
        };
        if error != nil {
            let code: isize = msg_send![error, code];
            if code == -999 {
                return;
            } // superseded/cancelled navigation is not a page failure
            let description: id = msg_send![error, localizedDescription];
            send(&state, NativeEvent::Failed(string(description)));
        } else {
            send(
                &state,
                NativeEvent::Failed("WebKit navigation failed".into()),
            );
        }
    }
}
extern "C" fn process_terminated(this: &Object, _: Sel, _: id) {
    // SAFETY: WebKit invokes the registered delegate on the main thread.
    if let Some(state) = unsafe { delegate_state(this) } {
        send(
            &state,
            NativeEvent::Failed("WebKit page process ended; reload to retry".into()),
        );
    }
}
extern "C" fn new_window(this: &Object, _: Sel, _: id, _: id, action: id, _: id) -> id {
    // SAFETY: WebKit owns the callback parameters. Reuse the existing page for
    // HTTP(S) target=_blank navigation rather than creating an unmanaged window.
    unsafe {
        if let Some(state) = delegate_state(this) {
            let request: id = msg_send![action, request];
            let url: id = msg_send![request, URL];
            let scheme: id = msg_send![url, scheme];
            if matches!(string(scheme).as_str(), "http" | "https") {
                let _: id = msg_send![state.view, loadRequest: request];
            }
        }
    }
    nil
}
extern "C" fn delegate_dealloc(this: &mut Object, _: Sel) {
    // SAFETY: the ivar was allocated once with Box::into_raw during construction
    // and is released exactly once by NSObject's final deallocation.
    unsafe {
        let pointer = *this.get_ivar::<*mut c_void>("state");
        if !pointer.is_null() {
            drop(Box::from_raw(pointer.cast::<Weak<State>>()));
        }
        let _: () = msg_send![super(this, class!(NSObject)), dealloc];
    }
}
fn delegate_class() -> &'static Class {
    static CLASS: OnceLock<&'static Class> = OnceLock::new();
    CLASS.get_or_init(|| {
        let mut declaration = ClassDecl::new("KnightCodeBrowserPreviewDelegate", class!(NSObject))
            .expect("unique browser delegate class");
        declaration.add_ivar::<*mut c_void>("state");
        for name in ["WKNavigationDelegate", "WKUIDelegate"] {
            declaration
                .add_protocol(objc::runtime::Protocol::get(name).expect("system WebKit protocol"));
        }
        // SAFETY: these signatures exactly match the WebKit delegate selectors.
        unsafe {
            declaration.add_method(
                sel!(observeValueForKeyPath:ofObject:change:context:),
                observed as extern "C" fn(&Object, Sel, id, id, id, *mut c_void),
            );
            declaration.add_method(
                sel!(webView:didStartProvisionalNavigation:),
                started as extern "C" fn(&Object, Sel, id, id),
            );
            declaration.add_method(
                sel!(webView:didFinishNavigation:),
                finished as extern "C" fn(&Object, Sel, id, id),
            );
            declaration.add_method(
                sel!(webView:didFailNavigation:withError:),
                failed as extern "C" fn(&Object, Sel, id, id, id),
            );
            declaration.add_method(
                sel!(webView:didFailProvisionalNavigation:withError:),
                failed as extern "C" fn(&Object, Sel, id, id, id),
            );
            declaration.add_method(
                sel!(webViewWebContentProcessDidTerminate:),
                process_terminated as extern "C" fn(&Object, Sel, id),
            );
            declaration.add_method(
                sel!(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:),
                new_window as extern "C" fn(&Object, Sel, id, id, id, id) -> id,
            );
            declaration.add_method(
                sel!(dealloc),
                delegate_dealloc as extern "C" fn(&mut Object, Sel),
            );
        }
        declaration.register()
    })
}

impl NativeHost {
    pub fn new(window: &Window, _: BrowserResources) -> Result<(Self, Receiver<NativeEvent>)> {
        let parent = parent_view(window)?;
        // SAFETY: GPUI creates/uses this Rc host on AppKit's foreground thread.
        unsafe {
            let main_thread: BOOL = msg_send![class!(NSThread), isMainThread];
            if main_thread != YES {
                bail!("WKWebView must be created on the main thread");
            }
            let configuration: id = msg_send![class!(WKWebViewConfiguration), new];
            let allocated: id = msg_send![class!(WKWebView), alloc];
            let view: id = msg_send![allocated, initWithFrame: NSRect::new(NSPoint::new(0., 0.), NSSize::new(0., 0.)) configuration: configuration];
            let _: () = msg_send![configuration, release];
            if view == nil {
                bail!("macOS could not create WKWebView");
            }
            let delegate: id = msg_send![delegate_class(), new];
            let (sender, receiver) = async_channel::unbounded();
            let state = Rc::new(State {
                view,
                delegate,
                sender,
            });
            let pointer = Box::into_raw(Box::new(Rc::downgrade(&state))).cast::<c_void>();
            (*delegate).set_ivar("state", pointer);
            for key in ["URL", "canGoBack", "canGoForward"] {
                let name = NSString::alloc(nil).init_str(key);
                let _: () = msg_send![view, addObserver: delegate forKeyPath: name options: 0usize context: std::ptr::null_mut::<c_void>()];
                let _: () = msg_send![name, release];
            }
            let _: () = msg_send![view, setNavigationDelegate: delegate];
            let _: () = msg_send![view, setUIDelegate: delegate];
            let _: () = msg_send![view, setHidden: YES];
            let _: () = msg_send![parent, addSubview: view];
            send(&state, NativeEvent::Ready);
            Ok((Self { state }, receiver))
        }
    }
    pub fn navigate(&self, url: &str) -> Result<()> {
        // SAFETY: NSString/NSURLRequest live through the call; WebKit retains
        // the request for asynchronous navigation. Release our owned NSString.
        unsafe {
            let text = NSString::alloc(nil).init_str(url);
            let url: id = msg_send![class!(NSURL), URLWithString: text];
            let _: () = msg_send![text, release];
            if url == nil {
                bail!("Invalid browser URL");
            }
            let request: id = msg_send![class!(NSURLRequest), requestWithURL: url];
            let _: id = msg_send![self.state.view, loadRequest: request];
        }
        Ok(())
    }
    pub fn reload(&self) -> Result<()> {
        // SAFETY: the retained WKWebView is used on its main thread.
        unsafe {
            let _: id = msg_send![self.state.view, reload];
        }
        Ok(())
    }
    pub fn back(&self) -> Result<()> {
        // SAFETY: the retained WKWebView is used on its main thread.
        unsafe {
            let _: id = msg_send![self.state.view, goBack];
        }
        Ok(())
    }
    pub fn forward(&self) -> Result<()> {
        // SAFETY: the retained WKWebView is used on its main thread.
        unsafe {
            let _: id = msg_send![self.state.view, goForward];
        }
        Ok(())
    }
    pub fn focus(&self) -> Result<()> {
        // SAFETY: the WKWebView belongs to the retained AppKit parent window.
        unsafe {
            let window: id = msg_send![self.state.view, window];
            if window == nil {
                bail!("Browser is not attached to a window");
            }
            let focused: BOOL = msg_send![window, makeFirstResponder: self.state.view];
            if focused != YES {
                bail!("Could not focus the browser page");
            }
        }
        send(&self.state, NativeEvent::Focused);
        Ok(())
    }
    pub fn return_focus_to_parent(&self) -> Result<()> {
        // SAFETY: ask AppKit for the current superview; never keep a raw pointer
        // to a parent that could have been released when its window closed.
        unsafe {
            let parent: id = msg_send![self.state.view, superview];
            if parent == nil {
                bail!("Browser is not attached to a window");
            }
            let window: id = msg_send![parent, window];
            let _: BOOL = msg_send![window, makeFirstResponder: parent];
        }
        Ok(())
    }
    pub fn set_visible(&self, visible: bool) -> Result<()> {
        // SAFETY: hiding the native child does not change its retained lifetime.
        unsafe {
            let _: () = msg_send![self.state.view, setHidden: if visible { NO } else { YES }];
        }
        Ok(())
    }
    pub fn set_bounds(&self, window: &Window, bounds: Bounds<Pixels>, visible: bool) -> Result<()> {
        let parent = parent_view(window)?;
        // SAFETY: GPUI bounds are logical points; NSView uses those same points.
        // Reparent before layout and respect the parent's flipped coordinate system.
        unsafe {
            let previous_parent: id = msg_send![self.state.view, superview];
            if parent != previous_parent {
                let _: () = msg_send![self.state.view, removeFromSuperview];
                let _: () = msg_send![parent, addSubview: self.state.view];
            }
            let flipped: BOOL = msg_send![parent, isFlipped];
            let parent_bounds: NSRect = msg_send![parent, bounds];
            let x = f64::from(f32::from(bounds.origin.x));
            let height = f64::from(f32::from(bounds.size.height));
            let y = f64::from(f32::from(bounds.origin.y));
            let y = if flipped == YES {
                y
            } else {
                parent_bounds.size.height - y - height
            };
            let frame = NSRect::new(
                NSPoint::new(x, y),
                NSSize::new(f64::from(f32::from(bounds.size.width)), height),
            );
            let _: () = msg_send![self.state.view, setFrame: frame];
        }
        self.set_visible(visible)
    }
    pub fn capture_png(&self) -> Result<oneshot::Receiver<Result<NativeCapture>>> {
        let (sender, receiver) = oneshot::channel();
        let sender = Rc::new(std::cell::RefCell::new(Some(sender)));
        // SAFETY: WebKit's snapshot callback owns its temporary NSImage. Copy
        // encoded PNG bytes while it is valid; the completion block is copied by WebKit.
        unsafe {
            let url: id = msg_send![self.state.view, URL];
            let url: id = if url == nil {
                nil
            } else {
                msg_send![url, absoluteString]
            };
            let page_url = string(url);
            let callback = ConcreteBlock::new(move |image: id, error: id| {
                let result = (|| -> Result<NativeCapture> {
                    if error != nil {
                        let message: id = msg_send![error, localizedDescription];
                        bail!("WebKit snapshot: {}", string(message));
                    }
                    if image == nil {
                        bail!("WebKit returned no page snapshot");
                    }
                    let tiff: id = msg_send![image, TIFFRepresentation];
                    if tiff == nil {
                        bail!("Cannot encode page snapshot");
                    }
                    let bitmap: id = msg_send![class!(NSBitmapImageRep), imageRepWithData: tiff];
                    if bitmap == nil {
                        bail!("Cannot decode page snapshot");
                    }
                    let properties: id = msg_send![class!(NSDictionary), dictionary];
                    let data: id =
                        msg_send![bitmap, representationUsingType: 4usize properties: properties];
                    if data == nil {
                        bail!("Cannot encode snapshot as PNG");
                    }
                    let length: usize = msg_send![data, length];
                    let bytes: *const u8 = msg_send![data, bytes];
                    if bytes.is_null() || length == 0 {
                        bail!("WebKit returned an empty PNG");
                    }
                    Ok(NativeCapture {
                        png: std::slice::from_raw_parts(bytes, length).to_vec(),
                        page_url: page_url.clone(),
                    })
                })();
                if let Some(sender) = sender.borrow_mut().take() {
                    let _ = sender.send(result);
                }
            })
            .copy();
            let _: () = msg_send![self.state.view, takeSnapshotWithConfiguration: nil completionHandler: &*callback];
        }
        Ok(receiver)
    }
}
