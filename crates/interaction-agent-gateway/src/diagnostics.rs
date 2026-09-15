//! 子程序診斷輸出（stderr）的共用處理（D16）。
//!
//! 三個連接器（codex app-server／codex exec／claude）以前各做各的：
//! app-server 讀了就丟、另外兩個各自留 2000 字未脫敏的 tail。這裡收斂成
//! 同一份實作，並且守住簡報裡「diagnostic」這一類紀錄的承諾：
//!
//! - **有界**：記憶體裡永遠只留最後 [`TAIL_MAX_BYTES`] bytes 的行，逐行再受
//!   [`LINE_MAX_CHARS`] 限制；丟棄量與截斷事實都要數出來，不是靜默吃掉。
//! - **脫敏**：[`redact`] 在 **push 當下**就跑完，所以記憶體裡不存在未脫敏
//!   的原文——之後任何路徑（事件、log、SessionClosed detail）都不可能外洩。
//! - **可截斷**：截斷／丟棄一律標 `truncated`，不假裝自己看完了全部。
//!
//! 誠實階梯：stderr **不是**業務結局的證據。這個模組只負責「把 agent 說過
//! 什麼誠實地留下來」，絕不因為裡面有 "error" 字樣就把一輪判成失敗——結局
//! 仍然只由 exit code／協定事件決定（見 `codex_exec::drain_outcome_event`）。

use crate::GatewayEvent;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::ChildStderr;
use tokio::task::JoinHandle;

/// 記憶體裡保留的 tail 上限（bytes）。超過就從最舊的行丟起。
pub const TAIL_MAX_BYTES: usize = 4096;
/// 逐行上限（chars）。超過就截斷並標記。
pub const LINE_MAX_CHARS: usize = 500;
/// 前幾行以 warn 記錄，之後降為 debug（agent 一次噴幾 MB 時不淹沒 log）。
pub const WARN_LINES: u64 = 20;
/// `StderrCaptured` 事件裡 tail 的上限（chars）。
pub const EVENT_TAIL_MAX_CHARS: usize = 600;
/// 等 reader task 收乾的有界上限。收不乾就用當下快照，絕不無限等待。
pub const READER_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

/// 逐行截斷後附加的標記（明示這行沒有被完整保留）。
const LINE_TRUNCATION_MARK: &str = "…[truncated]";
/// 脫敏前先砍掉的超長行上限（chars）。子程序可能一行噴好幾 MB；脫敏是
/// 線性掃描，先有界化才不會被單行拖垮。遠大於 [`LINE_MAX_CHARS`]，所以
/// 任何在行首附近出現的 secret 都還在掃描範圍內。
const REDACT_INPUT_MAX_CHARS: usize = 4000;

const REDACTED_TOKEN: &str = "[redacted-token]";
const REDACTED_VALUE: &str = "[redacted]";

/// 一份 stderr 快照（純資料，可安全放進事件與 log）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StderrSnapshot {
    /// 已脫敏、已有界的最後數行（以 `\n` 相接）。
    pub tail: String,
    /// 讀到過的行數（含已丟棄的）。
    pub lines_seen: u64,
    /// 讀到過的原始位元組數（含已丟棄的）。
    pub bytes_seen: u64,
    /// 因為有界而被丟掉的行數。
    pub lines_dropped: u64,
    /// 有任何行被截斷或丟棄。
    pub truncated: bool,
}

impl StderrSnapshot {
    /// 事件用的 tail：最後 [`EVENT_TAIL_MAX_CHARS`] 個字（已脫敏）。
    pub fn event_tail(&self) -> String {
        self.tail_suffix(EVENT_TAIL_MAX_CHARS)
    }

    /// 最後 `max_chars` 個字（已脫敏）。
    pub fn tail_suffix(&self, max_chars: usize) -> String {
        let trimmed = self.tail.trim();
        let count = trimmed.chars().count();
        if count <= max_chars {
            return trimmed.to_string();
        }
        trimmed.chars().skip(count - max_chars).collect()
    }

    /// 完全沒有讀到任何 stderr。
    pub fn is_empty(&self) -> bool {
        self.lines_seen == 0
    }

    /// 轉成正規化事件。**沒讀到任何一行就不發事件**（沉默不等於有話沒說）。
    pub fn into_event(self) -> Option<GatewayEvent> {
        if self.is_empty() {
            return None;
        }
        Some(GatewayEvent::StderrCaptured {
            tail: self.event_tail(),
            lines_seen: self.lines_seen,
            bytes_seen: self.bytes_seen,
            lines_dropped: self.lines_dropped,
            truncated: self.truncated,
        })
    }
}

#[derive(Debug, Default)]
struct TailState {
    /// 已脫敏、已逐行有界的保留行。
    lines: VecDeque<String>,
    /// `lines` 目前佔用的位元組數（含行間換行）。
    bytes: usize,
    lines_seen: u64,
    bytes_seen: u64,
    lines_dropped: u64,
    truncated: bool,
}

/// 共用的有界脫敏 stderr tail。`clone` 共用同一份狀態（reader task 與收攤
/// 路徑各持一個 handle）。
#[derive(Debug, Clone, Default)]
pub struct StderrTail {
    state: Arc<Mutex<TailState>>,
}

impl StderrTail {
    pub fn new() -> Self {
        Self::default()
    }

    /// 收下一行 stderr：**先脫敏**再入列，回傳這行的序號（1 起算）。
    ///
    /// 呼叫端只會拿到序號，永遠拿不到未脫敏的原文——要記 log 請自己對原始
    /// 字串呼叫 [`redact`]（`spawn_reader` 就是這樣做的）。
    pub fn push_line(&self, raw: &str) -> u64 {
        let (line, line_truncated) = bound_line(raw);
        let mut state = self.state.lock().expect("stderr tail lock");
        state.lines_seen = state.lines_seen.saturating_add(1);
        state.bytes_seen = state.bytes_seen.saturating_add(raw.len() as u64);
        if line_truncated {
            state.truncated = true;
        }
        state.bytes = state.bytes.saturating_add(line.len() + 1);
        state.lines.push_back(line);
        while state.bytes > TAIL_MAX_BYTES {
            let Some(dropped) = state.lines.pop_front() else {
                break;
            };
            state.bytes = state.bytes.saturating_sub(dropped.len() + 1);
            state.lines_dropped = state.lines_dropped.saturating_add(1);
            state.truncated = true;
        }
        state.lines_seen
    }

    pub fn snapshot(&self) -> StderrSnapshot {
        let state = self.state.lock().expect("stderr tail lock");
        let tail = state
            .lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");
        StderrSnapshot {
            tail,
            lines_seen: state.lines_seen,
            bytes_seen: state.bytes_seen,
            lines_dropped: state.lines_dropped,
            truncated: state.truncated,
        }
    }
}

/// 脫敏＋逐行有界。回傳 (可保留的行, 這行是否被截斷)。
fn bound_line(raw: &str) -> (String, bool) {
    let raw_chars = raw.chars().count();
    let capped: String = if raw_chars > REDACT_INPUT_MAX_CHARS {
        raw.chars().take(REDACT_INPUT_MAX_CHARS).collect()
    } else {
        raw.to_string()
    };
    // 脫敏一定在截斷之前：截斷只會砍掉尾巴，不可能把已經遮掉的東西露出來。
    let redacted = redact(&capped);
    let count = redacted.chars().count();
    if count <= LINE_MAX_CHARS && raw_chars <= REDACT_INPUT_MAX_CHARS {
        return (redacted, false);
    }
    let mut kept: String = redacted.chars().take(LINE_MAX_CHARS).collect();
    kept.push_str(LINE_TRUNCATION_MARK);
    (kept, true)
}

/// 持續逐行讀 stderr 到 EOF（**永不塞管線**：子程序噴幾 MB 也不會卡住）。
///
/// `session_hint` 只用於 log 關聯，不是可信身分。
pub fn spawn_reader(stderr: ChildStderr, tail: StderrTail, session_hint: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let seq = tail.push_line(&line);
                    // 前幾行值得注意；之後降級，避免 agent 的診斷輸出淹沒 log。
                    // 兩條路徑都記脫敏後的文字（tracing 巨集只在該層級啟用時
                    // 才會求值，所以關掉 debug 不會付出重複脫敏的成本）。
                    if seq <= WARN_LINES {
                        tracing::warn!(
                            target: "agent.stderr",
                            session = %session_hint,
                            line = %redact(&line),
                            "agent subprocess stderr"
                        );
                    } else {
                        tracing::debug!(
                            target: "agent.stderr",
                            session = %session_hint,
                            line = %redact(&line),
                            "agent subprocess stderr"
                        );
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    tracing::debug!(
                        target: "agent.stderr",
                        session = %session_hint,
                        error = %error,
                        "agent subprocess stderr read ended"
                    );
                    break;
                }
            }
        }
        let snapshot = tail.snapshot();
        tracing::info!(
            target: "agent.stderr",
            session = %session_hint,
            lines_seen = snapshot.lines_seen,
            bytes_seen = snapshot.bytes_seen,
            lines_dropped = snapshot.lines_dropped,
            truncated = snapshot.truncated,
            "agent subprocess stderr closed"
        );
    })
}

/// 收攤時把 reader 收乾（**有界** [`READER_DRAIN_TIMEOUT`]）再取快照。
/// 收不乾就用當下快照並照實標 `truncated`——絕不為了等齊而卡住收場路徑。
pub async fn drain_stderr(tail: &StderrTail, reader: Option<JoinHandle<()>>) -> StderrSnapshot {
    if let Some(reader) = reader {
        if tokio::time::timeout(READER_DRAIN_TIMEOUT, reader)
            .await
            .is_err()
        {
            let mut snapshot = tail.snapshot();
            snapshot.truncated = true;
            return snapshot;
        }
    }
    tail.snapshot()
}

/// 純函式脫敏。輸入是子程序的原始輸出（不可信），輸出可安全進事件／log。
///
/// 覆蓋：ANSI escape、`Bearer <token>`、`Authorization:`／`token=`／`api_key=`
/// 之後的值、`iat-*`／`sk-*`／`ghp_*`／`xox[abp]-*` 形狀的 token，以及
/// `/Users/<name>/`、`/home/<name>/`（→ `~/`，不外洩本機使用者名稱）。
pub fn redact(input: &str) -> String {
    let stripped = strip_ansi(input);
    let bearer = redact_bearer(&stripped);
    let marked = redact_marker_values(&bearer);
    let tokens = redact_token_shapes(&marked);
    redact_home_paths(&tokens)
}

/// 去掉 ANSI escape（CSI／OSC／單字元 escape）。
fn strip_ansi(input: &str) -> String {
    if !input.contains('\u{1b}') {
        return input.to_string();
    }
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\u{1b}' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        i += 1;
        match chars.get(i) {
            Some('[') => {
                i += 1;
                // CSI：參數／中介位元組之後是 0x40..=0x7E 的終結位元組。
                while i < chars.len() && !matches!(chars[i], '\u{40}'..='\u{7e}') {
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
            }
            Some(']') => {
                i += 1;
                // OSC：BEL 或 ESC \ 結束。
                while i < chars.len() {
                    if chars[i] == '\u{7}' {
                        i += 1;
                        break;
                    }
                    if chars[i] == '\u{1b}' && chars.get(i + 1) == Some(&'\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            Some(_) => i += 1,
            None => {}
        }
    }
    out
}

fn is_ws(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// `Bearer <token>` → `Bearer [redacted]`（保留原本的大小寫拼法）。
fn redact_bearer(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let word_boundary = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
        if word_boundary && matches_ci(&chars, i, "bearer") {
            let word_end = i + 6;
            let after_word = chars.get(word_end).copied();
            if after_word.is_some_and(is_ws) {
                let mut j = word_end;
                while j < chars.len() && is_ws(chars[j]) {
                    j += 1;
                }
                let value_start = j;
                while j < chars.len() && !chars[j].is_whitespace() {
                    j += 1;
                }
                if j > value_start {
                    out.extend(&chars[i..value_start]);
                    out.push_str(REDACTED_VALUE);
                    i = j;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 已知的 `Authorization:` scheme：保留 scheme 本身，遮掉憑證。
const AUTH_SCHEMES: &[&str] = &["bearer", "basic", "token", "digest"];
/// 之後的值一律要遮掉的標記。
const VALUE_MARKERS: &[&str] = &["authorization:", "token=", "api_key="];

fn redact_marker_values(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let mut found: Option<&str> = None;
        for marker in VALUE_MARKERS {
            if matches_ci(&chars, i, marker) {
                found = Some(marker);
                break;
            }
        }
        let Some(marker) = found else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        let is_auth = marker == "authorization:";
        let mut j = i + marker.chars().count();
        out.extend(&chars[i..j]);
        j = copy_while(&chars, j, &mut out, is_ws);
        if let Some(quote) = chars.get(j).filter(|c| **c == '"' || **c == '\'') {
            out.push(*quote);
            j += 1;
        }
        if is_auth {
            for scheme in AUTH_SCHEMES {
                if !matches_ci(&chars, j, scheme) {
                    continue;
                }
                let scheme_end = j + scheme.chars().count();
                // 只有真的是獨立的 scheme 字（後面接空白）才保留。
                if chars.get(scheme_end).copied().is_some_and(is_ws) {
                    out.extend(&chars[j..scheme_end]);
                    j = copy_while(&chars, scheme_end, &mut out, is_ws);
                }
                break;
            }
        }
        let value_start = j;
        while j < chars.len() && !is_value_terminator(chars[j]) {
            j += 1;
        }
        if j > value_start {
            out.push_str(REDACTED_VALUE);
        }
        i = j;
    }
    out
}

fn is_value_terminator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';' | '&' | '}' | ')')
}

fn copy_while(
    chars: &[char],
    mut i: usize,
    out: &mut String,
    pred: impl Fn(char) -> bool,
) -> usize {
    while i < chars.len() && pred(chars[i]) {
        out.push(chars[i]);
        i += 1;
    }
    i
}

/// 依 token 形狀遮蔽：`iat-session-<hex>`、`iat-<words><hex≥16>`、
/// `sk-…`、`ghp_…`、`xox[abp]-…`。
fn redact_token_shapes(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        if let Some(end) = token_shape_at(&chars, i) {
            out.push_str(REDACTED_TOKEN);
            i = end;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn token_shape_at(chars: &[char], i: usize) -> Option<usize> {
    if i > 0 && (chars[i - 1].is_ascii_alphanumeric() || chars[i - 1] == '-' || chars[i - 1] == '_')
    {
        return None;
    }
    if matches_ci(chars, i, "iat-session-") {
        let start = i + "iat-session-".chars().count();
        let end = run_end(chars, start, is_hex);
        if end > start {
            return Some(end);
        }
    }
    if matches_ci(chars, i, "iat-") {
        // `iat-[a-z-]*[0-9a-f]{16,}`：先取最長的候選範圍，再找出讓尾段
        // 全是 hex 且長度 ≥16 的最小切點（等同 regex 的回溯）。
        let body_start = i + 4;
        let body_end = run_end(chars, body_start, |c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
        });
        for split in body_start..body_end {
            if body_end - split < 16 {
                break;
            }
            let prefix_ok = chars[body_start..split]
                .iter()
                .all(|c| c.is_ascii_lowercase() || *c == '-');
            if prefix_ok && chars[split..body_end].iter().all(|c| is_hex(*c)) {
                return Some(body_end);
            }
        }
    }
    for (prefix, min_len, charset) in [
        ("sk-", 8usize, TokenCharset::Base64Urlish),
        ("ghp_", 8, TokenCharset::AlnumUnderscore),
        ("xoxa-", 8, TokenCharset::AlnumDash),
        ("xoxb-", 8, TokenCharset::AlnumDash),
        ("xoxp-", 8, TokenCharset::AlnumDash),
    ] {
        if !matches_ci(chars, i, prefix) {
            continue;
        }
        let start = i + prefix.chars().count();
        let end = run_end(chars, start, |c| charset.contains(c));
        if end - start >= min_len {
            return Some(end);
        }
    }
    None
}

#[derive(Clone, Copy)]
enum TokenCharset {
    Base64Urlish,
    AlnumUnderscore,
    AlnumDash,
}

impl TokenCharset {
    fn contains(self, c: char) -> bool {
        match self {
            Self::Base64Urlish => c.is_ascii_alphanumeric() || c == '_' || c == '-',
            Self::AlnumUnderscore => c.is_ascii_alphanumeric() || c == '_',
            Self::AlnumDash => c.is_ascii_alphanumeric() || c == '-',
        }
    }
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

fn run_end(chars: &[char], mut i: usize, pred: impl Fn(char) -> bool) -> usize {
    while i < chars.len() && pred(chars[i]) {
        i += 1;
    }
    i
}

/// 家目錄前綴：`/Users/<name>`（macOS）與 `/home/<name>`（Linux）。
const HOME_PREFIXES: &[&str] = &["/Users/", "/home/"];

/// `/Users/<name>` 與 `/home/<name>` → `~`（本機使用者名稱不進診斷紀錄）。
fn redact_home_paths(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < chars.len() {
        let mut matched = false;
        for prefix in HOME_PREFIXES {
            if !matches_exact(&chars, i, prefix) {
                continue;
            }
            let seg_start = i + prefix.chars().count();
            let seg_end = run_end(&chars, seg_start, |c| {
                !matches!(c, '/' | '"' | '\'' | ':' | ',' | ';' | ')' | ']' | '}')
                    && !c.is_whitespace()
            });
            if seg_end > seg_start {
                out.push('~');
                i = seg_end;
                matched = true;
            }
            break;
        }
        if matched {
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn matches_ci(chars: &[char], at: usize, needle: &str) -> bool {
    let mut i = at;
    for want in needle.chars() {
        match chars.get(i) {
            Some(got) if got.eq_ignore_ascii_case(&want) => i += 1,
            _ => return false,
        }
    }
    true
}

fn matches_exact(chars: &[char], at: usize, needle: &str) -> bool {
    let mut i = at;
    for want in needle.chars() {
        match chars.get(i) {
            Some(got) if *got == want => i += 1,
            _ => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 表驅動：每種 pattern 一筆，外加不該誤傷的正常文字。
    #[test]
    fn redaction_covers_every_pattern_without_mangling_ordinary_text() {
        let cases: &[(&str, &str, &str)] = &[
            (
                "bearer",
                "GET /v1/status Authorization: Bearer abc123def456",
                "GET /v1/status Authorization: Bearer [redacted]",
            ),
            (
                "bare bearer without a header name",
                "sending Bearer eyJhbGciOiJIUzI1NiJ9 now",
                "sending Bearer [redacted] now",
            ),
            (
                "iat session token",
                "resume iat-session-0a1b2c3d4e5f failed",
                "resume [redacted-token] failed",
            ),
            (
                "iat long hex token",
                "token iat-agent-0123456789abcdef0123 rejected",
                "token [redacted-token] rejected",
            ),
            (
                "openai style key",
                "spawn failed with sk-abcdEFGH1234_x-9 in argv",
                "spawn failed with [redacted-token] in argv",
            ),
            (
                // `api_key=` 這個標記先命中，所以遮罩是值標記的那一種——
                // 兩條規則都把 secret 蓋掉了，這裡只是釘住實際輸出。
                "api key env assignment",
                "OPENAI_API_KEY=sk-abcdEFGH1234_x-9 is set",
                "OPENAI_API_KEY=[redacted] is set",
            ),
            (
                "github token",
                "remote: ghp_ABCdef0123456789XYZ denied",
                "remote: [redacted-token] denied",
            ),
            (
                "slack token",
                "hook xoxb-123456789012-abcdefGHIJKL used",
                "hook [redacted-token] used",
            ),
            (
                "token= value",
                "GET /v1/events?token=s3cr3t-value&x=1",
                "GET /v1/events?token=[redacted]&x=1",
            ),
            (
                "api_key= value",
                "config api_key=\"zzzz1111\" loaded",
                "config api_key=\"[redacted]\" loaded",
            ),
            (
                "authorization without a known scheme",
                "Authorization: OpaqueCredential99",
                "Authorization: [redacted]",
            ),
            (
                "macos home path",
                "reading /Users/someone/secret/notes.md",
                "reading ~/secret/notes.md",
            ),
            (
                "linux home path",
                "reading /home/someone/secret/notes.md",
                "reading ~/secret/notes.md",
            ),
            (
                "home path without a trailing segment",
                "cwd=/Users/someone",
                "cwd=~",
            ),
            (
                "ansi escapes",
                "\u{1b}[31mfatal\u{1b}[0m: boom",
                "fatal: boom",
            ),
            // 以下都是正常文字：一個字都不該被改。
            (
                "ordinary prose is untouched",
                "compiling interaction-agent-gateway v0.8.0 (2 warnings)",
                "compiling interaction-agent-gateway v0.8.0 (2 warnings)",
            ),
            (
                "a bare word 'token' is not a secret",
                "the token budget was exceeded",
                "the token budget was exceeded",
            ),
            (
                "short sk- words are not keys",
                "sk-1 and skate are fine",
                "sk-1 and skate are fine",
            ),
            (
                "system paths outside home stay readable",
                "/usr/local/bin/codex not found",
                "/usr/local/bin/codex not found",
            ),
            (
                "an iat- word without enough hex stays",
                "iat-agent-beta started",
                "iat-agent-beta started",
            ),
        ];
        for (name, input, want) in cases {
            assert_eq!(redact(input), *want, "case: {name}");
        }
    }

    /// 脫敏發生在 push 當下：記憶體裡的 tail 永遠不含原文 secret。
    #[test]
    fn secrets_never_reach_memory_unredacted() {
        let tail = StderrTail::new();
        tail.push_line("Authorization: Bearer abc123def456");
        tail.push_line("workdir=/Users/someone/secret");
        let snapshot = tail.snapshot();
        assert!(!snapshot.tail.contains("abc123def456"), "{snapshot:?}");
        assert!(!snapshot.tail.contains("/Users/someone"), "{snapshot:?}");
        assert!(snapshot.tail.contains("Bearer [redacted]"), "{snapshot:?}");
        assert!(snapshot.tail.contains("~/secret"), "{snapshot:?}");
    }

    /// 有界：一百萬字級的輸入之後，記憶體裡仍然只有 ≤ TAIL_MAX_BYTES。
    #[test]
    fn the_tail_stays_bounded_under_a_flood() {
        let tail = StderrTail::new();
        for i in 0..100_000u64 {
            tail.push_line(&format!("line {i} of noisy agent diagnostics"));
        }
        let snapshot = tail.snapshot();
        assert!(
            snapshot.tail.len() <= TAIL_MAX_BYTES,
            "tail is {} bytes",
            snapshot.tail.len()
        );
        assert_eq!(snapshot.lines_seen, 100_000);
        assert!(snapshot.truncated, "dropping lines must be disclosed");
        let kept = snapshot.tail.lines().count() as u64;
        assert_eq!(
            snapshot.lines_dropped + kept,
            snapshot.lines_seen,
            "every line is either kept or counted as dropped"
        );
        assert!(snapshot.bytes_seen > 3_000_000, "{snapshot:?}");
        // 保留的是**最後**幾行，不是最早的幾行。
        assert!(snapshot.tail.contains("line 99999 "), "{}", snapshot.tail);
        assert!(!snapshot.tail.contains("line 0 "), "{}", snapshot.tail);
    }

    /// 逐行上限：超長的一行被截斷並標記，且不會把 tail 撐爆。
    #[test]
    fn a_single_enormous_line_is_truncated_and_marked() {
        let tail = StderrTail::new();
        tail.push_line(&"x".repeat(5_000_000));
        let snapshot = tail.snapshot();
        assert!(snapshot.truncated);
        assert!(
            snapshot.tail.ends_with(LINE_TRUNCATION_MARK),
            "{snapshot:?}"
        );
        assert!(
            snapshot.tail.chars().count() <= LINE_MAX_CHARS + LINE_TRUNCATION_MARK.chars().count()
        );
        assert_eq!(snapshot.lines_seen, 1);
        assert_eq!(snapshot.bytes_seen, 5_000_000);
    }

    /// 沒有 stderr 就不發事件（沉默 ≠ 有話沒說）。
    #[test]
    fn an_empty_tail_produces_no_event() {
        assert!(StderrTail::new().snapshot().into_event().is_none());
        let tail = StderrTail::new();
        tail.push_line("warning: something");
        let event = tail.snapshot().into_event().expect("one line ⇒ one event");
        match event {
            GatewayEvent::StderrCaptured {
                tail,
                lines_seen,
                lines_dropped,
                truncated,
                ..
            } => {
                assert_eq!(tail, "warning: something");
                assert_eq!(lines_seen, 1);
                assert_eq!(lines_dropped, 0);
                assert!(!truncated);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    /// 事件 tail 有界（≤ EVENT_TAIL_MAX_CHARS），且取的是最後一段。
    #[test]
    fn the_event_tail_is_bounded_to_the_last_lines() {
        let tail = StderrTail::new();
        for i in 0..200u32 {
            tail.push_line(&format!("noise {i}"));
        }
        tail.push_line("the very last thing the agent said");
        let snapshot = tail.snapshot();
        let event_tail = snapshot.event_tail();
        assert!(event_tail.chars().count() <= EVENT_TAIL_MAX_CHARS);
        assert!(
            event_tail.ends_with("the very last thing the agent said"),
            "{event_tail}"
        );
    }

    #[tokio::test]
    async fn draining_without_a_reader_task_still_snapshots() {
        let tail = StderrTail::new();
        tail.push_line("hello");
        let snapshot = drain_stderr(&tail, None).await;
        assert_eq!(snapshot.lines_seen, 1);
    }
}
