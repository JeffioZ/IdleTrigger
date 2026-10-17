//! In-process UI kit for IdleTrigger's form windows: shared control
//! creation primitives, the secondary-window shell, a live-window
//! registry, and role fonts. Scope discipline: this serves this crate
//! only — no generality abstractions, no public-library ambitions.
//! Dependency direction is one-way: windows → kit → paint/theme/choice
//! internals. Kit code never reads window-level globals.

pub mod controls;
pub mod flow;
pub mod fonts;
pub mod form;
pub mod layout;
pub mod registry;
pub mod spec;
