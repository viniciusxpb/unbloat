//! Shared, dependency-free localization for the application and Explorer DLL.
use std::{fmt::Display, sync::OnceLock};

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Portuguese,
    English,
}

pub fn from_tag(tag: &str) -> Language {
    match tag
        .split(['-', '_', '.', '@'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "pt" => Language::Portuguese,
        _ => Language::English,
    }
}

pub fn language() -> Language {
    static LANGUAGE: OnceLock<Language> = OnceLock::new();
    *LANGUAGE.get_or_init(|| match std::env::var("UNBLOAT_LANG") {
        Ok(tag) if !tag.trim().is_empty() => from_tag(tag.trim()),
        _ => system_language(),
    })
}

/// The Windows UI language or POSIX locale, ignoring the UNBLOAT_LANG override.
pub fn system_language() -> Language {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetUserDefaultUILanguage() -> u16;
        }
        // LANG_PORTUGUESE, including both Brazil and Portugal.
        if unsafe { GetUserDefaultUILanguage() } & 0x3ff == 0x16 {
            Language::Portuguese
        } else {
            Language::English
        }
    }
    #[cfg(not(windows))]
    {
        ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|tag| !tag.is_empty())
            .map_or(Language::English, |tag| from_tag(&tag))
    }
}

fn catalog(language: Language) -> &'static [(&'static str, &'static str)] {
    match language {
        Language::Portuguese => PT,
        Language::English => EN,
    }
}

pub fn text(key: &str) -> &'static str {
    text_for(language(), key)
}

/// Text in a specific language, falling back to English like `text`.
pub fn text_for(language: Language, key: &str) -> &'static str {
    fn lookup(language: Language, key: &str) -> Option<&'static str> {
        let entries = catalog(language);
        entries
            .binary_search_by_key(&key, |&(key, _)| key)
            .ok()
            .map(|index| entries[index].1)
    }
    lookup(language, key)
        .or_else(|| lookup(Language::English, key))
        .expect("Unknown localization key")
}

/// Substitute positional placeholders once; inserted paths are never interpreted.
pub fn render(template: &str, args: &[&dyn Display]) -> String {
    let mut result = String::new();
    for part in template.split_inclusive('}') {
        if let Some((prefix, suffix)) = part.rsplit_once('{')
            && let Some(index) = suffix
                .strip_suffix('}')
                .and_then(|s| s.parse::<usize>().ok())
            && let Some(value) = args.get(index)
        {
            use std::fmt::Write;
            result.push_str(prefix);
            write!(result, "{value}").unwrap();
            continue;
        }
        result.push_str(part);
    }
    result
}

pub fn message(key: &str, args: &[&dyn Display]) -> String {
    render(text(key), args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn languages_have_identical_keys_and_placeholders() {
        let pt: BTreeMap<_, _> = catalog(Language::Portuguese).iter().copied().collect();
        let en: BTreeMap<_, _> = catalog(Language::English).iter().copied().collect();
        assert_eq!(pt.keys().collect::<Vec<_>>(), en.keys().collect::<Vec<_>>());
        fn placeholders(value: &str) -> Vec<usize> {
            let mut result: Vec<_> = value
                .split('}')
                .filter_map(|part| part.rsplit_once('{')?.1.parse::<usize>().ok())
                .collect();
            result.sort();
            result
        }
        for (key, value) in &pt {
            assert!(!value.is_empty(), "Empty translation: {key}");
            assert!(!en[key].is_empty(), "Empty translation: {key}");
            assert_eq!(
                placeholders(value),
                placeholders(en[key]),
                "Placeholder mismatch: {key}"
            );
        }
        assert!(pt["make.heading"].contains('í'));
    }

    #[test]
    fn regional_languages_and_fallback() {
        for tag in ["pt-BR", "pt-PT", "PT_br.UTF-8"] {
            assert_eq!(from_tag(tag), Language::Portuguese);
        }
        for tag in ["en-US", "en-GB", "fr-FR", "", "C"] {
            assert_eq!(from_tag(tag), Language::English);
        }
    }

    #[test]
    fn substitutions_do_not_interpret_user_paths() {
        assert_eq!(
            render("{0}: {1}", &[&"C:\\{1}\\ação", &"done"]),
            "C:\\{1}\\ação: done"
        );
        assert_eq!(render("{1} {0} {1}", &[&7, &"ok"]), "ok 7 ok");
    }
}
