//! Embeds the shared Windows manifest (comctl v6 + PerMonitorV2 DPI), the app
//! icon, and the Go-parity version resource (resourcegen.go field set).

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return;
    }
    // Release builds pass the tag-derived version; local builds fall back to
    // the crate version (Go: resourcegen -version).
    let version = std::env::var("IDLETRIGGER_VERSION")
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string());

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon("../../build/windows/icons/app.ico");
    // Theme-specific tray icons under Go's resourceid numbers (3 = dark
    // strokes for light mode, 4 = light strokes for dark mode).
    resource.set_icon_with_id("../../build/windows/icons/tray-dark.ico", "3");
    resource.set_icon_with_id("../../build/windows/icons/tray-light.ico", "4");
    resource.set_manifest_file("../../build/windows/manifest.xml");

    // String fields exactly as the Go resourcegen writes them.
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
    // Fixed numeric FILEVERSION/PRODUCTVERSION (Go versionParts).
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
    println!("cargo:rerun-if-changed=../../build/windows/manifest.xml");
    println!("cargo:rerun-if-changed=../../build/windows/icons/app.ico");
    println!("cargo:rerun-if-changed=../../build/windows/icons/tray-dark.ico");
    println!("cargo:rerun-if-changed=../../build/windows/icons/tray-light.ico");
}

/// "1.2.3-beta" → "1.2.3.0" (Go numericVersion).
fn numeric_version(version: &str) -> String {
    let parts = version_parts(version);
    format!("{}.{}.{}.{}", parts[0], parts[1], parts[2], parts[3])
}

/// First four digit groups, missing components zero (Go versionParts).
fn version_parts(version: &str) -> [u16; 4] {
    let mut parts = [0u16; 4];
    for (slot, group) in version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|g| !g.is_empty())
        .take(4)
        .enumerate()
    {
        parts[slot] = group.parse().unwrap_or(0);
    }
    parts
}
