//! IdleTrigger i18n: locale loading and string lookup.
//!
//! Reuses the product locale files from the Go version (`en.json`,
//! `zh-CN.json`). Missing keys fall back to the key name so a stale locale
//! file never crashes the app; the caller passes a resolved language.

use std::collections::HashMap;

const EN: &str = include_str!("../locales/en.json");
const ZH_CN: &str = include_str!("../locales/zh-CN.json");

pub struct I18n {
    map: HashMap<String, String>,
}

impl I18n {
    /// `lang` must already be resolved to `"en"` or `"zh-CN"`.
    pub fn load(lang: &str) -> I18n {
        let text = if lang == "zh-CN" { ZH_CN } else { EN };
        let map = parse_flat(text).unwrap_or_default();
        I18n { map }
    }

    /// Looks up a key; missing keys return the key itself.
    pub fn t<'a>(&'a self, key: &'a str) -> &'a str {
        self.map.get(key).map(String::as_str).unwrap_or(key)
    }
}

fn parse_flat(text: &str) -> Option<HashMap<String, String>> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let object = value.as_object()?;
    let mut map = HashMap::with_capacity(object.len());
    for (key, val) in object {
        if let Some(text) = val.as_str() {
            map.insert(key.clone(), text.to_string());
        }
    }
    Some(map)
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
        let en = parse_flat(EN).unwrap();
        let zh = parse_flat(ZH_CN).unwrap();
        assert_eq!(en.len(), zh.len());
        let placeholders = |text: &str| {
            text.as_bytes()
                .windows(2)
                .filter(|w| w[0] == b'%' && matches!(w[1], b's' | b'd'))
                .map(|w| w[1])
                .collect::<Vec<_>>()
        };
        for (key, value) in en {
            assert_eq!(
                placeholders(&value),
                placeholders(zh.get(&key).unwrap()),
                "{key}"
            );
        }
    }
}
