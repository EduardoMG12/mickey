//! macOS speaker diversion (placeholder until CoreAudio Process Taps are added).

pub struct Diverted;

pub fn divert() -> Result<Diverted, String> {
    Err("shared audio diversion on macOS is not implemented yet".into())
}

pub fn clean_up() -> Result<(), String> {
    Ok(())
}
