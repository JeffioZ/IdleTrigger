//! IdleTrigger core: pure logic shared by the tray app and tests.
//!
//! No Win32 or UI dependencies: `automation` defines rules and runtime state,
//! `config` loads and saves TOML, `rule_document` preserves rule edits and
//! comments, and `i18n` loads the product locale files.

pub mod automation;
pub mod config;
pub mod i18n;
pub mod rule_document;
