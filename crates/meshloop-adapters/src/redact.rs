//! Length-cap and secret-pattern redaction before persistence (ADR 0007 / ML-008).
//! Evidence longer than the cap keeps diagnostic lines, a suffix, and the original length.

const CAP: usize = 2048;

/// Previous cap kept `CAP` bytes and then pushed `…` (3 bytes).
const MAX_STORED: usize = CAP + 3;

const PATTERNS: &[&str] = &["-----BEGIN", "sk-", "ghp_", "Bearer ", "api_key="];

/// `sk-` plus at least this many ASCII alphanumeric characters is a key.
const SK_MIN_KEY_CHARS: usize = 40;

/// Substrings that mark a line the repair lattice can use.
/// `error:` does not match rustc `error[E0308]`, so the bracket forms are separate.
const DIAGNOSTIC_NEEDLES: &[&str] = &[
    "error:", "error[", "warning:", "warning[", "-->", "FAILED", "panicked",
];

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
    // Secrets are removed on the full string, before any bytes are dropped.
    if out.len() > CAP {
        return shrink_evidence(&out);
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

/// Marker plus a char-aligned suffix, with the last diagnostic lines in front.
/// The marker records `text.len()` and is counted inside `MAX_STORED`.
fn shrink_evidence(text: &str) -> String {
    let marker = truncation_marker(text.len());
    if marker.len() >= MAX_STORED {
        return prefix_within(&marker, MAX_STORED).to_string();
    }
    let body_budget = MAX_STORED - marker.len();
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| is_diagnostic_line(line))
        .collect();
    // At most half the body is diagnostics, so a suffix always remains when there
    // is room. Fewer matching lines leave the unused bytes for the suffix.
    let diag = diagnostic_section(&lines, body_budget / 2);
    let sep = usize::from(!diag.is_empty());
    let tail = suffix_within(text, body_budget.saturating_sub(diag.len() + sep));
    let mut stored = String::with_capacity(diag.len() + sep + marker.len() + tail.len());
    if !diag.is_empty() {
        stored.push_str(&diag);
        stored.push('\n');
    }
    stored.push_str(&marker);
    stored.push_str(tail);
    debug_assert!(stored.len() <= MAX_STORED);
    stored
}

fn truncation_marker(original_len: usize) -> String {
    // `truncated` has no `error` substring, so the marker is not a diagnostic.
    format!("…[truncated {original_len} bytes]…")
}

fn is_diagnostic_line(line: &str) -> bool {
    diagnostic_needle_at(line).is_some()
}

fn diagnostic_needle_at(line: &str) -> Option<usize> {
    DIAGNOSTIC_NEEDLES
        .iter()
        .filter_map(|needle| line.find(needle))
        .min()
}

/// Last matching lines that fit, in original order. One oversized line is clipped
/// on a char boundary instead of being dropped or stored whole.
fn diagnostic_section(lines: &[&str], budget: usize) -> String {
    if budget == 0 || lines.is_empty() {
        return String::new();
    }
    let mut chosen: Vec<&str> = Vec::new();
    let mut used = 0usize;
    for line in lines.iter().rev() {
        let line = *line;
        let sep = usize::from(!chosen.is_empty());
        let cost = used.saturating_add(sep).saturating_add(line.len());
        if cost <= budget {
            chosen.push(line);
            used += sep + line.len();
            continue;
        }
        if chosen.is_empty() {
            let clipped = clip_diagnostic_line(line, budget);
            if !clipped.is_empty() {
                return clipped;
            }
            continue;
        }
        break;
    }
    chosen.reverse();
    chosen.join("\n")
}

fn clip_diagnostic_line(line: &str, budget: usize) -> String {
    if budget == 0 {
        return String::new();
    }
    if line.len() <= budget {
        return line.to_string();
    }
    let Some(at) = diagnostic_needle_at(line) else {
        return String::new();
    };
    let clipped = if line.len() - at <= budget {
        suffix_within(line, budget)
    } else {
        prefix_within(&line[at..], budget)
    };
    if is_diagnostic_line(clipped) {
        clipped.to_string()
    } else {
        String::new()
    }
}

fn prefix_within(text: &str, max_bytes: usize) -> &str {
    if max_bytes >= text.len() {
        text
    } else {
        &text[..text.floor_char_boundary(max_bytes)]
    }
}

fn suffix_within(text: &str, max_bytes: usize) -> &str {
    if max_bytes >= text.len() {
        text
    } else {
        let start = text.ceil_char_boundary(text.len() - max_bytes);
        &text[start..]
    }
}

#[cfg(test)]
mod tests {
    use meshloop_domain::diagnostic::parse_diagnostics;

    use super::*;

    fn non_diagnostic_noise(min_bytes: usize) -> String {
        let line = "note: compiling crate\n";
        line.repeat(min_bytes.div_ceil(line.len()))
    }

    fn compile_noise_then_duplicated_attribute() -> String {
        let mut input = non_diagnostic_noise(50_000);
        input.push_str("error: duplicated attribute");
        input
    }

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

    #[test]
    fn leaves_input_at_the_cap_unchanged() {
        let input = "a".repeat(CAP);
        assert_eq!(redact(&input), input);
    }

    #[test]
    fn keeps_trailing_error_and_original_length() {
        let input = compile_noise_then_duplicated_attribute();
        let out = redact(&input);
        let marker = format!("…[truncated {} bytes]…", input.len());
        assert!(!marker.contains("error"));
        assert!(out.contains("error: duplicated attribute"), "{out}");
        assert!(out.contains(&marker), "{out}");
        assert!(out.len() <= CAP + 3, "{}", out.len());
    }

    #[test]
    fn trailing_error_blocks_the_repair_lattice() {
        let out = redact(&compile_noise_then_duplicated_attribute());
        assert!(parse_diagnostics(&out).blocking() > 0, "{out}");
    }

    #[test]
    fn keeps_a_middle_diagnostic_ahead_of_a_long_tail() {
        let diagnostic = "error: only in the middle";
        let input = format!(
            "{}{diagnostic}\n{}",
            non_diagnostic_noise(CAP + 1),
            non_diagnostic_noise(CAP + 1),
        );
        let out = redact(&input);
        assert!(out.contains(diagnostic), "{out}");
        assert!(
            out.contains(&format!("…[truncated {} bytes]…", input.len())),
            "{out}"
        );
        assert!(out.len() <= CAP + 3, "{}", out.len());
        assert!(
            input.len() > CAP + non_diagnostic_noise(CAP + 1).len(),
            "tail after the diagnostic must exceed the cap"
        );
    }

    #[test]
    fn keeps_diagnostic_markers_ahead_of_a_long_tail() {
        let samples = [
            "error: boom",
            "error[E0308]: mismatched types",
            "warning: boom",
            "warning[unused]: boom",
            " --> src/lib.rs:1:1",
            "test case FAILED",
            "thread panicked at boom",
        ];
        for diagnostic in samples {
            let input = format!(
                "{}{diagnostic}\n{}",
                non_diagnostic_noise(CAP + 1),
                non_diagnostic_noise(CAP + 1),
            );
            let out = redact(&input);
            assert!(
                out.contains(diagnostic),
                "dropped {diagnostic:?} from {out}"
            );
            assert!(out.len() <= CAP + 3, "{}", out.len());
        }
    }

    #[test]
    fn redacts_a_key_in_the_tail_before_shortening() {
        let key = format!("sk-{}", "c".repeat(SK_MIN_KEY_CHARS));
        // A newline keeps the `sk-` boundary; glued `aaaask-` is not a key.
        let input = format!("{}\n{key}", "a".repeat(CAP));
        let redacted_len = CAP + 1 + "[REDACTED]".len();
        let out = redact(&input);
        assert!(out.contains("[REDACTED]"), "{out}");
        assert!(!out.contains("sk-"), "{out}");
        assert!(
            out.contains(&format!("…[truncated {redacted_len} bytes]…")),
            "{out}"
        );
        assert!(
            !out.contains(&format!("truncated {} bytes", input.len())),
            "{out}"
        );
        assert!(out.len() <= CAP + 3, "{}", out.len());
    }

    #[test]
    fn caps_length_keeps_the_tail_when_nothing_is_diagnostic() {
        let long = format!("{}{}", "a".repeat(4000), "b".repeat(1000));
        let out = redact(&long);
        assert!(out.len() <= CAP + 3, "{}", out.len());
        assert!(out.ends_with('b'), "{out}");
        assert!(!out.starts_with('a'), "{out}");
        assert!(out.contains(&format!("…[truncated {} bytes]…", long.len())));
    }

    #[test]
    fn bounds_repeated_rustc_diagnostics() {
        let chunk = "error[E0308]: mismatched types\n --> src/lib.rs:1:1\n";
        let input = chunk.repeat((64 * 1024_usize).div_ceil(chunk.len()));
        assert!(input.len() >= 64 * 1024);
        let out = redact(&input);
        assert!(out.contains("error[E0308]"), "{out}");
        assert!(out.contains("-->"), "{out}");
        assert!(out.len() <= CAP + 3, "{}", out.len());
        assert!(out.contains(&format!("…[truncated {} bytes]…", input.len())));
    }

    #[test]
    fn snaps_multibyte_tail_without_panicking() {
        let input = "你".repeat(1_000);
        assert!(input.len() > CAP);
        let out = redact(&input);
        assert!(out.len() <= CAP + 3, "{}", out.len());
        assert!(out.contains('…'));
        assert!(
            out.chars()
                .all(|ch| ch == '你' || !ch.is_alphabetic() || ch.is_ascii())
        );
    }
}
