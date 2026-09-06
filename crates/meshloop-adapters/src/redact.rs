//! Length-cap and secret-pattern redaction before persistence (ADR 0007 / ML-008).

const CAP: usize = 2048;

const PATTERNS: &[&str] = &["-----BEGIN", "sk-", "ghp_", "Bearer ", "api_key="];

pub fn redact(input: &str) -> String {
    let mut out = input.to_string();
    for p in PATTERNS {
        if out.contains(p) {
            out = out.replace(p, "[REDACTED]");
        }
    }
    if out.len() > CAP {
        out.truncate(CAP);
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_token_prefixes() {
        assert!(redact("Authorization: Bearer abc").contains("[REDACTED]"));
        assert!(!redact("sk-secret").contains("sk-"));
    }

    #[test]
    fn caps_length() {
        let long = "a".repeat(5000);
        assert!(redact(&long).len() <= CAP + 3);
    }
}
