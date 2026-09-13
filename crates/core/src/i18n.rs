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
