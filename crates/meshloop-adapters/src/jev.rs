//! Opt-in Jev decision client. [`JevClient::connect`] builds a ureq agent and
//! does not send. Offline tests use [`JevTransportFake`], which returns scripted
//! bytes and a latency value. It does not sleep, spawn, or open a socket.

use std::collections::HashMap;
use std::error::Error;
use std::io::Read;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use meshloop_domain::decision::{DecisionOutcome, SanitizedReason};
use meshloop_engine::ports::{
    BatchNoulQuery, ChoiceQuery, DecisionPort, DecisionPortError, NoulQuery, ScoreQuery,
};

/// One overall request budget. Not three stacked connect/read/write phases.
pub const TOTAL_TIMEOUT: Duration = Duration::from_millis(500);

/// Largest response body that is parsed. One byte past this is [`JevError::TooLarge`].
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Same cap as `ScriptedDecisionPort`. The engine constant is private.
const MAX_BATCH_PROPOSITIONS: usize = 24;

/// Bytes from one transport round trip. `latency` is reported, not slept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JevRaw {
    pub latency: Duration,
    pub body: Vec<u8>,
}

/// Transport failure. Display text is a stable code, never a path or a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JevError {
    Timeout,
    TooLarge,
    Malformed,
    /// I/O or a missing fake fixture. Not a parse of a response body.
    Transport,
}

impl JevError {
    /// Stable fingerprint used when this error is collapsed into the engine.
    ///
    /// Temporary collapse: `DecisionPortError` has no transport variant, so
    /// [`DecisionPort`] reports `timeout`, `too-large`, or `malformed` as
    /// `Unscripted`. `transport` is an I/O failure that is none of those three.
    pub fn code(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::TooLarge => "too-large",
            Self::Malformed => "malformed",
            Self::Transport => "transport",
        }
    }
}

impl std::fmt::Display for JevError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl Error for JevError {}

/// No sockets. The live client and the fake both implement this.
pub trait JevTransport {
    fn round_trip(&self, request: &[u8]) -> Result<JevRaw, JevError>;
}

/// In-memory bodies keyed by the exact request bytes.
///
/// `insert` stores `latency` as data. `round_trip` does not sleep or spawn.
#[derive(Debug, Default)]
pub struct JevTransportFake {
    responses: HashMap<Vec<u8>, JevRaw>,
}

impl JevTransportFake {
    pub fn insert(&mut self, request: impl AsRef<[u8]>, body: impl AsRef<[u8]>, latency: Duration) {
        self.responses.insert(
            request.as_ref().to_vec(),
            JevRaw {
                latency,
                body: body.as_ref().to_vec(),
            },
        );
    }
}

impl JevTransport for JevTransportFake {
    fn round_trip(&self, request: &[u8]) -> Result<JevRaw, JevError> {
        self.responses
            .get(request)
            .cloned()
            .ok_or(JevError::Transport)
    }
}

/// Live ureq 2 agent. Constructed by [`JevClient::connect`], which does not send.
pub struct UreqTransport {
    endpoint: String,
    agent: ureq::Agent,
}

impl std::fmt::Debug for UreqTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UreqTransport")
    }
}

impl JevTransport for UreqTransport {
    fn round_trip(&self, request: &[u8]) -> Result<JevRaw, JevError> {
        let started = Instant::now();
        let response = self
            .agent
            .post(&self.endpoint)
            .set("Content-Type", "application/json")
            .send_bytes(request)
            .map_err(classify_ureq)?;
        let body = read_capped(response.into_reader())?;
        Ok(JevRaw {
            latency: started.elapsed(),
            body,
        })
    }
}

/// Decision client over any [`JevTransport`].
#[derive(Debug)]
pub struct JevClient<T> {
    transport: T,
}

impl<T: JevTransport> JevClient<T> {
    pub fn from_transport(transport: T) -> Self {
        Self { transport }
    }

    /// Transport, then budget, then size, then JSON. A late or oversized body is not parsed.
    pub fn exchange(&self, request: &[u8]) -> Result<DecisionOutcome, JevError> {
        let raw = self.transport.round_trip(request)?;
        if raw.latency > TOTAL_TIMEOUT {
            return Err(JevError::Timeout);
        }
        if raw.body.len() > MAX_RESPONSE_BYTES {
            return Err(JevError::TooLarge);
        }
        parse_outcome(&raw.body)
    }

    fn decide(&self, query: &JevQuery<'_>) -> Result<DecisionOutcome, DecisionPortError> {
        let bytes = request_bytes(query).map_err(port_error)?;
        self.exchange(&bytes).map_err(port_error)
    }
}

impl JevClient<UreqTransport> {
    /// Build a ureq 2 agent with rustls and a 500 ms budget.
    ///
    /// Does not perform a request. ureq's overall timeout does not override
    /// `timeout_connect`, whose default is 30 s, and it cannot interrupt DNS.
    /// Connect is capped at the same 500 ms so the live path does not keep
    /// that default. Read and write use the overall timeout.
    pub fn connect(endpoint: &str) -> Result<Self, JevError> {
        if endpoint.is_empty() {
            return Err(JevError::Transport);
        }
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(TOTAL_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .build();
        Ok(Self {
            transport: UreqTransport {
                endpoint: endpoint.to_string(),
                agent,
            },
        })
    }
}

/// Query encoded as the bytes [`JevClient`] sends. Fixtures must use this.
pub enum JevQuery<'a> {
    Choice {
        prompt: &'a str,
        options: &'a [String],
    },
    Noul {
        proposition: &'a str,
    },
    Score {
        subject: &'a str,
    },
    BatchNoul {
        propositions: &'a [String],
    },
}

/// Canonical JSON for `query`. Key order is the struct field order.
pub fn request_bytes(query: &JevQuery<'_>) -> Result<Vec<u8>, JevError> {
    match query {
        JevQuery::Choice { prompt, options } => to_vec(&ChoiceWire {
            kind: "choice",
            prompt,
            options,
        }),
        JevQuery::Noul { proposition } => to_vec(&NoulWire {
            kind: "noul",
            proposition,
        }),
        JevQuery::Score { subject } => to_vec(&ScoreWire {
            kind: "score",
            subject,
        }),
        JevQuery::BatchNoul { propositions } => to_vec(&BatchWire {
            kind: "batch-noul",
            propositions,
        }),
    }
}

impl<T: JevTransport> DecisionPort for JevClient<T> {
    fn evaluate_choice(&self, query: &ChoiceQuery) -> Result<DecisionOutcome, DecisionPortError> {
        if query.prompt.is_empty()
            || query.options.is_empty()
            || query.options.iter().any(String::is_empty)
        {
            return Err(DecisionPortError::EmptyQuery);
        }
        self.decide(&JevQuery::Choice {
            prompt: &query.prompt,
            options: &query.options,
        })
    }

    fn evaluate_noul(&self, query: &NoulQuery) -> Result<DecisionOutcome, DecisionPortError> {
        if query.proposition.is_empty() {
            return Err(DecisionPortError::EmptyQuery);
        }
        self.decide(&JevQuery::Noul {
            proposition: &query.proposition,
        })
    }

    fn evaluate_score(&self, query: &ScoreQuery) -> Result<DecisionOutcome, DecisionPortError> {
        if query.subject.is_empty() {
            return Err(DecisionPortError::EmptyQuery);
        }
        self.decide(&JevQuery::Score {
            subject: &query.subject,
        })
    }

    fn evaluate_batch_noul(
        &self,
        query: &BatchNoulQuery,
    ) -> Result<Vec<DecisionOutcome>, DecisionPortError> {
        if query.propositions.is_empty() || query.propositions.iter().any(String::is_empty) {
            return Err(DecisionPortError::EmptyQuery);
        }
        if query.propositions.len() > MAX_BATCH_PROPOSITIONS {
            return Err(DecisionPortError::BatchTooLarge {
                len: query.propositions.len(),
            });
        }
        let outcome = self.decide(&JevQuery::BatchNoul {
            propositions: &query.propositions,
        })?;
        // One response outcome applies to every proposition.
        Ok(vec![outcome; query.propositions.len()])
    }
}

/// Temporary collapse into [`DecisionPortError::Unscripted`].
///
/// The engine has no transport variant. Fingerprints are `timeout`,
/// `too-large`, or `malformed`. An I/O failure that is none of those three
/// uses `transport`. The fingerprint is not a path or a body.
fn port_error(error: JevError) -> DecisionPortError {
    DecisionPortError::Unscripted {
        fingerprint: error.code().to_string(),
    }
}

#[derive(Serialize)]
struct ChoiceWire<'a> {
    kind: &'static str,
    prompt: &'a str,
    options: &'a [String],
}

#[derive(Serialize)]
struct NoulWire<'a> {
    kind: &'static str,
    proposition: &'a str,
}

#[derive(Serialize)]
struct ScoreWire<'a> {
    kind: &'static str,
    subject: &'a str,
}

#[derive(Serialize)]
struct BatchWire<'a> {
    kind: &'static str,
    propositions: &'a [String],
}

fn to_vec(value: &impl Serialize) -> Result<Vec<u8>, JevError> {
    serde_json::to_vec(value).map_err(|_| JevError::Malformed)
}

#[derive(Deserialize)]
#[serde(tag = "outcome")]
enum WireOutcome {
    #[serde(rename = "boolean")]
    Boolean { value: bool },
    #[serde(rename = "selected")]
    Selected { choice: String },
    #[serde(rename = "score")]
    Score { value: f64 },
    #[serde(rename = "abstain")]
    Abstain { reason: String },
}

fn parse_outcome(body: &[u8]) -> Result<DecisionOutcome, JevError> {
    let wire: WireOutcome = serde_json::from_slice(body).map_err(|_| JevError::Malformed)?;
    match wire {
        WireOutcome::Boolean { value } => Ok(DecisionOutcome::Boolean(value)),
        WireOutcome::Selected { choice } => {
            DecisionOutcome::try_selected(choice).map_err(|_| JevError::Malformed)
        }
        WireOutcome::Score { value } => {
            DecisionOutcome::try_score(value).map_err(|_| JevError::Malformed)
        }
        WireOutcome::Abstain { reason } => {
            let reason = SanitizedReason::try_new(reason).map_err(|_| JevError::Malformed)?;
            Ok(DecisionOutcome::abstain(reason))
        }
    }
}

/// At most `MAX_RESPONSE_BYTES + 1` bytes. Over the cap, do not parse.
fn read_capped(reader: impl Read) -> Result<Vec<u8>, JevError> {
    let mut limited = reader.take((MAX_RESPONSE_BYTES as u64) + 1);
    let mut body = Vec::new();
    limited.read_to_end(&mut body).map_err(map_io)?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(JevError::TooLarge);
    }
    Ok(body)
}

fn map_io(err: std::io::Error) -> JevError {
    if is_io_timeout(&err) {
        JevError::Timeout
    } else {
        JevError::Transport
    }
}

fn classify_ureq(err: ureq::Error) -> JevError {
    if io_timeout(&err) {
        JevError::Timeout
    } else {
        JevError::Transport
    }
}

fn is_io_timeout(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    )
}

fn io_timeout(err: &ureq::Error) -> bool {
    let mut current: Option<&dyn Error> = Some(err);
    while let Some(error) = current {
        if let Some(io) = error.downcast_ref::<std::io::Error>()
            && is_io_timeout(io)
        {
            return true;
        }
        current = error.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use meshloop_domain::decision::{DecisionOutcome, SanitizedReason};
    use meshloop_engine::ports::{
        BatchNoulQuery, ChoiceQuery, DecisionPort, DecisionPortError, NoulQuery, ScoreQuery,
    };

    use super::*;

    const BOOLEAN_BODY: &[u8] = br#"{"outcome":"boolean","value":true}"#;

    fn exchange_body(body: &[u8], latency: Duration) -> Result<DecisionOutcome, JevError> {
        let request = request_bytes(&JevQuery::Noul {
            proposition: "safe",
        })
        .unwrap();
        let mut fake = JevTransportFake::default();
        fake.insert(&request, body, latency);
        JevClient::from_transport(fake).exchange(&request)
    }

    fn client_for(
        query: &JevQuery<'_>,
        body: &[u8],
        latency: Duration,
    ) -> JevClient<JevTransportFake> {
        let request = request_bytes(query).unwrap();
        let mut fake = JevTransportFake::default();
        fake.insert(request, body, latency);
        JevClient::from_transport(fake)
    }

    #[test]
    fn budget_returns_boolean_at_70ms_and_500ms_and_timeout_at_501ms() {
        assert_eq!(TOTAL_TIMEOUT, Duration::from_millis(500));
        assert_eq!(
            exchange_body(BOOLEAN_BODY, Duration::from_millis(70)).unwrap(),
            DecisionOutcome::Boolean(true)
        );
        assert_eq!(
            exchange_body(BOOLEAN_BODY, TOTAL_TIMEOUT).unwrap(),
            DecisionOutcome::Boolean(true)
        );
        // Malformed body, but the budget is checked first and the body is not parsed.
        assert_eq!(
            exchange_body(b"{", Duration::from_millis(501)),
            Err(JevError::Timeout)
        );
    }

    #[test]
    fn malformed_json_is_malformed() {
        assert_eq!(
            exchange_body(b"{", Duration::from_millis(70)),
            Err(JevError::Malformed)
        );
    }

    #[test]
    fn body_over_cap_is_too_large_and_the_cap_itself_is_parsed() {
        assert_eq!(
            exchange_body(&vec![b'{'; MAX_RESPONSE_BYTES], Duration::from_millis(70)),
            Err(JevError::Malformed)
        );
        assert_eq!(
            exchange_body(
                &vec![b'{'; MAX_RESPONSE_BYTES + 1],
                Duration::from_millis(70)
            ),
            Err(JevError::TooLarge)
        );
    }

    #[test]
    fn abstain_json_returns_sanitized_reason() {
        let outcome = exchange_body(
            br#"{"outcome":"abstain","reason":"needs a human"}"#,
            Duration::from_millis(70),
        )
        .unwrap();
        let reason = SanitizedReason::try_new("needs a human").unwrap();
        assert_eq!(outcome, DecisionOutcome::abstain(reason));
    }

    #[test]
    fn selected_and_score_json() {
        assert_eq!(
            exchange_body(br#"{"outcome":"selected","choice":"keep"}"#, Duration::ZERO).unwrap(),
            DecisionOutcome::try_selected("keep").unwrap()
        );
        assert_eq!(
            exchange_body(br#"{"outcome":"score","value":0.5}"#, Duration::ZERO).unwrap(),
            DecisionOutcome::try_score(0.5).unwrap()
        );
    }

    #[test]
    fn invalid_outcomes_are_malformed_and_do_not_panic() {
        let bodies: &[&[u8]] = &[
            br#"{"outcome":"nope"}"#,
            br#"{"outcome":"selected","choice":""}"#,
            br#"{"outcome":"score","value":null}"#,
            br#"{"outcome":"score","value":NaN}"#,
            br#"{"outcome":"score","value":1e309}"#,
            br#"{"outcome":"abstain","reason":""}"#,
            br#"{"outcome":"abstain","reason":"bad\nreason"}"#,
        ];
        for body in bodies {
            assert_eq!(
                exchange_body(body, Duration::from_millis(1)),
                Err(JevError::Malformed),
                "body {}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn request_bytes_are_canonical_and_are_what_the_client_sends() {
        let options = vec!["keep".to_string(), "drop".to_string()];
        let choice = request_bytes(&JevQuery::Choice {
            prompt: "pick",
            options: &options,
        })
        .unwrap();
        assert_eq!(
            choice,
            br#"{"kind":"choice","prompt":"pick","options":["keep","drop"]}"#
        );
        assert_eq!(
            request_bytes(&JevQuery::Noul {
                proposition: "safe",
            })
            .unwrap(),
            br#"{"kind":"noul","proposition":"safe"}"#
        );
        assert_eq!(
            request_bytes(&JevQuery::Score { subject: "diff" }).unwrap(),
            br#"{"kind":"score","subject":"diff"}"#
        );
        let propositions = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            request_bytes(&JevQuery::BatchNoul {
                propositions: &propositions,
            })
            .unwrap(),
            br#"{"kind":"batch-noul","propositions":["a","b"]}"#
        );

        let client = client_for(
            &JevQuery::Choice {
                prompt: "pick",
                options: &options,
            },
            br#"{"outcome":"selected","choice":"keep"}"#,
            Duration::from_millis(70),
        );
        assert_eq!(
            client
                .evaluate_choice(&ChoiceQuery {
                    prompt: "pick".into(),
                    options: options,
                })
                .unwrap(),
            DecisionOutcome::try_selected("keep").unwrap()
        );
    }

    #[test]
    fn empty_queries_and_oversized_batch_do_not_call_transport() {
        let client = JevClient::from_transport(JevTransportFake::default());
        assert_eq!(
            client.evaluate_choice(&ChoiceQuery {
                prompt: String::new(),
                options: vec!["keep".into()],
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            client.evaluate_choice(&ChoiceQuery {
                prompt: "pick".into(),
                options: Vec::new(),
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            client.evaluate_choice(&ChoiceQuery {
                prompt: "pick".into(),
                options: vec!["keep".into(), String::new()],
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            client.evaluate_noul(&NoulQuery {
                proposition: String::new(),
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            client.evaluate_score(&ScoreQuery {
                subject: String::new(),
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            client.evaluate_batch_noul(&BatchNoulQuery {
                propositions: Vec::new(),
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        assert_eq!(
            client.evaluate_batch_noul(&BatchNoulQuery {
                propositions: vec!["ok".into(), String::new()],
            }),
            Err(DecisionPortError::EmptyQuery)
        );
        let too_many: Vec<String> = (0..25).map(|i| format!("p{i}")).collect();
        assert_eq!(
            client.evaluate_batch_noul(&BatchNoulQuery {
                propositions: too_many,
            }),
            Err(DecisionPortError::BatchTooLarge { len: 25 })
        );
    }

    #[test]
    fn batch_applies_one_outcome_to_every_proposition() {
        let propositions = vec!["a".to_string(), "b".to_string()];
        let client = client_for(
            &JevQuery::BatchNoul {
                propositions: &propositions,
            },
            BOOLEAN_BODY,
            Duration::from_millis(70),
        );
        assert_eq!(
            client
                .evaluate_batch_noul(&BatchNoulQuery { propositions })
                .unwrap(),
            vec![
                DecisionOutcome::Boolean(true),
                DecisionOutcome::Boolean(true)
            ]
        );

        let at_limit: Vec<String> = (0..MAX_BATCH_PROPOSITIONS)
            .map(|i| format!("p{i}"))
            .collect();
        let client = client_for(
            &JevQuery::BatchNoul {
                propositions: &at_limit,
            },
            BOOLEAN_BODY,
            Duration::from_millis(500),
        );
        let got = client
            .evaluate_batch_noul(&BatchNoulQuery {
                propositions: at_limit,
            })
            .unwrap();
        assert_eq!(got.len(), MAX_BATCH_PROPOSITIONS);
        assert!(
            got.iter()
                .all(|outcome| *outcome == DecisionOutcome::Boolean(true))
        );
    }

    #[test]
    fn transport_failures_collapse_to_stable_codes() {
        let query = NoulQuery {
            proposition: "safe".into(),
        };
        let timeout = client_for(
            &JevQuery::Noul {
                proposition: "safe",
            },
            b"{",
            Duration::from_millis(501),
        );
        assert_eq!(
            timeout.evaluate_noul(&query),
            Err(DecisionPortError::Unscripted {
                fingerprint: "timeout".into(),
            })
        );
        let malformed = client_for(
            &JevQuery::Noul {
                proposition: "safe",
            },
            b"{",
            Duration::from_millis(1),
        );
        assert_eq!(
            malformed.evaluate_noul(&query),
            Err(DecisionPortError::Unscripted {
                fingerprint: "malformed".into(),
            })
        );
        let too_large = client_for(
            &JevQuery::Noul {
                proposition: "safe",
            },
            &vec![b'x'; MAX_RESPONSE_BYTES + 1],
            Duration::from_millis(1),
        );
        assert_eq!(
            too_large.evaluate_noul(&query),
            Err(DecisionPortError::Unscripted {
                fingerprint: "too-large".into(),
            })
        );
        let missing = JevClient::from_transport(JevTransportFake::default());
        assert_eq!(
            missing.evaluate_noul(&query),
            Err(DecisionPortError::Unscripted {
                fingerprint: "transport".into(),
            })
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fake_exchanges_do_not_change_tasks_or_tcp_tables() {
        // Sibling tests exit on their own threads, and `/proc/self/net/tcp`
        // lists every socket in the netns. Wait until the task count settles,
        // then compare that count and the data lines whose inode is a socket
        // fd of this process. The fake itself does not sleep or spawn, and
        // reading these proc files does not open a socket.
        for attempt in 0..3 {
            wait_until_task_count_settles();
            let before = proc_snapshot();
            run_fake_exchanges();
            let after = proc_snapshot();
            if before == after {
                return;
            }
            if attempt == 2 {
                assert_eq!(before.tasks, after.tasks, "task count changed");
                assert_eq!(before.tcp, after.tcp, "tcp data lines changed");
                assert_eq!(before.tcp6, after.tcp6, "tcp6 data lines changed");
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn run_fake_exchanges() {
        assert_eq!(
            exchange_body(BOOLEAN_BODY, Duration::from_millis(70)).unwrap(),
            DecisionOutcome::Boolean(true)
        );
        assert_eq!(
            exchange_body(BOOLEAN_BODY, Duration::from_millis(500)).unwrap(),
            DecisionOutcome::Boolean(true)
        );
        assert_eq!(
            exchange_body(b"{", Duration::from_millis(501)),
            Err(JevError::Timeout)
        );
        assert_eq!(
            exchange_body(b"{", Duration::from_millis(70)),
            Err(JevError::Malformed)
        );
        assert_eq!(
            exchange_body(
                &vec![b'{'; MAX_RESPONSE_BYTES + 1],
                Duration::from_millis(70)
            ),
            Err(JevError::TooLarge)
        );
        let outcome = exchange_body(
            br#"{"outcome":"abstain","reason":"needs a human"}"#,
            Duration::from_millis(70),
        )
        .unwrap();
        assert!(
            matches!(outcome, DecisionOutcome::Abstain(reason) if reason.as_str() == "needs a human")
        );
    }

    #[cfg(target_os = "linux")]
    fn wait_until_task_count_settles() {
        let mut previous = task_count();
        let mut stable = 0;
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(10));
            let now = task_count();
            if now == previous {
                stable += 1;
                if stable == 3 {
                    return;
                }
            } else {
                stable = 0;
                previous = now;
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[derive(PartialEq, Eq)]
    struct ProcSnapshot {
        tasks: usize,
        tcp: Vec<String>,
        tcp6: Vec<String>,
    }

    #[cfg(target_os = "linux")]
    fn proc_snapshot() -> ProcSnapshot {
        let inodes = process_socket_inodes();
        ProcSnapshot {
            tasks: task_count(),
            tcp: owned_data_lines("/proc/self/net/tcp", &inodes),
            tcp6: owned_data_lines("/proc/self/net/tcp6", &inodes),
        }
    }

    /// Inodes of `socket:[…]` fds. Skips inode 0, which is not a process fd.
    #[cfg(target_os = "linux")]
    fn process_socket_inodes() -> std::collections::BTreeSet<String> {
        let mut inodes = std::collections::BTreeSet::new();
        let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
            return inodes;
        };
        for entry in entries.flatten() {
            let Ok(target) = std::fs::read_link(entry.path()) else {
                continue;
            };
            let text = target.to_string_lossy();
            let Some(inode) = text
                .strip_prefix("socket:[")
                .and_then(|rest| rest.strip_suffix(']'))
            else {
                continue;
            };
            if inode != "0" {
                inodes.insert(inode.to_string());
            }
        }
        inodes
    }

    #[cfg(target_os = "linux")]
    fn owned_data_lines(path: &str, inodes: &std::collections::BTreeSet<String>) -> Vec<String> {
        proc_data_lines(path)
            .into_iter()
            .filter(|line| {
                tcp_inode(line).is_some_and(|inode| inode != "0" && inodes.contains(inode))
            })
            .collect()
    }

    /// Tenth whitespace field of a `/proc/net/tcp` data line.
    #[cfg(target_os = "linux")]
    fn tcp_inode(line: &str) -> Option<&str> {
        line.split_whitespace().nth(9)
    }

    #[cfg(target_os = "linux")]
    fn task_count() -> usize {
        std::fs::read_dir("/proc/self/task")
            .expect("read /proc/self/task")
            .collect::<Result<Vec<_>, _>>()
            .expect("task entry")
            .len()
    }

    /// Lines after the header. Reading this file does not open a socket.
    #[cfg(target_os = "linux")]
    fn proc_data_lines(path: &str) -> Vec<String> {
        let text = std::fs::read_to_string(path).unwrap_or_else(|err| panic!("{path}: {err}"));
        text.lines().skip(1).map(str::to_string).collect()
    }
}
