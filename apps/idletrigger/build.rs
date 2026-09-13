//! Embeds the shared Windows manifest (comctl v6 + PerMonitorV2 DPI), the app
//! icon, and Windows version resources.

mod build_version;
use build_version::version_parts;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }
    // Release builds pass the tag-derived version; local builds fall back to
    // the crate version.
    let version = std::env::var("IDLETRIGGER_VERSION")
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../build/windows/icons/app.ico");
    // Theme-specific tray icon resource IDs (3 = dark
    // strokes for light mode, 4 = light strokes for dark mode).
    resource.set_icon_with_id("../../build/windows/icons/tray-dark.ico", "3");
    resource.set_icon_with_id("../../build/windows/icons/tray-light.ico", "4");
    resource.set_manifest_file("../../build/windows/manifest.xml");

    // Keep the original filename specific to the shipped architecture.
    let original_filename = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "IdleTrigger-x86.exe",
        _ => "IdleTrigger-x64.exe",
    };
    let numeric = numeric_version(&version);
    resource.set("CompanyName", "JeffioZ");
    resource.set("FileDescription", "IdleTrigger");
    resource.set("FileVersion", &numeric);
    resource.set("InternalName", "IdleTrigger");
    resource.set("LegalCopyright", "Copyright (C) 2026 JeffioZ");
    resource.set("OriginalFilename", original_filename);
    resource.set("ProductName", "IdleTrigger");
    resource.set("ProductVersion", &version);
    // Fixed numeric FILEVERSION/PRODUCTVERSION exclude SemVer labels/metadata.
    let parts = version_parts(&version);
    let packed = (u64::from(parts[0]) << 48)
        | (u64::from(parts[1]) << 32)
        | (u64::from(parts[2]) << 16)
        | u64::from(parts[3]);
    resource.set_version_info(winresource::VersionInfo::FILEVERSION, packed);
    resource.set_version_info(winresource::VersionInfo::PRODUCTVERSION, packed);

    if let Err(err) = resource.compile() {
        panic!("winresource failed: {err}");
    }
    println!("cargo:rerun-if-env-changed=IDLETRIGGER_VERSION");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_version.rs");
    println!("cargo:rerun-if-changed=../../build/windows/manifest.xml");
    println!("cargo:rerun-if-changed=../../build/windows/icons/app.ico");
    println!("cargo:rerun-if-changed=../../build/windows/icons/tray-dark.ico");
    println!("cargo:rerun-if-changed=../../build/windows/icons/tray-light.ico");
}

/// "1.2.3-beta" → "1.2.3.0".
fn numeric_version(version: &str) -> String {
    let parts = version_parts(version);
    format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3])
}
