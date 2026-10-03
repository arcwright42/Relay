//! Keep user-enabled microphone work responsive when every window is hidden or closed.
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_foundation::{NSActivityOptions, NSObjectProtocol, NSProcessInfo, NSString};

/// Owned by the capture thread, independently of any window. Dropping the guard
/// ends the activity on cancellation, capture failure, or application shutdown.
pub(super) struct ListeningActivity(Retained<ProtocolObject<dyn NSObjectProtocol>>);

impl ListeningActivity {
    pub(super) fn begin() -> Self {
        // Prevent App Nap, but allow the Mac to sleep normally. This activity is
        // scoped to an explicitly enabled listener rather than the whole app.
        Self(
            NSProcessInfo::processInfo().beginActivityWithOptions_reason(
                NSActivityOptions::UserInitiatedAllowingIdleSystemSleep,
                &NSString::from_str("Listening for Hey Relay"),
            ),
        )
    }
}

impl Drop for ListeningActivity {
    fn drop(&mut self) {
        // SAFETY: this token came from beginActivityWithOptions:reason: and is
        // ended exactly once by its sole owner, on the same capture thread.
        unsafe { NSProcessInfo::processInfo().endActivity(&self.0) };
    }
}
