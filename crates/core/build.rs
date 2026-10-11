//! Compile the editable JSON locales into one shared-key, allocation-free table.
use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::PathBuf;

fn locale(name: &str) -> BTreeMap<String, String> {
    let path = format!("locales/{name}.json");
    println!("cargo:rerun-if-changed={path}");
    serde_json::from_slice(&std::fs::read(&path).expect("read locale"))
        .unwrap_or_else(|error| panic!("invalid string map in {path}: {error}"))
}

fn main() {
    let en = locale("en");
    let zh = locale("zh-CN");
    assert!(en.keys().eq(zh.keys()), "locale keys must match");

    // Each record stores the start of key / English / Chinese / next record.
    // Byte offsets avoid three fat pointers and relocations per translation.
    let mut text = String::new();
    let mut index = String::from("const ENTRIES: &[[u32; 4]] = &[\n");
    for (key, english) in &en {
        let start = text.len();
        text.push_str(key);
        let english_start = text.len();
        text.push_str(english);
        let chinese_start = text.len();
        text.push_str(&zh[key]);
        writeln!(
            index,
            "    [{start}, {english_start}, {chinese_start}, {}],",
            text.len()
        )
        .unwrap();
    }
    assert!(
        u32::try_from(text.len()).is_ok(),
        "locale table is too large"
    );
    index.push_str("];\n");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    std::fs::write(output.join("locales.txt"), text).expect("write locale text");
    std::fs::write(output.join("locales.rs"), index).expect("write locale index");
}
