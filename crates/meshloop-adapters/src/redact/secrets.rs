//! Secret-shape scrubbing for evidence text (#183).
//!
//! The input is read once, left to right, as *words* (maximal runs of ASCII
//! `[A-Za-z0-9_-]`) and the bytes between them. ANSI CSI sequences
//! (`ESC [ params final`) are copied whole as separators, so colour codes neither
//! hide a key nor get split. A word is replaced by [`MARK`] when it has a
//! documented provider key shape or passes the generic high-entropy rule. Some
//! words open a *context*: `Bearer`, `Authorization:`, `-----BEGIN`, JWTs, Slack
//! webhook URLs, and secret-named keys such as `API_KEY=`, `"token":` or
//! `--password VALUE`; then the value after them is replaced and the key text is
//! kept. `scheme://user:password@host` loses only the password.
//!
//! # Linear runtime
//!
//! Linearity is structural, not argued per rule. The cursor only moves forward,
//! and every forward scan goes through [`Reader::run`], which memoises the last
//! scan of each kind: a scan that starts inside the previous scan of its kind
//! returns that scan's end without reading. The re-reads that made earlier
//! versions quadratic (one rule per word starting inside one long run, whether
//! it hit or missed) cost O(1) each. The answers are those of plain scans, so the
//! output does not depend on the memo. Scans of values and
//! header tails stop at an earlier replacement (`[REDACTED`), so a value is judged
//! on what precedes it and a second pass leaves it alone.
//!
//! The remaining work is a fixed number of passes over each word and over each
//! value or token a scan returned, the PEM `-----END` search (a hit, or a miss
//! that is never repeated), and one pass over the output for base64 runs
//! ([`scrub_b64_runs`]). `Reader` counts all of it; a test bounds the count by
//! `INSPECT_PER_BYTE` (8) times the input length over the adversarial shapes and
//! random concatenations of every probe fragment.

use std::cell::Cell;

/// Replacement text. Its brackets are not word bytes, so a second pass sees the
/// bare word `REDACTED`, which matches no rule (idempotency).
pub(super) const MARK: &str = "[REDACTED]";

/// `sk-` plus at least this many ASCII alphanumeric characters is a key even
/// without a digit. With a digit, [`SK_MIN_KEY_CHARS_WITH_DIGIT`] is enough.
pub(crate) const SK_MIN_KEY_CHARS: usize = 40;
const SK_MIN_KEY_CHARS_WITH_DIGIT: usize = 20;

/// `(prefix, minimum body length, body needs a digit)` for prefixed tokens:
/// GitHub (classic, OAuth, user-to-server, server-to-server, refresh,
/// fine-grained), GitLab PAT, Slack app-level, npm, Hugging Face, Stripe live keys.
const PREFIXED_TOKENS: &[(&str, usize, bool)] = &[
    ("ghp_", 16, false),
    ("gho_", 16, false),
    ("ghu_", 16, false),
    ("ghs_", 16, false),
    ("ghr_", 16, false),
    ("github_pat_", 22, false),
    ("glpat-", 20, false),
    ("xapp-", 20, true),
    ("npm_", 30, false),
    ("hf_", 30, true),
    ("sk_live_", 20, false),
    ("rk_live_", 20, false),
];

/// Slack token prefixes: `xoxa-`, `xoxb-`, `xoxp-`, `xoxo-`, `xoxs-`, `xoxr-`.
const SLACK_KINDS: &[u8] = b"abposr";
const SLACK_MIN_BODY: usize = 10;
/// Slack incoming-webhook and workflow URLs: the path after these is the secret.
const SLACK_HOOK_PREFIXES: &[&str] = &[
    "hooks.slack.com/services/",
    "hooks.slack.com/workflows/",
    "hooks.slack.com/triggers/",
];

/// Google API keys are `AIza` plus 35 base64url characters.
const GOOGLE_PREFIX: &str = "AIza";
const GOOGLE_MIN_BODY: usize = 30;

/// AWS access key ids: long-term `AKIA` and temporary `ASIA`, plus 16 of `[A-Z0-9]`.
const AWS_PREFIXES: &[&str] = &["AKIA", "ASIA"];
const AWS_KEY_LEN: usize = 20;

/// Upper-cased key-name suffixes whose assigned value is a secret. The suffix must
/// start the word, follow `_` or `-`, or start a `camelCase` hump, so `api_key`,
/// `apiKey` and `KEY` match and `monkey`, `hotkey` and `turkey` do not.
const SECRET_KEY_SUFFIXES: &[&str] = &[
    "KEY",
    "APIKEY",
    "TOKEN",
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "PASS",
    "PWD",
    "CREDENTIAL",
    "CREDENTIALS",
    "AUTH",
];
/// Shell working-directory variables hold paths, not passwords.
const NOT_SECRET_KEY_NAMES: &[&str] = &["PWD", "OLDPWD"];
/// An unquoted `NAME=value` with no blanks around `=`, a quoted value, and a
/// `--flag VALUE` are redacted from this length.
const ASSIGNED_MIN_LEN: usize = 8;
/// Any other unquoted value (after `:` or a spaced `=`) from this length, unless
/// it reads as code (a call, a path or a type name).
const KEYED_MIN_LEN: usize = 12;
/// A value that is a credential without help from the key name.
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

const PEM_BEGIN: &str = "-----BEGIN";
/// PEM body lines are 64 (OpenSSH: 70) characters; shorter is the last line.
const PEM_FULL_LINE: usize = 40;
/// Longest `user:password` part scanned after `://`.
const URL_USERINFO_MAX: usize = 512;

/// Generic rule: a word this long or longer can be a high-entropy secret.
const GENERIC_MIN_LEN: usize = 32;
/// A `[A-Za-z0-9+/_-]` run (standard base64) is tested whole from this length.
const B64_RUN_MIN_LEN: usize = 40;
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

/// What a scan accepts. Each kind is a fixed predicate on the byte at an index
/// (with a constant look-ahead for the `[REDACTED` marker), so a scan's result
/// depends only on the input and its start.
#[derive(Clone, Copy)]
enum Kind {
    Word,
    Blank,
    Token,
    /// Unquoted value body: value bytes except `=` and `:`, up to a marker.
    ValueBody,
    Equals,
    Line,
    CsiParam,
    CsiInter,
    Userinfo,
    PemBody,
    Dashes,
    HookPath,
    B64Body,
    /// Quoted value: up to a quote, backslash, newline or marker.
    Quoted,
    /// `Authorization` value up to the end of the line or a marker, and also up
    /// to `"` or `'` when the header is quoted.
    AuthTail,
    AuthTailDq,
    AuthTailSq,
}

const KINDS: usize = 17;

/// Reader over the input that every rule scans through.
///
/// Linearity is structural. Each scan kind remembers its last scan
/// `start..end`: every byte in it was accepted and the byte at `end` was not.
/// A later scan of the same kind that starts inside that range has the same
/// answer, `end`, and costs O(1). Scans start at or after the cursor's word,
/// which only moves forward, so a fresh scan of a kind rarely overlaps the last
/// one; `inspected` makes that checkable. The answers are exactly those of an
/// unmemoised scan, so outputs (and idempotency) do not depend on the memo.
///
/// `inspected` counts bytes read by scans plus the other linear passes over
/// scanned text (word rules, value checks, searches); tests bound it by a
/// constant times the input length.
struct Reader<'a> {
    text: &'a str,
    bytes: &'a [u8],
    memo: [Cell<(usize, usize)>; KINDS],
    inspected: Cell<usize>,
}

impl<'a> Reader<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            bytes: text.as_bytes(),
            memo: std::array::from_fn(|_| Cell::new((usize::MAX, 0))),
            inspected: Cell::new(0),
        }
    }

    /// Records a linear pass over `n` bytes.
    fn note(&self, n: usize) {
        self.inspected.set(self.inspected.get() + n);
    }

    fn mark_at(&self, i: usize) -> bool {
        self.bytes[i..].starts_with(MARK_PREFIX.as_bytes())
    }

    fn accepts(&self, kind: Kind, i: usize) -> bool {
        let b = self.bytes[i];
        match kind {
            Kind::Word => is_word_byte(b),
            Kind::Blank => is_blank(b),
            Kind::Token => is_token_byte(b),
            Kind::ValueBody => is_value_byte(b) && b != b'=' && b != b':' && !self.mark_at(i),
            Kind::Equals => b == b'=',
            Kind::Line => b != b'\n' && b != b'\r',
            Kind::CsiParam => (0x30..=0x3f).contains(&b),
            Kind::CsiInter => (0x20..=0x2f).contains(&b),
            Kind::Userinfo => {
                b.is_ascii_graphic()
                    && !matches!(
                        b,
                        b'/' | b'@' | b'?' | b'#' | b'"' | b'\'' | b'<' | b'>' | b'`'
                    )
            }
            Kind::PemBody => is_pem_body_byte(b),
            Kind::Dashes => b == b'-',
            Kind::HookPath => is_word_byte(b) || b == b'/',
            Kind::B64Body => is_b64_body_byte(b),
            Kind::Quoted => !is_quote(b) && b != b'\\' && b != b'\n' && !self.mark_at(i),
            Kind::AuthTail => b != b'\n' && b != b'\r' && !self.mark_at(i),
            Kind::AuthTailDq => b != b'\n' && b != b'\r' && b != b'"' && !self.mark_at(i),
            Kind::AuthTailSq => b != b'\n' && b != b'\r' && b != b'\'' && !self.mark_at(i),
        }
    }

    /// End of the run of `kind` bytes from `from`.
    fn run(&self, kind: Kind, from: usize) -> usize {
        let memo = &self.memo[kind as usize];
        let (start, end) = memo.get();
        if start <= from && from <= end {
            self.note(1);
            return end;
        }
        let mut i = from;
        while i < self.bytes.len() && self.accepts(kind, i) {
            i += 1;
        }
        self.note(i - from + 1);
        memo.set((from, i));
        i
    }

    fn line_end(&self, from: usize) -> usize {
        self.run(Kind::Line, from)
    }

    /// `text[from..].find(pat)`, counted.
    fn find(&self, from: usize, to: usize, pat: &str) -> Option<usize> {
        let found = self.text[from..to].find(pat).map(|at| from + at);
        self.note(found.map_or(to, |at| at + pat.len()) - from);
        found
    }
}

/// Start of an earlier replacement. Scans of values stop here, so a value is
/// judged on what precedes it and a second pass leaves it alone.
const MARK_PREFIX: &str = "[REDACTED";

pub(super) fn scrub(input: &str) -> String {
    scrub_counting(input).0
}

/// [`scrub`] plus the number of bytes inspected (both passes).
pub(super) fn scrub_counting(input: &str) -> (String, usize) {
    let r = Reader::new(input);
    let bytes = r.bytes;
    let mut out = String::with_capacity(input.len());
    let mut no_pem_end = false;
    let mut i = 0;
    while i < bytes.len() {
        if !is_word_byte(bytes[i]) {
            if let Some(stop) = ansi_csi_end(&r, i) {
                out.push_str(&input[i..stop]);
                i = stop;
                continue;
            }
            if let Some((pw_start, pw_end)) = url_password(&r, i) {
                // The username can itself be a token (`https://ghp_…:x@host`):
                // its words get the key-shape rules, then the password is replaced.
                out.push_str("://");
                push_words_scrubbed(&r, &mut out, i + 3, pw_start - 1);
                out.push(':');
                out.push_str(MARK);
                i = pw_end;
                continue;
            }
            // `i` is always on a char boundary: words, CSI sequences and URL
            // userinfo are ASCII and other text is copied a whole char at a time.
            let Some(ch) = input[i..].chars().next() else {
                break;
            };
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let end = r.run(Kind::Word, i);
        let word = &input[i..end];
        // The word rules below are a fixed number of passes over the word.
        r.note(word.len());
        if let Some(stop) = pem_block_end(&r, i, end, &mut no_pem_end) {
            out.push_str(MARK);
            i = stop;
            continue;
        }
        if let Some(stop) = jwt_end(&r, i, end) {
            out.push_str(MARK);
            i = stop;
            continue;
        }
        if let Some((secret_start, stop)) = slack_hook(&r, i) {
            out.push_str(&input[i..secret_start]);
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
        if let Some((value_start, value_end)) = context_value(&r, end, word, opener) {
            out.push_str(&input[i..value_start]);
            out.push_str(MARK);
            i = value_end;
            continue;
        }
        out.push_str(word);
        i = end;
    }
    let (out, second) = scrub_b64_runs(&out);
    (out, r.inspected.get() + second)
}

/// Copies the ASCII range `from..to`, replacing each word with a key shape.
fn push_words_scrubbed(r: &Reader<'_>, out: &mut String, from: usize, to: usize) {
    let mut i = from;
    while i < to {
        let end = if is_word_byte(r.bytes[i]) {
            r.run(Kind::Word, i).min(to)
        } else {
            i + 1
        };
        let piece = &r.text[i..end];
        r.note(piece.len());
        out.push_str(if is_secret_word(piece) { MARK } else { piece });
        i = end;
    }
}

/// Second pass: maximal `[A-Za-z0-9+/_-]` runs (plus `=` padding) that join words
/// with `+` or `/` and are high-entropy as a whole, such as a bare AWS secret
/// access key. It runs on the first pass's output so that a run is judged
/// without the parts already replaced; deciding it on the input made the result
/// depend on whether a key inside the run had been redacted, and a second
/// `redact` could then change it. Runs are maximal on both sides, so replacing
/// one does not change its neighbours and the pass is idempotent.
fn scrub_b64_runs(text: &str) -> (String, usize) {
    let r = Reader::new(text);
    let bytes = r.bytes;
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        if !is_b64_body_byte(bytes[i]) {
            r.note(1);
            i += 1;
            continue;
        }
        let body = r.run(Kind::B64Body, i);
        let run = r.run(Kind::Equals, body);
        r.note(run - i);
        if is_secret_b64_run(&text[i..run]) {
            out.push_str(&text[copied..i]);
            out.push_str(MARK);
            copied = run;
        }
        i = run;
    }
    out.push_str(&text[copied..]);
    (out, r.inspected.get())
}

fn is_b64_body_byte(b: u8) -> bool {
    is_word_byte(b) || b == b'+' || b == b'/'
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

/// End of an ANSI CSI sequence `ESC [ params(0x30-0x3F)* intermediates(0x20-0x2F)*
/// final(0x40-0x7E)` starting at `i`.
fn ansi_csi_end(r: &Reader<'_>, i: usize) -> Option<usize> {
    let bytes = r.bytes;
    if bytes[i] != 0x1b || bytes.get(i + 1) != Some(&b'[') {
        return None;
    }
    let params = r.run(Kind::CsiParam, i + 2);
    let inter = r.run(Kind::CsiInter, params);
    bytes
        .get(inter)
        .filter(|b| (0x40..=0x7e).contains(*b))
        .map(|_| inter + 1)
}

/// `scheme://user:password@host`: the password range, when `i` is at `://`.
fn url_password(r: &Reader<'_>, i: usize) -> Option<(usize, usize)> {
    let input = r.text;
    if !input[i..].starts_with("://") {
        return None;
    }
    let bytes = r.bytes;
    let from = i + 3;
    let stop = r.run(Kind::Userinfo, from);
    if stop - from >= URL_USERINFO_MAX || bytes.get(stop) != Some(&b'@') {
        return None;
    }
    // A userinfo holding an earlier replacement is left alone (idempotency).
    r.note(2 * (stop - from));
    if has_mark(&input[from..stop]) {
        return None;
    }
    let colon = bytes[from..stop].iter().position(|b| *b == b':')?;
    let start = from + colon + 1;
    (start < stop).then_some((start, stop))
}

/// `-----BEGIN …` through the matching `-----END …-----`. With no END after it,
/// the BEGIN line and the base64 body lines that follow (`[A-Za-z0-9+/=]`, with
/// literal `\n` allowed). `no_end` records a failed search so it runs once.
fn pem_block_end(
    r: &Reader<'_>,
    start: usize,
    word_end: usize,
    no_end: &mut bool,
) -> Option<usize> {
    let word = &r.text[start..word_end];
    let label = word.trim_start_matches('-');
    if word.len() - label.len() < 5 || !label.starts_with("BEGIN") {
        return None;
    }
    let bytes = r.bytes;
    if !*no_end {
        if let Some(at) = r.find(word_end, bytes.len(), "-----END") {
            let after_end = at + "-----END".len();
            let eol = r.line_end(after_end);
            return Some(match r.find(after_end, eol, "-----") {
                Some(close) => r.run(Kind::Dashes, close),
                None => eol,
            });
        }
        *no_end = true;
    }
    let mut stop = r.line_end(word_end);
    // Body lines are full width except the last; a short line ends the block.
    let mut prev_full = true;
    while prev_full {
        let mut next = stop;
        if bytes.get(next) == Some(&b'\r') {
            next += 1;
        }
        if bytes.get(next) != Some(&b'\n') {
            break;
        }
        next += 1;
        let body = r.run(Kind::PemBody, next);
        if body == next || body != r.line_end(next) {
            break;
        }
        prev_full = body - next >= PEM_FULL_LINE;
        stop = body;
    }
    Some(stop)
}

fn is_pem_body_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'\\')
}

/// `eyJ….eyJ…[.…]`: a JWT's header, payload and signature as one secret.
fn jwt_end(r: &Reader<'_>, start: usize, word_end: usize) -> Option<usize> {
    const MIN_SEGMENT: usize = 4;
    let bytes = r.bytes;
    if word_end - start < MIN_SEGMENT
        || !bytes[start..].starts_with(b"eyJ")
        || bytes.get(word_end) != Some(&b'.')
    {
        return None;
    }
    let payload = word_end + 1;
    if !bytes[payload..].starts_with(b"eyJ") {
        return None;
    }
    let payload_end = r.run(Kind::Word, payload);
    if payload_end - payload < MIN_SEGMENT {
        return None;
    }
    if bytes.get(payload_end) == Some(&b'.') {
        return Some(r.run(Kind::Word, payload_end + 1));
    }
    Some(payload_end)
}

/// `hooks.slack.com/services/T…/B…/…`: keeps the host and replaces the path.
fn slack_hook(r: &Reader<'_>, start: usize) -> Option<(usize, usize)> {
    let rest = &r.text[start..];
    let prefix = SLACK_HOOK_PREFIXES.iter().find(|p| rest.starts_with(**p))?;
    let secret = start + prefix.len();
    let end = r.run(Kind::HookPath, secret);
    (end > secret).then_some((secret, end))
}

/// A base64 run that joins words with `+` or `/` and is high-entropy as a whole.
fn is_secret_b64_run(run: &str) -> bool {
    run.len() >= B64_RUN_MIN_LEN
        && run.bytes().any(|b| b == b'+' || b == b'/')
        && !run.contains("//")
        && is_high_entropy(run.trim_end_matches('='))
}

/// The secret value after `word`, when it opens a context. A rule's decision
/// depends only on the bytes after the word, never on earlier text, so a second
/// pass over the output decides the same (idempotency).
fn context_value(
    r: &Reader<'_>,
    word_end: usize,
    word: &str,
    opener: Option<u8>,
) -> Option<(usize, usize)> {
    let result = if word.eq_ignore_ascii_case("bearer") {
        bearer_token(r, word_end)
    } else if ends_with_ignore_case(word, "authorization") {
        authorization_value(r, word_end, opener)
    } else if is_secret_key_name(word) {
        assigned_value(r, word, word_end)
    } else {
        return None;
    };
    result.ok()
}

/// Holds an earlier replacement: a second pass must not rescan around it.
fn has_mark(range: &str) -> bool {
    range.contains(MARK_PREFIX)
}

/// `Err` carries how far the rule read.
type Value = Result<(usize, usize), usize>;

fn bearer_token(r: &Reader<'_>, word_end: usize) -> Value {
    let start = r.run(Kind::Blank, word_end);
    if start == word_end {
        return Err(start);
    }
    let end = r.run(Kind::Token, start);
    let token = &r.text[start..end];
    r.note(token.len());
    if is_credential_like(token) {
        Ok((start, end))
    } else {
        Err(end)
    }
}

/// `Authorization: <scheme> <credentials>` (also `Proxy-Authorization`, JSON keys).
/// Redacts to the closing quote, the end of the line or an earlier replacement,
/// when the value starts with a scheme word or a credential.
fn authorization_value(r: &Reader<'_>, word_end: usize, opener: Option<u8>) -> Value {
    let (input, bytes) = (r.text, r.bytes);
    let mut j = word_end;
    if let Some(q) = opener
        && bytes.get(j) == Some(&q)
    {
        j += 1;
    }
    j = r.run(Kind::Blank, j);
    if bytes.get(j) != Some(&b':') {
        return Err(j);
    }
    let mut start = r.run(Kind::Blank, j + 1);
    let mut close = opener;
    if let Some(&q) = bytes.get(start)
        && is_quote(q)
    {
        start += 1;
        close = Some(q);
    }
    if input[start..].starts_with(MARK) || input[start..].starts_with(PEM_BEGIN) {
        return Err(start);
    }
    let first_end = r.run(Kind::Token, start);
    let first = &input[start..first_end];
    r.note(first.len());
    let is_scheme = AUTH_SCHEMES.iter().any(|s| first.eq_ignore_ascii_case(s));
    if !is_scheme && !is_strong_value(first) {
        return Err(first_end);
    }
    let tail = match close {
        Some(b'"') => Kind::AuthTailDq,
        Some(b'\'') => Kind::AuthTailSq,
        _ => Kind::AuthTail,
    };
    // Stops at an earlier replacement, so the cursor passes every byte read.
    let mut end = r.run(tail, start);
    while end > start && is_blank(bytes[end - 1]) {
        end -= 1;
    }
    if end > start {
        Ok((start, end))
    } else {
        Err(end)
    }
}

/// The value after a secret-named key: `KEY=v`, `"key": "v"`, `key: v`,
/// `key = 'v'`, `--api-key v`.
fn assigned_value(r: &Reader<'_>, word: &str, word_end: usize) -> Value {
    let (input, bytes) = (r.text, r.bytes);
    // `--api-key VALUE`: a flag takes its value after one blank.
    if word.starts_with('-')
        && bytes.get(word_end).copied().is_some_and(is_blank)
        && let Some(&first) = bytes.get(word_end + 1)
        && first != b'-'
        && is_value_byte(first)
    {
        let v = word_end + 1;
        let end = value_end(r, v);
        let value = &input[v..end];
        r.note(value.len());
        if let Some(hit) = scheme_credential(r, v, end) {
            return hit;
        }
        let ok = value.len() >= ASSIGNED_MIN_LEN && is_literal_value(value);
        return if ok { Ok((v, end)) } else { Err(end) };
    }
    let mut j = word_end;
    // Closing quote of a JSON or YAML key, possibly escaped (`\"key\":`).
    let key_quoted = skip_quote(bytes, &mut j).is_some();
    let tight = !key_quoted && bytes.get(j) == Some(&b'=');
    j = r.run(Kind::Blank, j);
    let Some(&sep) = bytes.get(j) else {
        return Err(j);
    };
    if sep != b'=' && sep != b':' {
        return Err(j);
    }
    j += 1;
    // `==` is a comparison and `::` a path, not an assignment.
    if bytes.get(j) == Some(&sep) {
        return Err(j);
    }
    let start = r.run(Kind::Blank, j);
    let tight = tight && start == j;
    let mut v = start;
    let quoted = skip_quote(bytes, &mut v).is_some();
    // Already redacted, or a PEM block that the PEM rule replaces whole.
    if input[v..].starts_with(MARK) || input[v..].starts_with(PEM_BEGIN) {
        return Err(v);
    }
    if quoted {
        let end = r.run(Kind::Quoted, v);
        let value = &input[v..end];
        let ok = value.len() >= ASSIGNED_MIN_LEN && is_literal_value(value);
        return if ok { Ok((v, end)) } else { Err(end) };
    }
    let end = value_end(r, v);
    let value = &input[v..end];
    r.note(value.len());
    if let Some(hit) = scheme_credential(r, v, end) {
        return hit;
    }
    let ok = is_literal_value(value)
        && if tight && sep == b'=' {
            value.len() >= ASSIGNED_MIN_LEN
        } else {
            value.len() >= KEYED_MIN_LEN && !looks_like_code(value)
        };
    if ok { Ok((v, end)) } else { Err(end) }
}

/// `auth: Bearer <token>`, `password: Basic <b64>`: a scheme word as the value
/// means the next token is the credential.
fn scheme_credential(r: &Reader<'_>, v: usize, end: usize) -> Option<Value> {
    let value = &r.text[v..end];
    if !AUTH_SCHEMES.iter().any(|s| value.eq_ignore_ascii_case(s)) {
        return None;
    }
    let start = r.run(Kind::Blank, end);
    if start == end {
        return None;
    }
    let token_end = r.run(Kind::Token, start);
    let token = &r.text[start..token_end];
    r.note(token.len());
    // Token bytes exclude `[`, so a token never holds a replacement.
    Some(if is_credential_like(token) {
        Ok((start, token_end))
    } else {
        Err(token_end)
    })
}

/// End of an unquoted value: value bytes up to the next `=`, `:` or earlier
/// replacement, plus trailing `=` padding.
fn value_end(r: &Reader<'_>, v: usize) -> usize {
    let body = r.run(Kind::ValueBody, v);
    r.run(Kind::Equals, body)
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

/// Not empty, not a shell or template reference.
fn is_literal_value(value: &str) -> bool {
    !value.is_empty() && !value.starts_with('$') && !value.starts_with('%')
}

/// A call, a `path::item`, a dotted field path, or a `TypeName`: code that names
/// a secret, not the secret.
fn looks_like_code(value: &str) -> bool {
    let b = value.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    value.contains('(')
        || value.contains("::")
        || (value.contains('.')
            && !b[0].is_ascii_digit()
            && b.iter().all(|c| ident(*c) || *c == b'.'))
        || (b[0].is_ascii_uppercase() && b.iter().all(u8::is_ascii_alphabetic))
}

fn is_secret_key_name(word: &str) -> bool {
    if NOT_SECRET_KEY_NAMES
        .iter()
        .any(|n| word.eq_ignore_ascii_case(n))
    {
        return false;
    }
    SECRET_KEY_SUFFIXES.iter().any(|suffix| {
        ends_with_ignore_case(word, suffix) && starts_a_name_part(word, word.len() - suffix.len())
    })
}

/// `at` starts the word, follows `_`/`-`, or starts a `camelCase` hump.
fn starts_a_name_part(word: &str, at: usize) -> bool {
    let b = word.as_bytes();
    at == 0
        || matches!(b[at - 1], b'_' | b'-')
        || (b[at - 1].is_ascii_lowercase() && b[at].is_ascii_uppercase())
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
        || is_prefixed_token(word)
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

fn is_prefixed_token(word: &str) -> bool {
    PREFIXED_TOKENS
        .iter()
        .any(|(prefix, min_body, needs_digit)| {
            word.strip_prefix(prefix).is_some_and(|body| {
                body.len() >= *min_body
                    && (!needs_digit || body.bytes().any(|b| b.is_ascii_digit()))
            })
        })
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
