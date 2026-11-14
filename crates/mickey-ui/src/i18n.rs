//! Translations for `mickey-ui`. The strings themselves live in
//! `mickey-config`'s `locales/` catalogues; here the settings window sets
//! the language from `config.toml` once at start-up, so every window the
//! owner opens shows the chosen language.
//!
//! A window that changes the language calls [`apply`] to switch at once,
//! without a restart: the next frame reads the new language through
//! [`t`] and [`tf`].

use mickey_config::i18n::{self, Lang};

/// Sets the language from `config.toml`'s `[interface] language`, or the
/// system's when it is `auto`. Called once at start-up; the settings window
/// also calls it after the owner picks a language, to apply it at once.
pub fn apply(language: &str) {
    let lang = if language == "auto" {
        Lang::system()
    } else {
        Lang::from_code(language).unwrap_or(Lang::En)
    };
    i18n::set_language(lang);
}

/// The text for `key` in the current language; English when this language
/// lacks it. A thin wrapper so the call sites stay short.
pub fn t(key: &'static str) -> &'static str {
    i18n::t(key)
}

/// Like `t`, with each `{name}` in the text replaced by its value.
pub fn tf(key: &'static str, values: &[(&str, &str)]) -> String {
    i18n::tf(key, values)
}
