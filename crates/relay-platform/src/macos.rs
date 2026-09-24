use relay_core::capture::Selection;
use std::{
    collections::VecDeque,
    ffi::{c_char, c_void},
    marker::PhantomData,
    ptr,
    rc::Rc,
    time::{Duration, Instant},
};

type Ref = *const c_void;
type MutRef = *mut c_void;

#[repr(C)]
struct EventType {
    class: u32,
    kind: u32,
}
#[repr(C)]
struct HotKeyId {
    signature: u32,
    id: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> MutRef;
    fn InstallEventHandler(
        target: MutRef,
        handler: extern "C" fn(MutRef, MutRef, MutRef) -> i32,
        count: u32,
        types: *const EventType,
        data: MutRef,
        result: *mut MutRef,
    ) -> i32;
    fn RegisterEventHotKey(
        code: u32,
        modifiers: u32,
        id: HotKeyId,
        target: MutRef,
        options: u32,
        result: *mut MutRef,
    ) -> i32;
    fn UnregisterEventHotKey(key: MutRef) -> i32;
    fn RemoveEventHandler(handler: MutRef) -> i32;
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: Ref) -> bool;
    static kAXTrustedCheckOptionPrompt: Ref;
    fn AXUIElementCreateSystemWide() -> Ref;
    fn AXUIElementCopyAttributeValue(element: Ref, attribute: Ref, value: *mut Ref) -> i32;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementSetAttributeValue(element: Ref, attribute: Ref, value: Ref) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: Ref,
        attribute: Ref,
        parameter: Ref,
        value: *mut Ref,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: Ref, seconds: f32) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: Ref);
    fn CFRetain(value: Ref) -> Ref;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    fn CFURLGetTypeID() -> usize;
    fn CFURLGetString(url: Ref) -> Ref;
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringCreateWithCString(allocator: Ref, text: *const c_char, encoding: u32) -> Ref;
    fn CFStringGetCString(value: Ref, buffer: *mut c_char, size: isize, encoding: u32) -> bool;
    static kCFBooleanTrue: Ref;
    fn CFDictionaryCreate(
        allocator: Ref,
        keys: *const Ref,
        values: *const Ref,
        count: isize,
        key_callbacks: Ref,
        value_callbacks: Ref,
    ) -> Ref;
}

/// Called only after the user presses the permission control.
pub fn request_accessibility() {
    // SAFETY: both dictionary entries are framework-owned constants. Null callbacks
    // leave their ownership unchanged; Owned releases only the temporary dictionary.
    unsafe {
        let options = CFDictionaryCreate(
            ptr::null(),
            &kAXTrustedCheckOptionPrompt,
            &kCFBooleanTrue,
            1,
            ptr::null(),
            ptr::null(),
        );
        if !options.is_null() {
            let options = Owned(options);
            AXIsProcessTrustedWithOptions(options.0);
        }
    }
}

extern "C" fn hotkey(_: MutRef, _: MutRef, data: MutRef) -> i32 {
    // SAFETY: registration retains this boxed sender until after the handler is removed.
    let sender = unsafe { &*data.cast::<async_channel::Sender<()>>() };
    let _ = sender.try_send(());
    0
}

/// Must be registered and dropped on the application event-loop thread.
pub struct Shortcut {
    key: MutRef,
    handler: MutRef,
    _sender: Box<async_channel::Sender<()>>,
    receiver: async_channel::Receiver<()>,
    _main_thread: PhantomData<Rc<()>>,
}
impl Shortcut {
    /// Control + Option + Space. Carbon reports conflicts instead of stealing a shortcut.
    pub fn register() -> Result<Self, String> {
        let (sender, receiver) = async_channel::bounded(1);
        let mut sender = Box::new(sender);
        let mut handler = ptr::null_mut();
        let mut key = ptr::null_mut();
        // SAFETY: correct Carbon ABI; callback has static lifetime, output pointers are valid.
        unsafe {
            let target = GetApplicationEventTarget();
            let event = EventType {
                class: u32::from_be_bytes(*b"keyb"),
                kind: 5,
            };
            let status = InstallEventHandler(
                target,
                hotkey,
                1,
                &event,
                (&mut *sender as *mut async_channel::Sender<()>).cast(),
                &mut handler,
            );
            if status != 0 {
                return Err(format!("Shortcut handler failed ({status})"));
            }
            let status = RegisterEventHotKey(
                49,
                (1 << 12) | (1 << 11),
                HotKeyId {
                    signature: u32::from_be_bytes(*b"Rely"),
                    id: 1,
                },
                target,
                0,
                &mut key,
            );
            if status != 0 {
                RemoveEventHandler(handler);
                return Err(format!("Control + Option + Space unavailable ({status})"));
            }
        }
        Ok(Self {
            key,
            handler,
            _sender: sender,
            receiver,
            _main_thread: PhantomData,
        })
    }
    pub fn discard_pending(&self) {
        while self.receiver.try_recv().is_ok() {}
    }
    pub async fn next_trigger(&self) -> bool {
        self.receiver.recv().await.is_ok()
    }
}
impl Drop for Shortcut {
    fn drop(&mut self) {
        // SAFETY: handles belong to this object, which cannot leave the main thread.
        unsafe {
            UnregisterEventHotKey(self.key);
            RemoveEventHandler(self.handler);
        }
    }
}

struct Owned(Ref);
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: only created from non-null retained Core Foundation objects.
        unsafe {
            CFRelease(self.0);
        }
    }
}
fn attribute(element: &Owned, name: &std::ffi::CStr) -> Option<Owned> {
    // SAFETY: all pointers are valid retained AX/CF objects; copied values are released by Owned.
    unsafe {
        let key = CFStringCreateWithCString(ptr::null(), name.as_ptr(), 0x08000100);
        if key.is_null() {
            return None;
        }
        let key = Owned(key);
        let mut value = ptr::null();
        let status = AXUIElementCopyAttributeValue(element.0, key.0, &mut value);
        if status == 0 && !value.is_null() {
            Some(Owned(value))
        } else {
            None
        }
    }
}
fn string(value: Owned) -> Option<String> {
    // SAFETY: check the CF type before using CFString APIs; output is bounded and NUL terminated.
    unsafe {
        let raw = if CFGetTypeID(value.0) == CFURLGetTypeID() {
            CFURLGetString(value.0)
        } else {
            value.0
        };
        if raw.is_null() || CFGetTypeID(raw) != CFStringGetTypeID() {
            return None;
        }
        let mut bytes = vec![0_u8; 128 * 1024];
        if !CFStringGetCString(
            raw,
            bytes.as_mut_ptr().cast(),
            bytes.len() as isize,
            0x08000100,
        ) {
            return None;
        }
        let end = bytes.iter().position(|b| *b == 0)?;
        String::from_utf8(bytes[..end].to_vec()).ok()
    }
}

// Browser selections may belong to a web area rather than the focused leaf.
// WebKit exposes text-marker ranges; Chromium can expose the same interface.
fn selected_text(element: &Owned) -> Option<String> {
    if attribute(element, c"AXSubrole").and_then(string).as_deref() == Some("AXSecureTextField") {
        return None;
    }
    if let Some(text) = attribute(element, c"AXSelectedText")
        .and_then(string)
        .filter(|s| !s.trim().is_empty())
    {
        return Some(text);
    }
    let range = attribute(element, c"AXSelectedTextMarkerRange")?;
    // SAFETY: retained AX element and marker, valid CFString key and output pointer.
    unsafe {
        let key = CFStringCreateWithCString(
            ptr::null(),
            c"AXStringForTextMarkerRange".as_ptr(),
            0x08000100,
        );
        if key.is_null() {
            return None;
        }
        let key = Owned(key);
        let mut value = ptr::null();
        if AXUIElementCopyParameterizedAttributeValue(element.0, key.0, range.0, &mut value) == 0
            && !value.is_null()
        {
            string(Owned(value)).filter(|s| !s.trim().is_empty())
        } else {
            None
        }
    }
}
fn web_url(element: &Owned) -> Option<String> {
    [c"AXURL", c"AXDocument"].into_iter().find_map(|key| {
        attribute(element, key)
            .and_then(string)
            .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
    })
}
fn children(element: &Owned) -> Vec<Owned> {
    let Some(array) = attribute(element, c"AXChildren") else {
        return vec![];
    };
    // SAFETY: type-check arrays and child elements before using or retaining them.
    unsafe {
        if CFGetTypeID(array.0) != CFArrayGetTypeID() {
            return vec![];
        }
        (0..CFArrayGetCount(array.0).min(128))
            .filter_map(|index| {
                let child = CFArrayGetValueAtIndex(array.0, index);
                (!child.is_null() && CFGetTypeID(child) == AXUIElementGetTypeID())
                    .then(|| Owned(CFRetain(child)))
            })
            .collect()
    }
}
fn browser_selection(window: Owned, deadline: Instant) -> Option<(String, Option<String>)> {
    let mut queue = VecDeque::from([(window, 0, None)]);
    for _ in 0..384 {
        if Instant::now() >= deadline {
            break;
        }
        let Some((element, depth, inherited_url)) = queue.pop_front() else {
            break;
        };
        let url = web_url(&element).or(inherited_url);
        // Never pick a stale selection from an unrelated sidebar or address field.
        if attribute(&element, c"AXRole").and_then(string).as_deref() == Some("AXWebArea")
            && let Some(text) = selected_text(&element)
        {
            return Some((text, url));
        }
        if depth < 12 {
            queue.extend(
                children(&element)
                    .into_iter()
                    .map(|child| (child, depth + 1, url.clone())),
            );
        }
    }
    None
}

/// Run before activating Relay. AX can block, so callers use a background executor.
pub fn capture_selection() -> Selection {
    let mut selection = Selection::default();
    // SAFETY: API has no pointer parameters and is safe to query from a worker.
    if !unsafe { AXIsProcessTrusted() } {
        selection.accessibility_missing = true;
        return selection;
    }
    // SAFETY: retained system-wide element is wrapped immediately; timeout bounds AX messaging.
    let raw = unsafe { AXUIElementCreateSystemWide() };
    if raw.is_null() {
        return selection;
    }
    let system = Owned(raw);
    unsafe {
        AXUIElementSetMessagingTimeout(system.0, 0.2);
    }
    let deadline = Instant::now() + Duration::from_millis(900);
    let app = attribute(&system, c"AXFocusedApplication");
    if let Some(app) = &app {
        selection.application = attribute(app, c"AXTitle").and_then(string);
        // Request the browser's accessibility tree; unsupported apps ignore this attribute.
        // SAFETY: the app and key remain retained through this synchronous AX call.
        unsafe {
            let key = CFStringCreateWithCString(
                ptr::null(),
                c"AXManualAccessibility".as_ptr(),
                0x08000100,
            );
            if !key.is_null() {
                let key = Owned(key);
                AXUIElementSetAttributeValue(app.0, key.0, kCFBooleanTrue);
            }
        }
    }
    let mut focus = attribute(&system, c"AXFocusedUIElement");
    for _ in 0..8 {
        if Instant::now() >= deadline {
            break;
        }
        let Some(element) = focus else {
            break;
        };
        if selection.text.is_empty() {
            selection.text = selected_text(&element).unwrap_or_default();
        }
        if selection.url.is_none() {
            selection.url = web_url(&element);
        }
        focus = attribute(&element, c"AXParent");
    }
    if let Some(app) = app
        && let Some(window) = attribute(&app, c"AXFocusedWindow")
    {
        if selection.url.is_none() {
            selection.url = web_url(&window);
        }
        if selection.text.is_empty()
            && let Some((text, url)) = browser_selection(window, deadline)
        {
            selection.text = text;
            if url.is_some() {
                selection.url = url;
            }
        }
    }
    selection
}

#[repr(C)]
struct Point {
    x: f64,
    y: f64,
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: Ref) -> Ref;
    fn CGEventGetLocation(event: Ref) -> Point;
}

/// Global desktop coordinates, with the origin at the main display's top left.
pub fn pointer_position() -> Option<(f32, f32)> {
    // SAFETY: a null source creates a current event; the retained event is released by Owned.
    unsafe {
        let event = CGEventCreate(ptr::null());
        if event.is_null() {
            return None;
        }
        let event = Owned(event);
        let point = CGEventGetLocation(event.0);
        Some((point.x as f32, point.y as f32))
    }
}
