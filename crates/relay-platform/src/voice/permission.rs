use block2::RcBlock;
use objc2::runtime::Bool;
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use relay_core::voice::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;

pub(super) fn authorize(
    cancelled: &AtomicBool,
    emit: &dyn Fn(VoiceInputEvent),
) -> Result<bool, VoiceError> {
    // SAFETY: AVMediaTypeAudio is a process-lifetime framework constant; only
    // the documented audio media type is passed to AVCaptureDevice.
    let audio = unsafe { AVMediaTypeAudio }.ok_or_else(|| {
        VoiceError::new(
            VoiceErrorKind::MicrophoneUnavailable,
            "macOS audio media type is unavailable.",
        )
    })?;
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) };
    if status == AVAuthorizationStatus::Authorized {
        return Ok(true);
    }
    if status != AVAuthorizationStatus::NotDetermined {
        return Err(denied());
    }
    emit(VoiceInputEvent::RequestingMicrophone);
    let (sender, receiver) = mpsc::sync_channel(1);
    let completion = RcBlock::new(move |granted: Bool| {
        let _ = sender.try_send(granted.as_bool());
    });
    // SAFETY: The copied block captures an owned channel and macOS retains it
    // for the asynchronous permission callback. It never touches UI state.
    unsafe {
        AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &completion);
    }
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Ok(false);
        }
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(true) => return Ok(true),
            Ok(false) => return Err(denied()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(denied()),
        }
    }
}

fn denied() -> VoiceError {
    VoiceError::new(
        VoiceErrorKind::MicrophoneDenied,
        "Microphone access is denied or restricted by macOS.",
    )
}
