//! IdleTrigger i18n: locale loading and string lookup.
//!
//! Compiles `en.json` and `zh-CN.json`. Missing keys fall back to the key name;
//! the caller passes a resolved language.

#[cfg(test)]
use std::collections::HashMap;

#[cfg(test)]
const EN: &str = include_str!("../locales/en.json");
#[cfg(test)]
const ZH_CN: &str = include_str!("../locales/zh-CN.json");

const TEXT: &str = include_str!(concat!(env!("OUT_DIR"), "/locales.txt"));
include!(concat!(env!("OUT_DIR"), "/locales.rs"));

pub struct I18n {
    language: usize,
}

impl I18n {
    /// `lang` must already be resolved to `"en"` or `"zh-CN"`.
    pub fn load(lang: &str) -> I18n {
        I18n {
            language: if lang == "zh-CN" { 2 } else { 1 },
        }
    }

    /// Looks up a key; missing keys return the key itself.
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        ENTRIES
            .binary_search_by(|entry| TEXT[entry[0] as usize..entry[1] as usize].cmp(key))
            .map(|index| {
                let entry = ENTRIES[index];
                &TEXT[entry[self.language] as usize..entry[self.language + 1] as usize]
            })
            .unwrap_or(key)
    }
}

#[cfg(test)]
fn parse_flat(text: &str) -> Option<(HashMap<String, String>, usize)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    let mut map = HashMap::with_capacity(object.len());
    let mut non_strings = 0usize;
    for (key, val) in object {
        if let Some(text) = val.as_str() {
            map.insert(key.clone(), text.to_string());
        } else {
            // Locale files are flat string maps; anything else would have
            // its keys silently fall back to the key name at lookup time.
            non_strings += 1;
        }
    }
    Some((map, non_strings))
}

/// Substitute the locale's ordered string/integer slots in one pass. Values
/// are never parsed again, even when a rule name contains `%s` or `%d`.
pub fn format(template: &str, arguments: &[&str]) -> String {
    let mut output = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    let mut arguments = arguments.iter();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            match chars.peek().copied() {
                Some('s' | 'd') => {
                    let kind = chars.next().unwrap();
                    if let Some(value) = arguments.next() {
                        output.push_str(value);
                    } else {
                        output.push('%');
                        output.push(kind);
                    }
                    continue;
                }
                Some('%') => {
                    chars.next();
                    output.push('%');
                    continue;
                }
                _ => {}
            }
        }
        output.push(ch);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn substitutions_preserve_literal_percent_in_arguments() {
        assert_eq!(
            format("%s: %s / %d / %%", &["name %s", "动作 %d", "3"]),
            "name %s: 动作 %d / 3 / %"
        );
    }
    #[test]
    fn locale_keys_and_placeholder_order_match() {
        let (en, en_non_strings) = parse_flat(EN).unwrap();
        let (zh, zh_non_strings) = parse_flat(ZH_CN).unwrap();
        assert_eq!(en_non_strings, 0, "en.json must be a flat string map");
        assert_eq!(zh_non_strings, 0, "zh-CN.json must be a flat string map");
        assert_eq!(en.len(), zh.len());
        assert_eq!(ENTRIES.len(), en.len());
        let compiled_en = I18n::load("en");
        let compiled_zh = I18n::load("zh-CN");
        let fallback = I18n::load("unknown");
        assert_eq!(compiled_zh.t("missing-locale-key"), "missing-locale-key");
        let placeholders = |text: &str| {
            text.as_bytes()
                .windows(2)
                .filter(|w| w[0] == b'%' && matches!(w[1], b's' | b'd'))
                .map(|w| w[1])
                .collect::<Vec<_>>()
        };
        for (key, value) in en {
            assert_eq!(fallback.t(&key), value, "{key}");
            assert_eq!(compiled_en.t(&key), value, "{key}");
            assert_eq!(compiled_zh.t(&key), zh[&key], "{key}");
            assert_eq!(
                placeholders(&value),
                placeholders(zh.get(&key).unwrap()),
                "{key}"
            );
        }
    }
}
