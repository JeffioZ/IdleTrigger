/// Windows fixed versions use the numeric core, never SemVer labels/metadata.
pub fn version_parts(version: &str) -> [u16; 4] {
    let core = version
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("");
    let mut parts = [0; 4];
    if !core.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return parts; // Named development builds, e.g. "ci".
    }
    for (index, value) in core.split('.').take(4).enumerate() {
        parts[index] = value
            .parse()
            .expect("Windows version components must be integers in 0..65535");
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn labels_do_not_change_fixed_version() {
        for version in [
            "1.2.3",
            "1.2.3-rc.999999999999",
            "1.2.3+20260913",
            "v1.2.3-beta.2+build.9",
        ] {
            assert_eq!(version_parts(version), [1, 2, 3, 0]);
        }
        assert_eq!(version_parts("65535.0.1"), [65535, 0, 1, 0]);
        assert_eq!(version_parts("ci"), [0; 4]);
    }
    #[test]
    #[should_panic(expected = "Windows version components")]
    fn oversized_core_fails_instead_of_silently_becoming_zero() {
        version_parts("65536.1.2");
    }
}
