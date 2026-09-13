//! IdleTrigger core: pure logic shared by the tray app and tests.
//!
//! No Win32 or UI dependencies here — `automation` is the Go
//! `internal/automation` port, `config` is the Go-compatible TOML
//! round-trip, and `i18n` loads the product locale files.

pub mod automation;
pub mod config;
pub mod i18n;
