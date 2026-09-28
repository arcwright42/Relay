use crate::placement::Rect;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSPanel, NSScreen};
use objc2_foundation::{NSPoint, NSRect, NSSize};

#[derive(Debug)]
pub struct PanelPlacement {
    pub display_id: u32,
    /// Coordinates relative to this display, as expected by GPUI's macOS window constructor.
    pub left: f32,
    pub top: f32,
}

pub const QUICK_PANEL_TITLE: &str = "Relay Quick";

fn top_left_rect(rect: objc2_foundation::NSRect, primary_height: f64) -> Rect {
    Rect {
        x: rect.origin.x,
        y: primary_height - rect.origin.y - rect.size.height,
        width: rect.size.width,
        height: rect.size.height,
    }
}

/// Choose the pointer's screen, excluding its menu bar and Dock.
/// AppKit and Quartz share logical units but have opposite vertical axes.
pub fn quick_origin(pointer: (f32, f32), width: f32, height: f32) -> Option<PanelPlacement> {
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    let primary_height = screens.firstObject()?.frame().size.height;
    for screen in screens {
        let frame = top_left_rect(screen.frame(), primary_height);
        let (x, y) = (pointer.0 as f64, pointer.1 as f64);
        if x >= frame.x && x < frame.x + frame.width && y >= frame.y && y < frame.y + frame.height {
            let usable = top_left_rect(screen.visibleFrame(), primary_height);
            let placed = Rect::near_pointer((x, y), width as f64, height as f64, usable);
            let local = placed.relative_to(frame);
            return Some(PanelPlacement {
                display_id: crate::macos::display_at_position(pointer.0, pointer.1)?,
                left: local.x as f32,
                top: local.y as f32,
            });
        }
    }
    None
}

/// Resize the owned quick panel and constrain its actual frame on its current
/// screen. Do not re-read the mouse: it may already be on a different display.
pub fn resize_quick_panel(width: f32, height: f32) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let screens = NSScreen::screens(mtm);
    let Some(primary) = screens.firstObject() else {
        return;
    };
    let primary_height = primary.frame().size.height;
    for window in NSApplication::sharedApplication(mtm).windows() {
        if window.title().to_string() != QUICK_PANEL_TITLE {
            continue;
        }
        let Some(screen) = window.screen() else {
            continue;
        };
        let original = top_left_rect(window.frame(), primary_height);
        let usable = top_left_rect(screen.visibleFrame(), primary_height);
        let size = Rect {
            width: width as f64,
            height: height as f64,
            ..original
        }
        .fit(usable);
        window.setContentSize(NSSize::new(size.width, size.height));
        let actual = top_left_rect(window.frame(), primary_height);
        let placed = Rect {
            x: original.x,
            y: original.y,
            ..actual
        }
        .fit(usable);
        window.setFrame_display(
            NSRect::new(
                NSPoint::new(placed.x, primary_height - placed.y - placed.height),
                NSSize::new(placed.width, placed.height),
            ),
            true,
        );
    }
}

/// Configure a nonactivating NSPanel as an overlay across applications/spaces.
pub fn configure_quick_panel() {
    use objc2_app_kit::NSWindowCollectionBehavior;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    for window in NSApplication::sharedApplication(mtm).windows().iter() {
        if window.title().to_string() == QUICK_PANEL_TITLE {
            // GPUI PopUp supplies NonactivatingPanel at construction, but its
            // default level (101) is too high for input-method candidate windows.
            window.setLevel(3); // NSFloatingWindowLevel
            if let Some(panel) = window.downcast_ref::<NSPanel>() {
                panel.setFloatingPanel(true);
            }
            window.setHidesOnDeactivate(false);
            window.setCollectionBehavior(
                NSWindowCollectionBehavior::CanJoinAllSpaces
                    | NSWindowCollectionBehavior::FullScreenAuxiliary
                    | NSWindowCollectionBehavior::CanJoinAllApplications
                    | NSWindowCollectionBehavior::Transient
                    | NSWindowCollectionBehavior::IgnoresCycle,
            );
        }
    }
}

/// A global shortcut is invoked while another app is active. GPUI's ordinary
/// orderFront does not guarantee front ordering in that case; do this only
/// after the root view, placement and collection behavior have been configured.
pub fn show_quick_panel() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    for window in NSApplication::sharedApplication(mtm).windows().iter() {
        if window.title().to_string() == QUICK_PANEL_TITLE {
            window.orderFrontRegardless();
            eprintln!(
                "quick: native panel number={} visible={} level={} frame={:?} active_space={} occlusion={:?} style={:?}",
                window.windowNumber(),
                window.isVisible(),
                window.level(),
                window.frame(),
                window.isOnActiveSpace(),
                window.occlusionState(),
                window.styleMask()
            );
        }
    }
}
