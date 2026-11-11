//! Shape tests for the secret scrubber (#183).
//!
//! Every key below is generated from a fixed seed at run time. None is a real key;
//! each only has the prefix, alphabet and length of the provider's format.

use crate::redact::redact;

const MARK: &str = "[REDACTED]";
const B64URL: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const UPPER_DIGIT: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const HEX: &[u8] = b"0123456789abcdef";

/// xorshift64: deterministic, so a failure reproduces.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).unwrap_or(0)
    }

    fn pick(&mut self, alphabet: &[u8], len: usize) -> String {
        (0..len)
            .map(|_| char::from(alphabet[self.below(alphabet.len())]))
            .collect()
    }
}

fn fake(seed: u64, alphabet: &[u8], len: usize) -> String {
    Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1).pick(alphabet, len)
}

/// One fake value per documented provider shape.
fn provider_keys() -> Vec<(&'static str, String)> {
    vec![
        ("openai legacy", format!("sk-{}", fake(1, ALNUM, 48))),
        (
            "openai project",
            format!("sk-proj-{}", fake(2, B64URL, 120)),
        ),
        (
            "openai svcacct",
            format!("sk-svcacct-{}", fake(3, B64URL, 100)),
        ),
        ("openrouter", format!("sk-or-v1-{}", fake(4, HEX, 64))),
        (
            "anthropic",
            format!("sk-ant-api03-{}AA", fake(5, B64URL, 93)),
        ),
        (
            "anthropic admin",
            format!("sk-ant-admin01-{}", fake(6, B64URL, 80)),
        ),
        ("google", format!("AIza{}", fake(7, B64URL, 35))),
        ("github classic", format!("ghp_{}", fake(8, ALNUM, 36))),
        ("github oauth", format!("gho_{}", fake(9, ALNUM, 36))),
        (
            "github user-to-server",
            format!("ghu_{}", fake(10, ALNUM, 36)),
        ),
        (
            "github server-to-server",
            format!("ghs_{}", fake(11, ALNUM, 36)),
        ),
        ("github refresh", format!("ghr_{}", fake(12, ALNUM, 76))),
        (
            "github fine-grained",
            format!("github_pat_{}_{}", fake(13, ALNUM, 22), fake(14, ALNUM, 59)),
        ),
        (
            "slack bot",
            format!(
                "xoxb-{}-{}-{}",
                fake(15, b"0123456789", 12),
                fake(16, b"0123456789", 13),
                fake(17, ALNUM, 24)
            ),
        ),
        (
            "slack user",
            format!(
                "xoxp-{}-{}",
                fake(18, b"0123456789", 12),
                fake(19, ALNUM, 32)
            ),
        ),
        ("slack app", format!("xoxa-2-{}", fake(20, ALNUM, 40))),
        (
            "aws access key id",
            format!("AKIA{}", fake(21, UPPER_DIGIT, 16)),
        ),
        (
            "aws session key id",
            format!("ASIA{}", fake(22, UPPER_DIGIT, 16)),
        ),
        ("generic base64url", fake(23, B64URL, 40)),
        ("generic odd-length hex", fake(24, HEX, 48)),
    ]
}

fn assert_scrubbed(input: &str, secret: &str) {
    let out = redact(input);
    assert!(
        out.contains(MARK),
        "nothing redacted in {input:?} -> {out:?}"
    );
    assert!(!out.contains(secret), "secret survived: {out:?}");
}

#[test]
fn redacts_each_documented_provider_shape_alone() {
    for (name, key) in provider_keys() {
        assert_eq!(redact(&key), MARK, "{name}: {key}");
    }
}

#[test]
fn redacts_each_shape_inside_prose_json_urls_and_query_strings() {
    for (name, key) in provider_keys() {
        let wrappers = [
            format!("using key {key} for the call"),
            format!("{{\"credential\":\"{key}\",\"n\":1}}"),
            format!("{{\"credential\": [\"{key}\"]}}"),
            format!("https://api.example.test/v1/models?credential={key}&page=2"),
            format!("https://api.example.test/{key}/models"),
            format!("error: provider rejected '{key}': 401"),
            format!("({key})"),
            format!("<{key}>"),
        ];
        for input in &wrappers {
            let out = redact(input);
            assert!(!out.contains(&key), "{name}: survived in {out:?}");
            assert!(out.contains(MARK), "{name}: {out:?}");
        }
    }
}

#[test]
fn keeps_the_surrounding_text_around_a_key() {
    let key = format!("sk-or-v1-{}", fake(30, HEX, 64));
    let out = redact(&format!(
        "error: 401 for {key} at https://openrouter.test/api"
    ));
    assert_eq!(
        out,
        "error: 401 for [REDACTED] at https://openrouter.test/api"
    );
}

#[test]
fn redacts_bearer_tokens_and_authorization_headers() {
    let jwt = format!(
        "eyJ{}.eyJ{}.{}",
        fake(40, B64URL, 30),
        fake(41, B64URL, 40),
        fake(42, B64URL, 43)
    );
    let opaque = fake(43, ALNUM, 24);
    let cases = [
        (
            format!("curl -H 'Authorization: Bearer {jwt}' x"),
            jwt.clone(),
        ),
        (format!("Authorization: Bearer {opaque}"), opaque.clone()),
        (format!("authorization: bearer {opaque}"), opaque.clone()),
        (format!("Authorization: Basic {opaque}=="), opaque.clone()),
        (
            format!("Proxy-Authorization: Token {opaque}"),
            opaque.clone(),
        ),
        (
            format!("{{\"Authorization\": \"Bearer {opaque}\"}}"),
            opaque.clone(),
        ),
        (format!("sent Bearer {jwt} to the proxy"), jwt.clone()),
        (
            "Authorization: Bearer my-secret-token".to_string(),
            "my-secret-token".to_string(),
        ),
    ];
    for (input, secret) in &cases {
        assert_scrubbed(input, secret);
    }
    assert_eq!(
        redact(&format!("Authorization: Bearer {opaque}\nnext line")),
        "Authorization: [REDACTED]\nnext line"
    );
    assert_eq!(
        redact(&format!("x Bearer {opaque} y")),
        "x Bearer [REDACTED] y"
    );
}

#[test]
fn redacts_secret_named_assignments() {
    let long = fake(50, ALNUM, 20);
    let cases = [
        format!("OPENROUTER_API_KEY={long}"),
        format!("export GITHUB_TOKEN={long}"),
        format!("CLIENT_SECRET={long}"),
        format!("DB_PASSWORD={long}"),
        format!("api_key={long}"),
        format!("--api-key={long}"),
        format!("{{\"api_key\": \"{long}\"}}"),
        format!("{{\"accessToken\":\"{long}\"}}"),
        format!("password: \"{long}\""),
        format!("x-api-key: {long}"),
        format!("https://h.test/cb?access_token={long}&state=1"),
        format!("{{\\\"api_key\\\":\\\"{long}\\\"}}"),
        format!("password = '{long}'"),
    ];
    for input in &cases {
        assert_scrubbed(input, &long);
    }
    assert_eq!(
        redact("Config api_key=secretvalue"),
        "Config api_key=[REDACTED]"
    );
    assert_eq!(
        redact(&format!("https://h.test/cb?token={long}&state=1")),
        "https://h.test/cb?token=[REDACTED]&state=1"
    );
}

#[test]
fn redacts_pem_private_key_blocks() {
    let body = fake(60, B64URL, 64);
    let pem = format!(
        "before\n-----BEGIN RSA PRIVATE KEY-----\n{body}\n{body}\n-----END RSA PRIVATE KEY-----\nafter"
    );
    let out = redact(&pem);
    assert_eq!(out, "before\n[REDACTED]\nafter");
    assert_scrubbed("Certificate: -----BEGIN RSA PRIVATE KEY-----", "BEGIN");
}

#[test]
fn leaves_digests_ids_and_evidence_fields_unchanged() {
    let sha1 = fake(70, HEX, 40);
    let sha256 = fake(71, HEX, 64);
    let md5 = fake(72, HEX, 32);
    let sha512 = fake(73, HEX, 128);
    let upper_sha = sha256.to_ascii_uppercase();
    let uuid = format!(
        "{}-{}-4{}-a{}-{}",
        fake(74, HEX, 8),
        fake(75, HEX, 4),
        fake(76, HEX, 3),
        fake(77, HEX, 3),
        fake(78, HEX, 12)
    );
    let samples = [
        sha1.clone(),
        sha256.clone(),
        md5,
        sha512,
        upper_sha,
        uuid.clone(),
        format!("commit {sha1}\nAuthor: someone"),
        format!(
            "{{\"patch_sha256\":\"{sha256}\",\"commit_sha\":\"{sha1}\",\"priors_sha256\":\"{sha256}\"}}"
        ),
        format!("patch_sha256={sha256} commit_sha={sha1}"),
        format!("sha256:{sha256}"),
        format!("{}..{}", &sha1[..12], sha1),
        format!("graph {uuid} task {uuid}"),
        format!("index {}..{} 100644", &sha1[..7], &sha256[..7]),
        format!("lattice:{sha256}:2:0"),
    ];
    for input in &samples {
        assert_eq!(&redact(input), input);
    }
}

#[test]
fn leaves_prose_code_and_short_base64_unchanged() {
    let samples = [
        // base64 of short text ("hello world", "meshloop rocks!")
        "aGVsbG8gd29ybGQ=",
        "bWVzaGxvb3Agcm9ja3Mh",
        "The task-1-attempt-3 worktree uses a bearer token from the keyring.",
        "Authorization: required for this endpoint is described in docs",
        "let token = parse_token(input);",
        "let api_key: Option<String> = None;",
        "pub struct Config { pub api_key: Option<String>, pub secret: SecretKind }",
        "fn redact_secret_value(token: &Token) -> Result<SecretValue, TokenError>",
        "if token == expected_token_value { return; }",
        "authorization: Option<HeaderValue>,",
        "self.api_key = config.api_key",
        "def test_redacts_openai_key_after_key_equals_and_keeps_tail(self):",
        "HashMapForTheConfigurationLoaderV2Builder::new()",
        "SECRET_KEY_BASE=$RAILS_SECRET",
        "error[E0308]: mismatched types\n --> crates/meshloop-adapters/src/redact.rs:20:5",
        "target/debug/deps/meshloop_adapters-1a2b3c4d5e6f7890.d",
        "_ZN4core3fmt5write17h2b7c3a9e1f0d4c6bE",
        "sk-secret",
        "ghp_ alone in prose",
        "A key like sk-... is redacted",
        "AKIA is the access key id prefix",
        "C:\\x\\task-1-attempt-3\\crates",
        "xoxb- tokens start like this",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
    ];
    for input in samples {
        assert_eq!(redact(input), input, "false positive on {input:?}");
    }
}

/// A key split by a line break is two words. The head keeps its provider prefix and is
/// caught. The tail is caught only when it alone passes the generic rule (32+ chars of
/// mixed classes); a hex tail of a digest length is indistinguishable from a digest and
/// is not caught. Callers must not rely on redaction for wrapped keys.
#[test]
fn split_key_head_is_redacted_tail_only_when_high_entropy() {
    let body = fake(80, B64URL, 80);
    let key = format!("sk-proj-{body}");
    let (head, tail) = key.split_at(40);
    let out = redact(&format!("{head}\n{tail}"));
    assert_eq!(out, "[REDACTED]\n[REDACTED]");

    let hex = fake(81, HEX, 64);
    let key = format!("sk-or-v1-{hex}");
    let (head, tail) = key.split_at(key.len() - 32);
    let out = redact(&format!("{head}\n{tail}"));
    assert_eq!(out, format!("[REDACTED]\n{tail}"), "documented gap");
}

#[test]
fn redaction_is_idempotent_on_generated_inputs() {
    let fragments: Vec<String> = provider_keys()
        .into_iter()
        .map(|(_, k)| k)
        .chain(
            [
                " ",
                "\n",
                "\"",
                "'",
                "=",
                ":",
                "&",
                "?",
                "/",
                "[",
                "]",
                "{",
                "}",
                "-",
                "_",
                "KEY=",
                "api_key=",
                "token: ",
                "://",
                "u:p@",
                "auth: Bearer ",
                "--auth ",
                "https://",
                "Authorization: ",
                "Bearer ",
                "bearer ",
                "-----BEGIN PRIVATE KEY-----",
                "-----END PRIVATE KEY-----",
                "[REDACTED]",
                "error: boom",
                "task-1",
                "sk-",
                "ghp_",
                "AKIA",
                "你",
                "…",
            ]
            .map(str::to_string),
        )
        .chain([
            fake(90, HEX, 40),
            fake(91, HEX, 64),
            fake(92, B64URL, 33),
            format!("ghp_{}:x@", fake(93, ALNUM, 36)),
            fake(94, ALNUM, 20),
        ])
        .collect();
    let mut rng = Rng(0x5EED_1830_0000_0001);
    for _ in 0..2_000 {
        let n = 1 + rng.below(12);
        let input: String = (0..n)
            .map(|_| fragments[rng.below(fragments.len())].as_str())
            .collect();
        let once = redact(&input);
        let twice = redact(&once);
        assert_eq!(once, twice, "not idempotent for {input:?}");
    }
}

#[test]
fn scrubbing_is_linear_on_adversarial_input() {
    // Shapes that would make a backtracking or rescanning matcher quadratic. The
    // key-named runs (`token=$…`) once re-read the value run at every key word.
    let inputs = [
        "sk-".repeat(200_000),
        "a-".repeat(300_000),
        "KEY ".repeat(150_000),
        "Bearer ".repeat(100_000),
        "-----BEGIN ".repeat(60_000),
        "api_key=\"".repeat(60_000),
        "api_key=\"$".repeat(200_000),
        "token=$".repeat(200_000),
        "KEY=%".repeat(200_000),
        "token:$".repeat(200_000),
        "--token -".repeat(200_000),
        "Authorization:".repeat(200_000),
        "eyJa.".repeat(200_000),
        "://a:b".repeat(200_000),
        "aB3+".repeat(300_000),
        "\u{1b}[".repeat(300_000),
        format!("Authorization:{}", " ".repeat(600_000)),
    ];
    for input in &inputs {
        let start = std::time::Instant::now();
        let _ = redact(input);
        // Linear work over ~2 MB is milliseconds even unoptimised; the quadratic
        // `token=$` case took 47 s at 1.1 MB.
        let took = start.elapsed();
        assert!(
            took < std::time::Duration::from_secs(5),
            "{took:?} for {:?}…",
            &input[..20]
        );
    }
}

#[test]
fn redacts_pem_blocks_inside_key_named_values() {
    let body = || {
        (0..3)
            .map(|n| {
                fake(
                    100 + n,
                    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
                    64,
                )
            })
            .collect::<Vec<_>>()
    };
    let lines = body();
    let ssh = format!(
        "SSH_PRIVATE_KEY=\"-----BEGIN OPENSSH PRIVATE KEY-----\n{}\n-----END OPENSSH PRIVATE KEY-----\"",
        lines.join("\n")
    );
    assert_eq!(redact(&ssh), "SSH_PRIVATE_KEY=\"[REDACTED]\"");
    let yaml = format!(
        "tls_key: -----BEGIN EC PRIVATE KEY-----\n{}\n-----END EC PRIVATE KEY-----\nnext: 1",
        lines.join("\n")
    );
    assert_eq!(redact(&yaml), "tls_key: [REDACTED]\nnext: 1");
    // GCP service-account JSON: the newlines are the two characters `\n`.
    let gcp = format!(
        "{{\"type\": \"service_account\", \"private_key\": \"-----BEGIN PRIVATE KEY-----\\n{}\\n-----END PRIVATE KEY-----\\n\", \"client_email\": \"x@y.test\"}}",
        lines.join("\\n")
    );
    let out = redact(&gcp);
    assert_eq!(
        out,
        "{\"type\": \"service_account\", \"private_key\": \"[REDACTED]\\n\", \"client_email\": \"x@y.test\"}"
    );
    for line in &lines {
        for piece in line.split(['+', '/']) {
            if piece.len() >= 8 {
                assert!(!out.contains(piece), "{piece} in {out}");
            }
        }
    }
}

#[test]
fn redacts_body_lines_of_an_unterminated_pem_block() {
    let lines: Vec<String> = (0..3)
        .map(|n| {
            fake(
                110 + n,
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/",
                64,
            )
        })
        .collect();
    let input = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\nerror: output cut here",
        lines.join("\n")
    );
    assert_eq!(redact(&input), "[REDACTED]\nerror: output cut here");
    let crlf = format!(
        "-----BEGIN RSA PRIVATE KEY-----\r\n{}\r\nok: done",
        lines.join("\r\n")
    );
    assert_eq!(redact(&crlf), "[REDACTED]\r\nok: done");
}

#[test]
fn redacts_standard_base64_runs_with_plus_and_slash() {
    let std_b64 = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    // AWS secret access key shape: 40 standard-base64 characters, bare.
    let mut aws = fake(120, std_b64, 40);
    aws.replace_range(13..14, "/");
    aws.replace_range(27..28, "+");
    assert_eq!(redact(&format!("  {aws}  ")), "  [REDACTED]  ");
    assert_eq!(
        redact(&format!("aws_secret_access_key = {aws}")),
        "aws_secret_access_key = [REDACTED]"
    );
    // Subresource-integrity hash (sha384): no piece reaches the 32-char word rule,
    // so the whole run is judged once and replaced.
    let mut sri_body = fake(121, std_b64, 64);
    sri_body.replace_range(25..26, "/");
    sri_body.replace_range(50..51, "+");
    let sri = format!("integrity sha384-{sri_body}");
    assert_eq!(redact(&sri), "integrity [REDACTED]");
    // A piece that alone passes the word rule is replaced in the first pass; the
    // run is then judged without it, so short fragments around it can remain.
    let long_piece = fake(122, ALNUM, 36);
    let out = redact(&format!("{long_piece}/Ab+9"));
    assert_eq!(out, "[REDACTED]/Ab+9");
    // Paths and URLs are not base64 secrets.
    for keep in [
        "crates/meshloop-adapters/src/redact/secrets_tests.rs",
        "https://github.com/smota/meshloop/pull/185/files",
        "a+b/c+d/e+f",
    ] {
        assert_eq!(redact(keep), keep);
    }
}

#[test]
fn redacts_flag_values_after_one_blank() {
    let value = fake(130, ALNUM, 24);
    for flag in [
        "--api-key",
        "--token",
        "--password",
        "-api-key",
        "--client-secret",
    ] {
        assert_eq!(
            redact(&format!("cmd {flag} {value} --verbose")),
            format!("cmd {flag} [REDACTED] --verbose")
        );
    }
    assert_eq!(redact("cmd --token --verbose"), "cmd --token --verbose");
    assert_eq!(redact("cmd --token short"), "cmd --token short");
}

#[test]
fn redacts_url_userinfo_passwords_and_keeps_host_and_path() {
    let ghs = format!("ghs_{}", fake(140, ALNUM, 36));
    let cases = [
        (
            "https://user:sMehGAkWvj7F@github.test/x.git",
            "https://user:[REDACTED]@github.test/x.git".to_string(),
        ),
        (
            "postgres://admin:hunter2pass@db:5432/x",
            "postgres://admin:[REDACTED]@db:5432/x".to_string(),
        ),
        (
            "redis://:p4ssw0rdLongEnough@h:6379",
            "redis://:[REDACTED]@h:6379".to_string(),
        ),
    ];
    for (input, want) in cases {
        assert_eq!(redact(input), want);
    }
    assert_eq!(
        redact(&format!("https://x-access-token:{ghs}@github.com/o/r.git")),
        "https://x-access-token:[REDACTED]@github.com/o/r.git"
    );
    for keep in [
        "https://github.com/o/r.git",
        "https://host:8080/path@v1",
        "ssh://git@github.com/o/r.git",
        "mailto:someone@example.test",
    ] {
        assert_eq!(redact(keep), keep);
    }
}

#[test]
fn redacts_keys_inside_ansi_colour_codes() {
    let akia = format!("AKIA{}", fake(150, UPPER_DIGIT, 16));
    let bearer = fake(151, ALNUM, 30);
    let proj = format!("sk-proj-{}", fake(152, B64URL, 60));
    assert_eq!(
        redact(&format!("\u{1b}[32m{akia}\u{1b}[0m")),
        "\u{1b}[32m[REDACTED]\u{1b}[0m"
    );
    assert_eq!(
        redact(&format!("\u{1b}[31mBearer {bearer}\u{1b}[0m")),
        "\u{1b}[31mBearer [REDACTED]\u{1b}[0m"
    );
    assert_eq!(
        redact(&format!("\u{1b}[1;31m{proj}\u{1b}[0m")),
        "\u{1b}[1;31m[REDACTED]\u{1b}[0m"
    );
    let plain = "\u{1b}[1m\u{1b}[31merror\u{1b}[0m: mismatched types";
    assert_eq!(redact(plain), plain);
}

#[test]
fn redacts_new_key_names_and_keyed_values_of_any_class() {
    for input in [
        "DB_PASS=pnnhcc826ZWOf0WOOsEg",
        "DB_PWD=pnnhcc826ZWOf0WOOsEg",
        "GOOGLE_CREDENTIALS=pnnhcc826ZWOf0WOOsEg",
        "x-auth: pnnhcc826ZWOf0WOOsEg",
        "CLIENT_SECRET: knialijbwylkytqpjtxaznan",
        "api_key: qydlpwbrsgwcsjfnaqgjyyba",
        "user_credential: \"correct horse battery staple\"",
    ] {
        let out = redact(input);
        assert!(
            out.ends_with("[REDACTED]") || out.ends_with("[REDACTED]\""),
            "{input:?} -> {out:?}"
        );
    }
    // Shell working-directory variables are paths.
    assert_eq!(redact("PWD=/home/user/code"), "PWD=/home/user/code");
    assert_eq!(redact("OLDPWD=/home/user"), "OLDPWD=/home/user");
}

#[test]
fn redacts_whole_jwts_and_more_prefixed_tokens() {
    let jwt = format!(
        "eyJhbGciOiJIUzI1NiJ9.eyJ{}.{}",
        fake(160, B64URL, 30),
        fake(161, B64URL, 43)
    );
    assert_eq!(redact(&format!("jwt {jwt} end")), "jwt [REDACTED] end");
    let unsigned = format!("eyJhbGciOiJub25lIn0.eyJ{}", fake(162, B64URL, 20));
    assert_eq!(redact(&unsigned), "[REDACTED]");
    for token in [
        format!("glpat-{}", fake(163, B64URL, 20)),
        format!("glpat-{}-{}", fake(168, ALNUM, 17), fake(169, ALNUM, 2)),
        format!("xapp-1-A0123-0123-{}", fake(164, HEX, 64)),
    ] {
        assert_eq!(redact(&token), "[REDACTED]", "{token}");
    }
    let hook = format!(
        "https://hooks.slack.com/services/T{}/B{}/{}",
        fake(165, UPPER_DIGIT, 8),
        fake(166, UPPER_DIGIT, 8),
        fake(167, ALNUM, 24)
    );
    assert_eq!(redact(&hook), "https://hooks.slack.com/services/[REDACTED]");
}

#[test]
fn key_name_suffix_needs_a_name_boundary() {
    for keep in [
        "monkey=bananaBread12",
        "hotkey=Ctrl+Shift+K",
        "turkey: 1234567890abcdefABCD",
        "jsonkey=\"abcdefgh\"",
        "bypass=abcdefghijkl",
        "max_tokens=4096",
        "tokens=12345678",
    ] {
        assert_eq!(redact(keep), keep);
    }
    for hit in [
        "api_key=abcdefgh",
        "KEY=abcdefgh",
        "apiKey=abcdefgh",
        "x-api-key=abcdefgh",
    ] {
        assert!(redact(hit).ends_with("=[REDACTED]"), "{hit}");
    }
}

#[test]
fn redacts_a_token_used_as_the_url_username() {
    let ghp = format!("ghp_{}", fake(170, ALNUM, 36));
    assert_eq!(
        redact(&format!(
            "git clone https://{ghp}:x-oauth-basic@github.com/o/r.git"
        )),
        "git clone https://[REDACTED]:[REDACTED]@github.com/o/r.git"
    );
    let proj = format!("sk-proj-{}", fake(171, B64URL, 60));
    assert_eq!(
        redact(&format!("https://{proj}:@api.example.test/")),
        "https://[REDACTED]:@api.example.test/"
    );
    assert_eq!(
        redact(&format!("https://{ghp}@github.com/o/r.git")),
        "https://[REDACTED]@github.com/o/r.git"
    );
    assert_eq!(
        redact("https://plain-user:pw123456@h.test/x"),
        "https://plain-user:[REDACTED]@h.test/x"
    );
}

#[test]
fn redacts_scheme_credentials_after_key_named_values() {
    let token = fake(180, ALNUM, 20);
    for prefix in [
        "auth: Bearer ",
        "X-Auth: Bearer ",
        "--auth Bearer ",
        "token: Bearer ",
        "proxy_auth: Bearer ",
        "password: Basic ",
    ] {
        assert_eq!(
            redact(&format!("{prefix}{token}")),
            format!("{prefix}[REDACTED]"),
            "{prefix}"
        );
    }
    assert_eq!(
        redact(&format!("api_key: $X Authorization: Bearer {token}")),
        "api_key: $X Authorization: [REDACTED]"
    );
    for keep in [
        "warning: Key: 'Bearer'",
        "error: auth: AuthenticationFailedForUser",
        "error: token: InvalidTokenFormatError",
        "pwd: /home/u/code",
    ] {
        assert_eq!(redact(keep), keep);
    }
}
