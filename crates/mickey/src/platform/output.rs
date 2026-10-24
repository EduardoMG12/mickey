//! The default sound output's mute, on Windows (PRD 0012): *Send only*
//! mutes the speakers while Mickey sends what they play. WASAPI's loopback
//! capture ignores that mute (measured: the same loudness muted or not),
//! so Mickey still records everything.

use std::io;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};

fn error(err: windows::core::Error) -> io::Error {
    io::Error::other(err.to_string())
}

/// The default output's volume control, on this thread.
fn endpoint() -> io::Result<IAudioEndpointVolume> {
    // SAFETY: COM is started for this thread; already started (or in
    // another mode) is fine for what follows, so the result is not needed.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    // SAFETY: the enumerator's class and interface are a valid pair, COM
    // is started, and each returned interface is checked by `?`.
    unsafe {
        let devices: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(error)?;
        devices
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(error)?
            .Activate(CLSCTX_ALL, None)
            .map_err(error)
    }
}

/// Whether the default output is muted.
pub fn muted() -> io::Result<bool> {
    // SAFETY: a valid interface from `endpoint`.
    unsafe { endpoint()?.GetMute() }
        .map(|muted| muted.as_bool())
        .map_err(error)
}

/// Mutes or unmutes the default output.
pub fn set_muted(muted: bool) -> io::Result<()> {
    // SAFETY: a valid interface from `endpoint`; no event context.
    unsafe { endpoint()?.SetMute(muted, std::ptr::null()) }.map_err(error)
}
