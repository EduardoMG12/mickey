//! macOS speaker diversion: CoreAudio Process Taps (`CATapMuted`) mute the
//! local speakers automatically while `Recorder::open(true)` is active, and
//! unmute them automatically when the tap is destroyed or the process exits.

pub struct Diverted;

pub fn divert() -> Result<Diverted, String> {
    Ok(Diverted)
}

pub fn clean_up() -> Result<(), String> {
    Ok(())
}
