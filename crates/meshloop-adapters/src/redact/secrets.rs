//! Secret-shape scrubbing for evidence text (#183).
//!
//! The input is read once, left to right, as *words* (maximal runs of ASCII
//! `[A-Za-z0-9_-]`) and the bytes between them. A word is replaced by
//! [`MARK`] when it has a documented provider key shape or passes the generic
//! high-entropy rule. A few words open a *context*: `Bearer`, `Authorization:`,
//! `-----BEGIN`, and secret-named keys such as `API_KEY=` / `"token":`; then the
//! value after them is replaced and the key text is kept.
//!
//! Runtime is linear in the input. The cursor only moves forward. At a word, a
//! rule reads the word, then at most one blank run, one separator, and one value
//! (a run of value bytes, a quoted string up to its closing quote or newline, or
//! the rest of the line). On a match the cursor jumps past everything read. On a
//! miss it moves to the end of the word, so the bytes read after it are read again
//! as later words, at most once more each. The PEM search for `-----END` runs at
//! most once without a match and is then skipped for the rest of the input. No
//! rule retries from inside a word, so there is no backtracking.

/// Replacement text. Its brackets are not word bytes, so a second pass sees the
/// bare word `REDACTED`, which matches no rule (idempotency).
pub(super) const MARK: &str = "[REDACTED]";

/// `sk-` plus at least this many ASCII alphanumeric characters is a key even
/// without a digit. With a digit, [`SK_MIN_KEY_CHARS_WITH_DIGIT`] is enough.
pub(crate) const SK_MIN_KEY_CHARS: usize = 40;
const SK_MIN_KEY_CHARS_WITH_DIGIT: usize = 20;

/// GitHub token prefixes (classic, OAuth, user-to-server, server-to-server, refresh).
const GITHUB_PREFIXES: &[&str] = &["ghp_", "gho_", "ghu_", "ghs_", "ghr_"];
const GITHUB_MIN_BODY: usize = 16;
const GITHUB_PAT_PREFIX: &str = "github_pat_";
const GITHUB_PAT_MIN_BODY: usize = 22;

/// Slack token prefixes: `xoxa-`, `xoxb-`, `xoxp-`, `xoxo-`, `xoxs-`, `xoxr-`.
const SLACK_KINDS: &[u8] = b"abposr";
const SLACK_MIN_BODY: usize = 10;

/// Google API keys are `AIza` plus 35 base64url characters.
const GOOGLE_PREFIX: &str = "AIza";
const GOOGLE_MIN_BODY: usize = 30;

/// AWS access key ids: long-term `AKIA` and temporary `ASIA`, plus 16 of `[A-Z0-9]`.
const AWS_PREFIXES: &[&str] = &["AKIA", "ASIA"];
const AWS_KEY_LEN: usize = 20;

/// Upper-cased key-name suffixes whose assigned value is a secret.
const SECRET_KEY_SUFFIXES: &[&str] = &["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD"];
/// An unquoted `NAME=value` with no blanks around `=` is redacted from this length.
const ASSIGNED_MIN_LEN: usize = 8;
/// Any other value (after `:` or a spaced `=`) must look like a credential.
const STRONG_MIN_LEN: usize = 16;

/// `Authorization` scheme words that mark the rest of the header as a credential.
const AUTH_SCHEMES: &[&str] = &[
    "bearer",
    "basic",
    "token",
    "digest",
    "bot",
    "apikey",
    "negotiate",
    "ntlm",
    "aws4-hmac-sha256",
];
const BEARER_MIN_LEN: usize = 8;

/// Generic rule: a word this long or longer can be a high-entropy secret.
const GENERIC_MIN_LEN: usize = 32;
/// Mixed-case + digit words need this much Shannon entropy (bits per char).
/// Random base64url of 32 chars averages about 4.6; English identifiers sit near 4.
const GENERIC_MIN_BITS: f64 = 4.3;
/// ... and at least this share of adjacent character-class changes. Random
/// base64url averages about 0.65 (1st percentile about 0.42 at 32 chars);
/// `CamelCaseIdentifierV2` names sit between 0.3 and 0.41, and alphabets like
/// `ABC…xyz…012` near 0. A few percent of random 32-char tokens fall below one
/// of the two thresholds: the generic rule trades recall for no false positives.
const GENERIC_MIN_CLASS_CHANGES: f64 = 0.45;
/// Pure hex needs this much entropy (maximum 4 bits).
const HEX_MIN_BITS: f64 = 3.0;
/// Allow-shapes: hex of these lengths is a digest (md5, sha1, sha224, sha256,
/// sha384, sha512): git commit SHAs and meshloop's own `patch_sha256`,
/// `commit_sha` and `priors_sha256` evidence fields.
const HEX_DIGEST_LENS: &[usize] = &[32, 40, 56, 64, 96, 128];

pub(super) fn scrub(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut no_pem_end = false;
    let mut i = 0;
    while i < bytes.len() {
        if !is_word_byte(bytes[i]) {
            // `i` is always on a char boundary: words are ASCII and other text is
            // copied a whole char at a time.
            let Some(ch) = input[i..].chars().next() else {
                break;
            };
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let end = run_end(bytes, i, is_word_byte);
        let word = &input[i..end];
        if let Some(stop) = pem_block_end(input, i, end, &mut no_pem_end) {
            out.push_str(MARK);
            i = stop;
            continue;
        }
        // A key shape wins over a context: `sk-…KEY=` is a key, not a key name.
        if is_secret_word(word) {
            out.push_str(MARK);
            i = end;
            continue;
        }
        let opener = i.checked_sub(1).map(|p| bytes[p]).filter(|b| is_quote(*b));
        if let Some((value_start, value_end)) = context_value(input, end, word, opener) {
            out.push_str(&input[i..value_start]);
            out.push_str(MARK);
            i = value_end;
            continue;
        }
        out.push_str(word);
        i = end;
    }
    out
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Bytes of a bearer token: base64, base64url and JWT dots.
fn is_token_byte(b: u8) -> bool {
    is_word_byte(b) || matches!(b, b'.' | b'+' | b'/' | b'=' | b'~')
}

/// Bytes of an unquoted assigned value: stops at blanks, quotes and the
/// delimiters of query strings, JSON, shell and markup.
fn is_value_byte(b: u8) -> bool {
    b.is_ascii_graphic()
        && !is_quote(b)
        && !matches!(
            b,
            b'&' | b';' | b',' | b')' | b'}' | b']' | b'>' | b'<' | b'`' | b'\\'
        )
}

fn is_quote(b: u8) -> bool {
    b == b'"' || b == b'\''
}

fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn run_end(bytes: &[u8], from: usize, pred: impl Fn(u8) -> bool) -> usize {
    bytes[from..]
        .iter()
        .position(|b| !pred(*b))
        .map_or(bytes.len(), |n| from + n)
}

fn line_end(bytes: &[u8], from: usize) -> usize {
    run_end(bytes, from, |b| b != b'\n' && b != b'\r')
}

/// `-----BEGIN …` through the matching `-----END …-----`, or to the end of the
/// line when no END follows. `no_end` records a failed search so it runs once.
fn pem_block_end(input: &str, start: usize, word_end: usize, no_end: &mut bool) -> Option<usize> {
    let word = &input[start..word_end];
    let label = word.trim_start_matches('-');
    if word.len() - label.len() < 5 || !label.starts_with("BEGIN") {
        return None;
    }
    let bytes = input.as_bytes();
    if !*no_end {
        if let Some(off) = input[word_end..].find("-----END") {
            let after_end = word_end + off + "-----END".len();
            let eol = line_end(bytes, after_end);
            return Some(match input[after_end..eol].find("-----") {
                Some(close) => run_end(bytes, after_end + close, |b| b == b'-'),
                None => eol,
            });
        }
        *no_end = true;
    }
    Some(line_end(bytes, word_end))
}

/// A secret value that follows `word`: `(value_start, value_end)`.
fn context_value(
    input: &str,
    word_end: usize,
    word: &str,
    opener: Option<u8>,
) -> Option<(usize, usize)> {
    let bytes = input.as_bytes();
    if word.eq_ignore_ascii_case("bearer") {
        return bearer_token(input, word_end);
    }
    if ends_with_ignore_case(word, "authorization") {
        return authorization_value(input, word_end, opener);
    }
    if is_secret_key_name(word) {
        return assigned_value(input, bytes, word_end);
    }
    None
}

fn bearer_token(input: &str, word_end: usize) -> Option<(usize, usize)> {
    let bytes = input.as_bytes();
    let start = run_end(bytes, word_end, is_blank);
    if start == word_end {
        return None;
    }
    let end = run_end(bytes, start, is_token_byte);
    is_credential_like(&input[start..end]).then_some((start, end))
}

/// `Authorization: <scheme> <credentials>` (also `Proxy-Authorization`, JSON keys).
/// Redacts to the closing quote or the end of the line, when the value starts
/// with a scheme word or a credential.
fn authorization_value(input: &str, word_end: usize, opener: Option<u8>) -> Option<(usize, usize)> {
    let bytes = input.as_bytes();
    let mut j = word_end;
    if let Some(q) = opener
        && bytes.get(j) == Some(&q)
    {
        j += 1;
    }
    j = run_end(bytes, j, is_blank);
    if bytes.get(j) != Some(&b':') {
        return None;
    }
    let mut start = run_end(bytes, j + 1, is_blank);
    let mut close = opener;
    if let Some(&q) = bytes.get(start)
        && is_quote(q)
    {
        start += 1;
        close = Some(q);
    }
    if input[start..].starts_with(MARK) {
        return None;
    }
    let first_end = run_end(bytes, start, is_token_byte);
    let first = &input[start..first_end];
    let is_scheme = AUTH_SCHEMES.iter().any(|s| first.eq_ignore_ascii_case(s));
    if !is_scheme && !is_strong_value(first) {
        return None;
    }
    let mut end = run_end(bytes, start, |b| {
        b != b'\n' && b != b'\r' && Some(b) != close
    });
    while end > start && is_blank(bytes[end - 1]) {
        end -= 1;
    }
    Some((start, end))
}

/// The value after a secret-named key: `KEY=v`, `"key": "v"`, `key: v`, `key = 'v'`.
fn assigned_value(input: &str, bytes: &[u8], word_end: usize) -> Option<(usize, usize)> {
    let mut j = word_end;
    // Closing quote of a JSON or YAML key, possibly escaped (`\"key\":`).
    let key_quoted = skip_quote(bytes, &mut j).is_some();
    let tight = !key_quoted && bytes.get(j) == Some(&b'=');
    j = run_end(bytes, j, is_blank);
    let sep = *bytes.get(j)?;
    if sep != b'=' && sep != b':' {
        return None;
    }
    j += 1;
    // `==` is a comparison and `::` a path, not an assignment.
    if bytes.get(j) == Some(&sep) {
        return None;
    }
    let start = run_end(bytes, j, is_blank);
    let tight = tight && start == j;
    let mut v = start;
    if skip_quote(bytes, &mut v).is_some() {
        let end = run_end(bytes, v, |b| !is_quote(b) && b != b'\\' && b != b'\n');
        let value = &input[v..end];
        return (value.len() >= ASSIGNED_MIN_LEN && is_literal_value(value)).then_some((v, end));
    }
    if input[v..].starts_with(MARK) {
        return None;
    }
    let end = run_end(bytes, v, is_value_byte);
    let value = &input[v..end];
    if !is_literal_value(value) {
        return None;
    }
    let hit = if tight && sep == b'=' {
        value.len() >= ASSIGNED_MIN_LEN
    } else {
        is_strong_value(value)
    };
    hit.then_some((v, end))
}

/// Advances past `"`, `'`, `\"` or `\'`, returning the quote.
fn skip_quote(bytes: &[u8], j: &mut usize) -> Option<u8> {
    let mut k = *j;
    if bytes.get(k) == Some(&b'\\') {
        k += 1;
    }
    let q = *bytes.get(k).filter(|b| is_quote(**b))?;
    *j = k + 1;
    Some(q)
}

/// Not empty, not already redacted, not a shell or template reference.
fn is_literal_value(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(MARK)
        && !value.starts_with('$')
        && !value.starts_with('%')
}

fn is_secret_key_name(word: &str) -> bool {
    SECRET_KEY_SUFFIXES
        .iter()
        .any(|suffix| ends_with_ignore_case(word, suffix))
}

fn ends_with_ignore_case(word: &str, suffix: &str) -> bool {
    word.len() >= suffix.len()
        && word.as_bytes()[word.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

/// A bearer token: long enough and not a plain English word.
fn is_credential_like(token: &str) -> bool {
    if token.len() < BEARER_MIN_LEN || token.starts_with(MARK) {
        return false;
    }
    let c = Classes::of(token.as_bytes());
    c.count() >= 2 || c.other
}

/// A value that is a credential without help from the key name.
fn is_strong_value(value: &str) -> bool {
    let c = Classes::of(value.as_bytes());
    (value.len() >= STRONG_MIN_LEN && c.digit && (c.lower || c.upper))
        || (value.bytes().all(is_word_byte) && is_secret_word(value))
}

/// Provider key shapes, then the generic high-entropy rule.
fn is_secret_word(word: &str) -> bool {
    is_sk_key(word)
        || is_github_token(word)
        || is_slack_token(word)
        || is_google_key(word)
        || is_aws_key_id(word)
        || is_high_entropy(word)
}

/// OpenAI (`sk-…`, `sk-proj-…`, `sk-svcacct-…`), OpenRouter (`sk-or-v1-…`) and
/// Anthropic (`sk-ant-api03-…`, `sk-ant-admin01-…`). Only alphanumerics count
/// toward the length, so `sk-some-kebab-name` stays.
fn is_sk_key(word: &str) -> bool {
    let Some(body) = word.strip_prefix("sk-") else {
        return false;
    };
    let alnum = body.bytes().filter(u8::is_ascii_alphanumeric).count();
    let digit = body.bytes().any(|b| b.is_ascii_digit());
    alnum >= SK_MIN_KEY_CHARS || (digit && alnum >= SK_MIN_KEY_CHARS_WITH_DIGIT)
}

fn is_github_token(word: &str) -> bool {
    let body_len = |prefix: &str| {
        word.strip_prefix(prefix)
            .map(|b| b.bytes().filter(u8::is_ascii_alphanumeric).count())
    };
    GITHUB_PREFIXES
        .iter()
        .any(|p| body_len(p).is_some_and(|n| n >= GITHUB_MIN_BODY))
        || body_len(GITHUB_PAT_PREFIX).is_some_and(|n| n >= GITHUB_PAT_MIN_BODY)
}

fn is_slack_token(word: &str) -> bool {
    let b = word.as_bytes();
    b.len() >= 5 + SLACK_MIN_BODY
        && b.starts_with(b"xox")
        && SLACK_KINDS.contains(&b[3])
        && b[4] == b'-'
        && b[5..].iter().any(u8::is_ascii_digit)
}

fn is_google_key(word: &str) -> bool {
    word.strip_prefix(GOOGLE_PREFIX)
        .is_some_and(|body| body.len() >= GOOGLE_MIN_BODY)
}

fn is_aws_key_id(word: &str) -> bool {
    word.len() == AWS_KEY_LEN
        && AWS_PREFIXES.iter().any(|p| word.starts_with(p))
        && word
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// Conservative fallback for unknown providers. Digests, UUIDs and mangled Rust
/// symbols are allow-shapes; everything else needs length, all three letter/digit
/// classes, frequent class changes and high entropy.
fn is_high_entropy(word: &str) -> bool {
    let b = word.as_bytes();
    if b.len() < GENERIC_MIN_LEN || is_uuid(b) || is_mangled_symbol(word) {
        return false;
    }
    let c = Classes::of(b);
    if is_single_case_hex(b) {
        return !HEX_DIGEST_LENS.contains(&b.len())
            && c.digit
            && (c.lower || c.upper)
            && shannon_bits(b) >= HEX_MIN_BITS;
    }
    c.lower
        && c.upper
        && c.digit
        && class_change_ratio(b) >= GENERIC_MIN_CLASS_CHANGES
        && shannon_bits(b) >= GENERIC_MIN_BITS
}

fn is_single_case_hex(b: &[u8]) -> bool {
    b.iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
        || b.iter().all(|c| matches!(c, b'0'..=b'9' | b'A'..=b'F'))
}

/// `8-4-4-4-12` hex.
fn is_uuid(b: &[u8]) -> bool {
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Legacy (`_ZN…E`) and v0 (`_R…`) Rust symbol mangling seen in backtraces.
fn is_mangled_symbol(word: &str) -> bool {
    word.starts_with("_ZN") || word.starts_with("_R")
}

#[derive(Default)]
struct Classes {
    lower: bool,
    upper: bool,
    digit: bool,
    other: bool,
}

impl Classes {
    fn of(b: &[u8]) -> Self {
        let mut c = Self::default();
        for &x in b {
            match class_of(x) {
                0 => c.lower = true,
                1 => c.upper = true,
                2 => c.digit = true,
                _ => c.other = true,
            }
        }
        c
    }

    fn count(&self) -> usize {
        usize::from(self.lower) + usize::from(self.upper) + usize::from(self.digit)
    }
}

fn class_of(b: u8) -> u8 {
    if b.is_ascii_lowercase() {
        0
    } else if b.is_ascii_uppercase() {
        1
    } else if b.is_ascii_digit() {
        2
    } else {
        3
    }
}

#[allow(clippy::cast_precision_loss)] // word lengths are far below 2^52
fn class_change_ratio(b: &[u8]) -> f64 {
    if b.len() < 2 {
        return 0.0;
    }
    let changes = b
        .windows(2)
        .filter(|w| class_of(w[0]) != class_of(w[1]))
        .count();
    changes as f64 / (b.len() - 1) as f64
}

#[allow(clippy::cast_precision_loss)] // word lengths are far below 2^52
fn shannon_bits(b: &[u8]) -> f64 {
    let mut counts = [0usize; 256];
    for &x in b {
        counts[usize::from(x)] += 1;
    }
    let n = b.len() as f64;
    counts
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}
