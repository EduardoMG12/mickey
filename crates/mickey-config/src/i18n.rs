//! Translations for what Mickey shows: the settings window, the switcher,
//! the pairing questions and the tray menu.
//!
//! Every text lives in a catalog file under `locales/`, one per language,
//! as `key = "text"` pairs in TOML tables (`[tab]` + `this_pc = ...` is the
//! key `tab.this_pc`). `en.toml` is the reference: it has every key, and a
//! language that lacks one shows the English text. To add a language, add
//! a `Lang` variant, its file, and list it in `Lang::ALL`; the tests then
//! check that it names no unknown key and keeps every `{placeholder}`.
//!
//! Only what the owner reads is translated. Command names, config.toml
//! keys, the network protocol and the control socket's words stay as they
//! are: other programs and other PCs depend on them.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    PtPt,
}

impl Lang {
    /// Every language Mickey is translated into, in the order the
    /// settings list them.
    pub const ALL: [Lang; 2] = [Lang::En, Lang::PtPt];

    /// The code written in config.toml and used in locale names.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::PtPt => "pt-PT",
        }
    }

    /// The language's own name, which is how a language list shows it.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::PtPt => "Português (Portugal)",
        }
    }

    /// A code from config.toml. Case and `_` or `-` do not matter.
    pub fn from_code(code: &str) -> Option<Lang> {
        let wanted = normalize(code);
        Lang::ALL
            .into_iter()
            .find(|lang| normalize(lang.code()) == wanted)
    }

    /// The system's language, from the usual environment variables, or
    /// English. Windows sets none of them, so there the owner picks.
    pub fn system() -> Lang {
        let get = |name: &str| std::env::var(name).ok();
        Lang::from_environment(
            get("LC_ALL").as_deref(),
            get("LC_MESSAGES").as_deref(),
            get("LANGUAGE").as_deref(),
            get("LANG").as_deref(),
        )
    }

    /// POSIX picks the first of `LC_ALL`, `LC_MESSAGES`, `LANG` that is
    /// set; `LANGUAGE` (a preference list, `pt_PT:en`) comes first of all
    /// where it applies. Each candidate that is a language Mickey knows
    /// wins; "C" and "POSIX" mean no preference.
    pub fn from_environment(
        lc_all: Option<&str>,
        lc_messages: Option<&str>,
        language: Option<&str>,
        lang: Option<&str>,
    ) -> Lang {
        // A function rather than a closure: closures that return a
        // reference borrowed from their argument cannot have their
        // lifetimes inferred, and the borrow checker rejects them.
        fn set(value: Option<&str>) -> Option<&str> {
            value.filter(|value| !value.trim().is_empty())
        }
        let locale = set(lc_all).or(set(lc_messages)).or(set(lang));
        // `LANGUAGE` is ignored when the locale is "C" (a GNU rule).
        let list = match locale {
            Some("C" | "POSIX") => None,
            _ => set(language),
        };
        list.into_iter()
            .flat_map(|list| list.split(':'))
            .chain(locale)
            .find_map(Lang::from_locale)
            .unwrap_or(Lang::En)
    }

    /// `pt_PT.UTF-8`, `pt-PT`, `pt_AO@euro`, `en_GB`... Portuguese other
    /// than Brazil's is European Portuguese; `pt_BR` is another dialect,
    /// not translated yet, so it is not matched.
    fn from_locale(locale: &str) -> Option<Lang> {
        let base = locale.split(['.', '@']).next()?;
        let normalized = normalize(base);
        if normalized == "en" || normalized.starts_with("en-") {
            return Some(Lang::En);
        }
        if normalized == "pt" || normalized.starts_with("pt-") {
            return (normalized != "pt-br").then_some(Lang::PtPt);
        }
        None
    }

    fn index(self) -> u8 {
        match self {
            Lang::En => 0,
            Lang::PtPt => 1,
        }
    }

    fn from_index(index: u8) -> Lang {
        Lang::ALL
            .into_iter()
            .find(|lang| lang.index() == index)
            .unwrap_or(Lang::En)
    }

    fn catalog_text(self) -> &'static str {
        match self {
            Lang::En => include_str!("../locales/en.toml"),
            Lang::PtPt => include_str!("../locales/pt-PT.toml"),
        }
    }
}

fn normalize(code: &str) -> String {
    code.trim().replace('_', "-").to_ascii_lowercase()
}

/// The language `t` answers in. A process shows one language at a time;
/// the settings window changes it as the owner picks one.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_language(lang: Lang) {
    CURRENT.store(lang.index(), Ordering::Relaxed);
}

pub fn language() -> Lang {
    Lang::from_index(CURRENT.load(Ordering::Relaxed))
}

type Catalog = HashMap<String, String>;

/// A catalog's text as a flat map: a table's keys get the table's name
/// and a dot in front. A file that does not parse is a bug caught by the
/// tests, so at run time it is an empty catalog, which falls back.
fn load(text: &str) -> Catalog {
    fn walk(prefix: &str, table: &toml::Table, out: &mut Catalog) {
        for (key, value) in table {
            let key = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match value {
                toml::Value::String(text) => {
                    out.insert(key, text.clone());
                }
                toml::Value::Table(table) => walk(&key, table, out),
                _ => {}
            }
        }
    }
    let mut out = Catalog::new();
    if let Ok(table) = text.parse::<toml::Table>() {
        walk("", &table, &mut out);
    }
    out
}

fn catalog(lang: Lang) -> &'static Catalog {
    static CATALOGS: [OnceLock<Catalog>; 2] = [OnceLock::new(), OnceLock::new()];
    CATALOGS[usize::from(lang.index())].get_or_init(|| load(lang.catalog_text()))
}

/// The text for `key` in the current language; English when this language
/// lacks it, and the key itself when English does too (so a typo shows up
/// as what it is).
pub fn t(key: &'static str) -> &'static str {
    lookup(language(), key)
}

fn lookup(lang: Lang, key: &'static str) -> &'static str {
    catalog(lang)
        .get(key)
        .or_else(|| catalog(Lang::En).get(key))
        .map_or(key, String::as_str)
}

/// Like `t`, with each `{name}` in the text replaced by its value.
pub fn tf(key: &'static str, values: &[(&str, &str)]) -> String {
    fill(lookup(language(), key), values)
}

fn fill(text: &str, values: &[(&str, &str)]) -> String {
    let mut out = text.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The `{names}` a text asks for.
    fn placeholders(text: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            found.insert(rest[open + 1..open + close].to_string());
            rest = &rest[open + close + 1..];
        }
        found
    }

    #[test]
    fn every_catalog_parses_and_english_is_complete() {
        for lang in Lang::ALL {
            let text = lang.catalog_text();
            let table: toml::Table = text
                .parse()
                .unwrap_or_else(|err| panic!("{} does not parse: {err}", lang.code()));
            assert!(!table.is_empty(), "{} is empty", lang.code());
        }
        for (key, text) in catalog(Lang::En) {
            assert!(!text.trim().is_empty(), "English `{key}` is empty");
        }
    }

    #[test]
    fn a_translation_names_only_known_keys_and_keeps_every_placeholder() {
        let english = catalog(Lang::En);
        for lang in Lang::ALL {
            for (key, text) in catalog(lang) {
                let reference = english
                    .get(key)
                    .unwrap_or_else(|| panic!("{} has `{key}`, English has not", lang.code()));
                assert_eq!(
                    placeholders(text),
                    placeholders(reference),
                    "{} `{key}` has other {{placeholders}} than English",
                    lang.code()
                );
            }
        }
    }

    #[test]
    fn portuguese_has_every_text_english_has() {
        let missing: Vec<&String> = catalog(Lang::En)
            .keys()
            .filter(|key| !catalog(Lang::PtPt).contains_key(*key))
            .collect();
        assert!(missing.is_empty(), "pt-PT lacks {missing:?}");
    }

    /// Every catalogue call in the code names a text that exists, and every
    /// text is used by something.
    #[test]
    fn the_code_and_the_catalog_agree() {
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut used = BTreeSet::new();
        for member in ["mickey", "mickey-ui", "mickey-config"] {
            visit(&crates.join(member).join("src"), &mut used);
        }
        let english = catalog(Lang::En);
        let unknown: Vec<&String> = used
            .iter()
            .filter(|key| !english.contains_key(*key))
            .collect();
        assert!(unknown.is_empty(), "used but not in en.toml: {unknown:?}");
        let unused: Vec<&String> = english.keys().filter(|key| !used.contains(*key)).collect();
        assert!(unused.is_empty(), "in en.toml but never used: {unused:?}");
    }

    fn visit(dir: &std::path::Path, used: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, used);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                keys_in(&text, used);
            }
        }
    }

    /// The first string after each `t(` or `tf(` that is a call of ours
    /// (not the end of a longer name like `format(`, `set(`).
    fn keys_in(text: &str, used: &mut BTreeSet<String>) {
        for call in ["t(\"", "tf(\"", "i18n::t(\"", "i18n::tf(\""] {
            for (at, _) in text.match_indices(call) {
                let before = text[..at].chars().next_back();
                let part_of_a_name =
                    before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':');
                if call.starts_with("i18n") || !part_of_a_name {
                    let rest = &text[at + call.len()..];
                    if let Some(end) = rest.find('"') {
                        used.insert(rest[..end].to_string());
                    }
                }
            }
        }
    }

    #[test]
    fn a_missing_translation_falls_back_to_english_then_to_the_key() {
        // A key that only English has, by taking one out of Portuguese.
        let mut portuguese = catalog(Lang::PtPt).clone();
        let (key, english) = catalog(Lang::En).iter().next().unwrap();
        portuguese.remove(key);
        assert!(!portuguese.contains_key(key));
        // The real lookup goes through the real catalogs: an unknown key
        // is answered with itself.
        assert_eq!(lookup(Lang::PtPt, "no.such.key"), "no.such.key");
        assert_eq!(lookup(Lang::En, "no.such.key"), "no.such.key");
        assert!(!english.is_empty());
    }

    #[test]
    fn the_language_follows_the_owner_and_the_system() {
        assert_eq!(Lang::from_code("pt-PT"), Some(Lang::PtPt));
        assert_eq!(Lang::from_code("pt_pt"), Some(Lang::PtPt));
        assert_eq!(Lang::from_code("EN"), Some(Lang::En));
        assert_eq!(Lang::from_code("fr"), None);
        let env = Lang::from_environment;
        assert_eq!(env(None, None, None, Some("pt_PT.UTF-8")), Lang::PtPt);
        assert_eq!(env(None, None, None, Some("pt_AO.UTF-8")), Lang::PtPt);
        assert_eq!(env(None, None, None, Some("en_US.UTF-8")), Lang::En);
        assert_eq!(env(None, None, None, Some("fr_FR.UTF-8")), Lang::En);
        assert_eq!(env(None, None, None, None), Lang::En);
        // LC_ALL beats LANG, as in POSIX.
        assert_eq!(env(Some("en_US"), None, None, Some("pt_PT")), Lang::En);
        // LANGUAGE is a preference list: the first known one wins.
        assert_eq!(
            env(None, None, Some("fr:pt_PT:en"), Some("en_US")),
            Lang::PtPt
        );
        // ...unless the locale is C.
        assert_eq!(env(Some("C"), None, Some("pt_PT"), None), Lang::En);
        // Brazilian Portuguese is not translated yet.
        assert_eq!(env(None, None, None, Some("pt_BR.UTF-8")), Lang::En);
        assert_eq!(env(None, None, Some("pt_BR:pt_PT"), None), Lang::PtPt);
    }

    #[test]
    fn placeholders_are_filled() {
        assert_eq!(
            fill("{who} is out; {n} left", &[("who", "Mickey"), ("n", "3")]),
            "Mickey is out; 3 left"
        );
    }

    #[test]
    fn the_current_language_is_switchable() {
        set_language(Lang::PtPt);
        assert_eq!(language(), Lang::PtPt);
        set_language(Lang::En);
        assert_eq!(language(), Lang::En);
    }
}
