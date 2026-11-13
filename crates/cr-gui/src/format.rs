//! Plain-text rendering of a card read. This is what Copy all puts on the
//! clipboard. It is the same fields the CLI prints; nothing is unmasked here.

use crlib::CardInfo;

pub fn plaintext(infos: &[CardInfo]) -> String {
    let mut out = String::new();
    for info in infos {
        out.push_str(&format!("== {} ==\n", info.app));
        for field in &info.fields {
            out.push_str(&format!("{:<22} {}\n", field.key, field.value));
        }
        if !info.raw.is_empty() {
            out.push_str("--- dump ---\n");
            for line in &info.raw {
                out.push_str(&format!("{:<22} {}\n", line.label, line.value));
            }
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_lists_safe_fields_and_redacted_dump() {
        let mut info = CardInfo::new("emv");
        info.push("scheme", "Visa");
        info.push("pan", "************4242");
        info.push_raw("5A", "[redacted]");

        let text = plaintext(&[info]);

        assert!(text.contains("== emv =="));
        assert!(text.contains("scheme                 Visa"));
        assert!(text.contains("************4242"));
        assert!(text.contains("[redacted]"));
        assert!(!text.contains("4242424242424242"));
    }
}
