//! Length-cap and secret-pattern redaction before persistence (ADR 0007 / ML-008).

const CAP: usize = 2048;

const PATTERNS: &[&str] = &["-----BEGIN", "sk-", "ghp_", "Bearer ", "api_key="];

/// `sk-` plus at least this many ASCII alphanumeric characters is a key.
const SK_MIN_KEY_CHARS: usize = 40;

pub fn redact(input: &str) -> String {
    let mut out = input.to_string();
    for p in PATTERNS {
        if !out.contains(p) {
            continue;
        }
        // Bare `sk-` is also the tail of `task-1`; only a bounded key is a secret.
        out = if *p == "sk-" {
            redact_sk_keys(&out)
        } else {
            out.replace(p, "[REDACTED]")
        };
    }
    if out.len() > CAP {
        out.truncate(CAP);
        out.push('…');
    }
    out
}

fn redact_sk_keys(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if let Some(end) = sk_key_end(bytes, i) {
            out.push_str("[REDACTED]");
            i = end;
            continue;
        }
        let Some(ch) = input[i..].chars().next() else {
            break;
        };
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// End offset of `sk-` plus its key characters, when this index starts a key.
fn sk_key_end(bytes: &[u8], i: usize) -> Option<usize> {
    let prefix = b"sk-";
    if i + prefix.len() > bytes.len() || &bytes[i..i + prefix.len()] != prefix {
        return None;
    }
    if i > 0 && bytes[i - 1].is_ascii_alphanumeric() {
        return None;
    }
    let mut end = i + prefix.len();
    let mut count = 0usize;
    while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
        end += 1;
        count += 1;
    }
    (count >= SK_MIN_KEY_CHARS).then_some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_token_prefixes() {
        assert!(redact("Authorization: Bearer abc").contains("[REDACTED]"));
        let key = format!("sk-{}", "a".repeat(SK_MIN_KEY_CHARS));
        let out = redact(&key);
        assert_eq!(out, "[REDACTED]");
        assert!(!out.contains("sk-"));
        assert_eq!(redact("sk-secret"), "sk-secret");
    }

    #[test]
    fn caps_length() {
        let long = "a".repeat(5000);
        assert!(redact(&long).len() <= CAP + 3);
    }

    #[test]
    fn leaves_windows_task_path_unchanged() {
        let path = r"C:\x\task-1-attempt-3\crates";
        assert_eq!(redact(path), path);
    }

    #[test]
    fn leaves_sk_glued_to_an_alphanumeric_token_unchanged() {
        let embedded = format!("task-{}", "a".repeat(SK_MIN_KEY_CHARS));
        assert_eq!(redact(&embedded), embedded);
    }

    #[test]
    fn redacts_openai_key_after_key_equals() {
        let key = format!("sk-{}", "b".repeat(SK_MIN_KEY_CHARS + 8));
        let input = format!("KEY={key}");
        let out = redact(&input);
        assert_eq!(out, "KEY=[REDACTED]");
        assert!(!out.contains("sk-"));
    }
}
