//! EasyDict-style local + global mouse monitoring. Never consume the user's click.
use block2::RcBlock;
use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSWindow};
use std::{marker::PhantomData, ptr::NonNull, rc::Rc};

/// One monitor pair per quick window, released when that window closes/replaces.
/// Callbacks only enqueue; GPUI removes its window after native event delivery.
pub struct QuickDismissMonitor {
    local: Option<Retained<AnyObject>>,
    global: Option<Retained<AnyObject>>,
    _main_thread: PhantomData<Rc<()>>,
}
impl QuickDismissMonitor {
    pub fn install() -> Option<(Self, async_channel::Receiver<()>)> {
        let mtm = MainThreadMarker::new()?;
        let window = NSApplication::sharedApplication(mtm)
            .windows()
            .iter()
            .find(|window| {
                window.isVisible() && window.title().to_string() == crate::QUICK_PANEL_TITLE
            })?;
        let number = window.windowNumber();
        let (sender, receiver) = async_channel::bounded(1);
        let mask =
            NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown | NSEventMask::OtherMouseDown;
        let local_sender = sender.clone();
        let local_handler = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
            // SAFETY: AppKit supplies a live event for the duration of this callback.
            let event_ref = unsafe { event.as_ref() };
            if let Some(mtm) = MainThreadMarker::new() {
                let mut target = event_ref.window(mtm);
                let mut inside = false;
                while let Some(window) = target {
                    if window.windowNumber() == number {
                        inside = true;
                        break;
                    }
                    target = window.parentWindow();
                }
                if !inside && clicked_outside(number, mtm) {
                    let _ = local_sender.try_send(());
                }
            }
            // Preserve both the pointer and delivery to the original target.
            event.as_ptr()
        });
        // SAFETY: callback returns the exact event AppKit passed in. The system
        // copies its block; the retained monitor token is removed on Drop.
        let local =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local_handler) };
        let global_handler = RcBlock::new(move |_: NonNull<NSEvent>| {
            if let Some(mtm) = MainThreadMarker::new()
                && clicked_outside(number, mtm)
            {
                let _ = sender.try_send(());
            }
        });
        let global = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &global_handler);
        let monitor = Self {
            local,
            global,
            _main_thread: PhantomData,
        };
        if monitor.local.is_none() || monitor.global.is_none() {
            return None;
        }
        Some((monitor, receiver))
    }
}
fn clicked_outside(number: isize, mtm: MainThreadMarker) -> bool {
    // AppKit hit testing works across Retina and negative-origin monitors;
    // don't compare a Quartz point with GPUI's display-local bounds.
    let hit =
        NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(NSEvent::mouseLocation(), 0, mtm);
    if hit == number {
        return false;
    }
    // Clicking a Pinyin candidate is part of editing this panel, not dismissal.
    let quick_is_key = NSApplication::sharedApplication(mtm)
        .keyWindow()
        .is_some_and(|window| window.windowNumber() == number);
    !(quick_is_key && crate::macos::is_input_method_window(hit))
}
impl Drop for QuickDismissMonitor {
    fn drop(&mut self) {
        for monitor in [&self.local, &self.global].into_iter().flatten() {
            // SAFETY: both tokens came from NSEvent monitor registration and
            // this !Send owner is dropped only on the application's main thread.
            unsafe {
                NSEvent::removeMonitor(monitor);
            }
        }
    }
}
