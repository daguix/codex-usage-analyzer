use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::Value;
use walkdir::WalkDir;

#[derive(Clone, Debug, Default, Serialize)]
pub struct UsageEvent {
    pub captured_at: DateTime<Utc>,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub lifetime_total_tokens: Option<u64>,
    pub context_total: Option<u64>,
    pub limit_5h_percent_left: Option<f64>,
    pub limit_weekly_percent_left: Option<f64>,
    pub model: Option<String>,
    pub directory: Option<String>,
    pub session_id: Option<String>,
    pub codex_version: Option<String>,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    pub events: Vec<UsageEvent>,
    pub files: usize,
    pub invalid_lines: usize,
}

#[derive(Debug, Default)]
struct FileResult {
    events: Vec<UsageEvent>,
    invalid_lines: usize,
}

#[derive(Clone, Debug, Default)]
struct Context {
    model: Option<String>,
    directory: Option<String>,
    session_id: Option<String>,
    codex_version: Option<String>,
}

pub fn scan_rollouts(root: &Path) -> Result<ScanResult> {
    if !root.exists() {
        return Ok(ScanResult::default());
    }
    let files: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy();
            name.starts_with("rollout-") && name.ends_with(".jsonl")
        })
        .map(|entry| entry.into_path())
        .collect();

    let parsed: Vec<Result<FileResult>> = files.par_iter().map(|path| scan_file(path)).collect();
    let mut result = ScanResult {
        files: files.len(),
        ..ScanResult::default()
    };
    for file in parsed {
        let file = file?;
        result.invalid_lines += file.invalid_lines;
        result.events.extend(file.events);
    }
    Ok(result)
}

fn scan_file(path: &Path) -> Result<FileResult> {
    let reader = BufReader::new(File::open(path)?);
    let mut context = Context::default();
    let mut result = FileResult::default();
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                result.invalid_lines += 1;
                continue;
            }
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                result.invalid_lines += 1;
                continue;
            }
        };
        parse_value(&value, &mut context, &mut result.events);
    }
    // The Python implementation's SQLite unique index suppresses identical
    // token events from the same rollout source. Preserve that behavior in
    // memory so reports remain compatible without a database.
    let mut seen = HashSet::new();
    result.events.retain(|event| {
        seen.insert((
            event.captured_at,
            event.total_tokens,
            event.input_tokens,
            event.cached_input_tokens,
            event.output_tokens,
            event.reasoning_output_tokens,
            event.session_id.clone(),
        ))
    });
    Ok(result)
}

fn parse_value(value: &Value, context: &mut Context, events: &mut Vec<UsageEvent>) {
    let Some(item_type) = value.get("type").and_then(Value::as_str) else {
        return;
    };
    let payload = value.get("payload").unwrap_or(&Value::Null);
    match item_type {
        "session_meta" => {
            replace_string(&mut context.session_id, payload.get("id"));
            replace_string(&mut context.directory, payload.get("cwd"));
            replace_string(&mut context.codex_version, payload.get("cli_version"));
        }
        "turn_context" => {
            replace_string(&mut context.model, payload.get("model"));
            replace_string(&mut context.directory, payload.get("cwd"));
        }
        "event_msg" if payload.get("type").and_then(Value::as_str) == Some("token_count") => {
            let event_payload = payload
                .get("payload")
                .filter(|value| value.is_object())
                .unwrap_or(payload);
            let Some(timestamp) = value
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_timestamp)
            else {
                return;
            };
            let info = event_payload.get("info").unwrap_or(&Value::Null);
            let last = info.get("last_token_usage").unwrap_or(&Value::Null);
            let total = info.get("total_token_usage").unwrap_or(&Value::Null);
            let limits = event_payload.get("rate_limits").unwrap_or(&Value::Null);
            events.push(UsageEvent {
                captured_at: timestamp,
                total_tokens: number(last, "total_tokens"),
                input_tokens: number(last, "input_tokens"),
                cached_input_tokens: number(last, "cached_input_tokens"),
                output_tokens: number(last, "output_tokens"),
                reasoning_output_tokens: number(last, "reasoning_output_tokens"),
                lifetime_total_tokens: optional_number(total, "total_tokens"),
                context_total: optional_number(info, "model_context_window"),
                limit_5h_percent_left: percent_left(limits.get("primary")),
                limit_weekly_percent_left: percent_left(limits.get("secondary")),
                model: context.model.clone(),
                directory: context.directory.clone(),
                session_id: context.session_id.clone(),
                codex_version: context.codex_version.clone(),
            });
        }
        _ => {}
    }
}

fn replace_string(target: &mut Option<String>, value: Option<&Value>) {
    if let Some(value) = value
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    {
        *target = Some(value.to_owned());
    }
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.to_utc())
}

fn number(value: &Value, key: &str) -> u64 {
    optional_number(value, key).unwrap_or(0)
}

fn optional_number(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
    })
}

fn percent_left(value: Option<&Value>) -> Option<f64> {
    value?
        .get("used_percent")?
        .as_f64()
        .map(|used| (100.0 - used).clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_token_payload_and_context() {
        let mut context = Context::default();
        let mut events = Vec::new();
        parse_value(
            &serde_json::json!({"type":"session_meta","payload":{"id":"s","cwd":"/tmp/p"}}),
            &mut context,
            &mut events,
        );
        parse_value(
            &serde_json::json!({"type":"turn_context","payload":{"model":"gpt-5.2"}}),
            &mut context,
            &mut events,
        );
        parse_value(
            &serde_json::json!({
                "timestamp":"2026-09-22T10:00:00Z",
                "type":"event_msg",
                "payload":{"type":"token_count","payload":{"info":{"last_token_usage":{
                    "total_tokens":16,"input_tokens":12,"cached_input_tokens":2,"output_tokens":4
                }}}}
            }),
            &mut context,
            &mut events,
        );
        assert_eq!(events[0].total_tokens, 16);
        assert_eq!(events[0].model.as_deref(), Some("gpt-5.2"));
        assert_eq!(events[0].session_id.as_deref(), Some("s"));
    }
}
