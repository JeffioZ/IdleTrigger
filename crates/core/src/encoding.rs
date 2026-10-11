//! Small encoders shared by IPC and update verification.

pub fn hex_lower(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 15) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex_preserves_the_ipc_and_digest_wire_format() {
        assert_eq!(super::hex_lower(&[]), "");
        let bytes: Vec<u8> = (0..=255).collect();
        let reference: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        assert_eq!(super::hex_lower(&bytes), reference);
    }
}
