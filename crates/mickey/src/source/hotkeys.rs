//! Hotkeys another PC saved and shared with this one.

use mickey_config as config;

/// Writes `lines`, another PC's hotkeys, into config.toml and returns the
/// config as it is now. Nothing is written unless the new file still reads
/// as valid hotkeys.
pub(super) fn take_hotkeys(lines: &str) -> Result<config::Config, String> {
    let hotkeys = config::Hotkeys::from_lines(lines)?;
    let path = config::path()?;
    let text = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
    let new = config::set_hotkeys(&text, &hotkeys)?;
    config::parse(&new)?.hotkeys()?;
    config::write(&path, &new)?;
    config::Config::load()?.ok_or_else(|| "config.toml is gone".to_string())
}
