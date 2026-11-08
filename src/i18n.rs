use std::sync::LazyLock;

// The same catalogs are embedded into the desktop binary and served to the browser.
pub const LOCALES: &[(&str, &str)] = &[
    ("de", include_str!("../locales/de.json")),
    ("en", include_str!("../locales/en.json")),
    ("es", include_str!("../locales/es.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("it", include_str!("../locales/it.json")),
    ("ja", include_str!("../locales/ja.json")),
    ("ko", include_str!("../locales/ko.json")),
    ("pl", include_str!("../locales/pl.json")),
    ("pt", include_str!("../locales/pt.json")),
    ("ru", include_str!("../locales/ru.json")),
    ("tr", include_str!("../locales/tr.json")),
    ("uk", include_str!("../locales/uk.json")),
    ("zh", include_str!("../locales/zh.json")),
];

pub fn web_bundle() -> &'static str {
    static BUNDLE: LazyLock<String> = LazyLock::new(|| {
        let entries = LOCALES
            .iter()
            .map(|(code, json)| format!("\"{code}\":{json}"))
            .collect::<Vec<_>>()
            .join(",");
        format!("const translations = {{{entries}}};\n")
    });
    &BUNDLE
}

#[cfg(feature = "gui")]
pub fn text(locale: &str, key: &'static str) -> &'static str {
    use std::collections::HashMap;

    static CATALOGS: LazyLock<HashMap<&'static str, HashMap<String, String>>> =
        LazyLock::new(|| {
            LOCALES
                .iter()
                .map(|(code, json)| (*code, serde_json::from_str(json).expect("invalid locale")))
                .collect()
        });
    CATALOGS
        .get(locale)
        .and_then(|catalog| catalog.get(key))
        .or_else(|| CATALOGS["en"].get(key))
        .map(String::as_str)
        .unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn catalogs_are_complete_and_nonempty() {
        let mut expected = None;
        for (code, json) in LOCALES {
            let messages: BTreeMap<String, String> = serde_json::from_str(json).unwrap();
            let keys: BTreeSet<_> = messages.keys().cloned().collect();
            if let Some(expected) = &expected {
                assert_eq!(&keys, expected, "inconsistent keys in {code}");
            } else {
                expected = Some(keys);
            }
            for (key, value) in messages {
                assert!(!value.trim().is_empty(), "empty {code} translation: {key}");
            }
        }
    }

    #[test]
    fn every_locale_file_is_registered() {
        let files: BTreeSet<_> = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/locales"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        let registered: BTreeSet<_> = LOCALES.iter().map(|(code, _)| (*code).to_owned()).collect();
        assert_eq!(files, registered);
    }

    #[cfg(feature = "gui")]
    #[test]
    fn desktop_translation_keys_exist() {
        let messages: BTreeMap<String, String> =
            serde_json::from_str(LOCALES.iter().find(|(code, _)| *code == "en").unwrap().1)
                .unwrap();
        for fragment in include_str!("gui.rs").split(".text(\"").skip(1) {
            let key = fragment.split('"').next().unwrap();
            assert!(
                messages.contains_key(key),
                "unknown desktop translation key: {key}"
            );
        }
    }
}
