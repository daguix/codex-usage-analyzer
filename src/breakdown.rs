use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use rayon::prelude::*;
use serde::Serialize;
use serde_json::Value;
use walkdir::WalkDir;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Instructions,
    RepositoryCode,
    ToolOutputCode,
    ToolOutputOther,
    UserPrompts,
    AssistantText,
    ToolCalls,
    CompactionSummary,
    Unknown,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Instructions => "instructions / AGENTS.md",
            Self::RepositoryCode => "source / repository code",
            Self::ToolOutputCode => "tool outputs: code / diffs",
            Self::ToolOutputOther => "tool outputs: other",
            Self::UserPrompts => "user prompts",
            Self::AssistantText => "assistant text / reasoning",
            Self::ToolCalls => "tool calls",
            Self::CompactionSummary => "compaction / summary",
            Self::Unknown => "unknown / protocol overhead",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BreakdownRow {
    pub category: Category,
    pub estimated_input_tokens: u64,
    pub estimated_cached_input_tokens: u64,
    pub input_percent: f64,
    pub cached_percent: f64,
}

#[derive(Debug, Default, Serialize)]
pub struct Breakdown {
    pub rows: Vec<BreakdownRow>,
    pub files: usize,
    pub calls: usize,
    pub invalid_lines: usize,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    category: Category,
    tokens: u64,
}

#[derive(Debug, Default)]
struct FileBreakdown {
    totals: BTreeMap<Category, (u64, u64)>,
    calls: usize,
    invalid_lines: usize,
    input_tokens: u64,
    cached_input_tokens: u64,
}

pub fn analyze(root: &Path, since: DateTime<Utc>) -> Result<Breakdown> {
    if !root.exists() {
        return Ok(Breakdown::default());
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
    let parsed: Vec<Result<FileBreakdown>> = files
        .par_iter()
        .map(|path| analyze_file(path, since))
        .collect();
    let mut result = Breakdown {
        files: files.len(),
        ..Breakdown::default()
    };
    let mut totals = BTreeMap::<Category, (u64, u64)>::new();
    for file in parsed {
        let file = file?;
        result.calls += file.calls;
        result.invalid_lines += file.invalid_lines;
        result.input_tokens += file.input_tokens;
        result.cached_input_tokens += file.cached_input_tokens;
        for (category, (input, cached)) in file.totals {
            let total = totals.entry(category).or_default();
            total.0 += input;
            total.1 += cached;
        }
    }
    result.rows = Category::all()
        .into_iter()
        .map(|category| {
            let (input, cached) = totals.get(&category).copied().unwrap_or_default();
            BreakdownRow {
                category,
                estimated_input_tokens: input,
                estimated_cached_input_tokens: cached,
                input_percent: percent(input, result.input_tokens),
                cached_percent: percent(cached, result.cached_input_tokens),
            }
        })
        .filter(|row| row.estimated_input_tokens > 0 || row.estimated_cached_input_tokens > 0)
        .collect();
    result
        .rows
        .sort_by_key(|row| std::cmp::Reverse(row.estimated_cached_input_tokens));
    Ok(result)
}

impl Category {
    fn all() -> [Self; 9] {
        [
            Self::Instructions,
            Self::RepositoryCode,
            Self::ToolOutputCode,
            Self::ToolOutputOther,
            Self::UserPrompts,
            Self::AssistantText,
            Self::ToolCalls,
            Self::CompactionSummary,
            Self::Unknown,
        ]
    }
}

fn analyze_file(path: &Path, since: DateTime<Utc>) -> Result<FileBreakdown> {
    let mut result = FileBreakdown::default();
    let mut context = Vec::<Item>::new();
    let mut pending = Vec::<Item>::new();
    let mut calls = HashMap::<String, (String, String)>::new();
    let mut has_usage_records = false;
    let mut protocol_overhead_hint = None;
    for line in BufReader::new(File::open(path)?).lines() {
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
        let item_type = value.get("type").and_then(Value::as_str).unwrap_or("");
        let payload = value.get("payload").unwrap_or(&Value::Null);
        match item_type {
            "response_item" => {
                ingest_response_item(payload, &mut context, &mut pending, &mut calls)
            }
            "compacted" => {
                context.clear();
                pending.clear();
                if let Some(history) = payload.get("replacement_history").and_then(Value::as_array)
                {
                    for item in history {
                        ingest_history_item(item, &mut context, &calls);
                    }
                }
            }
            "token_usage_record" => {
                has_usage_records = true;
                let timestamp = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(parse_timestamp);
                let usage = payload.get("usage").unwrap_or(&Value::Null);
                if timestamp.is_some_and(|timestamp| timestamp >= since) {
                    let input = number(usage, "input_tokens");
                    let cached = number(usage, "cached_input_tokens").min(input);
                    update_overhead_hint(&context, input, &mut protocol_overhead_hint);
                    attribute(
                        &context,
                        input,
                        cached,
                        protocol_overhead_hint,
                        &mut result.totals,
                    );
                    result.calls += 1;
                    result.input_tokens += input;
                    result.cached_input_tokens += cached;
                }
                append_compact(&mut context, pending.drain(..));
            }
            // Older rollout formats only have event_msg/token_count. Prefer
            // token_usage_record when present because it has one record per
            // model response and avoids cumulative snapshots.
            "event_msg"
                if !has_usage_records
                    && payload.get("type").and_then(Value::as_str) == Some("token_count") =>
            {
                let timestamp = value
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(parse_timestamp);
                let event = payload
                    .get("payload")
                    .filter(|value| value.is_object())
                    .unwrap_or(payload);
                let usage = event
                    .get("info")
                    .and_then(|info| info.get("last_token_usage"))
                    .unwrap_or(&Value::Null);
                if timestamp.is_some_and(|timestamp| timestamp >= since) {
                    let input = number(usage, "input_tokens");
                    let cached = number(usage, "cached_input_tokens").min(input);
                    update_overhead_hint(&context, input, &mut protocol_overhead_hint);
                    attribute(
                        &context,
                        input,
                        cached,
                        protocol_overhead_hint,
                        &mut result.totals,
                    );
                    result.calls += 1;
                    result.input_tokens += input;
                    result.cached_input_tokens += cached;
                }
                append_compact(&mut context, pending.drain(..));
            }
            _ => {}
        }
    }
    Ok(result)
}

fn ingest_response_item(
    payload: &Value,
    context: &mut Vec<Item>,
    pending: &mut Vec<Item>,
    calls: &mut HashMap<String, (String, String)>,
) {
    match payload.get("type").and_then(Value::as_str).unwrap_or("") {
        "message" => {
            let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
            let target = if role == "assistant" {
                pending
            } else {
                context
            };
            for text in content_texts(payload.get("content")) {
                let category = match role {
                    "developer" | "system" => Category::Instructions,
                    "user" if looks_like_instructions(text) => Category::Instructions,
                    "user" => Category::UserPrompts,
                    "assistant" => Category::AssistantText,
                    _ => Category::Unknown,
                };
                push_item(target, category, estimate_tokens(text));
            }
        }
        "reasoning" => {
            let tokens = text_fields(payload.get("summary")) + text_fields(payload.get("content"));
            push_item(pending, Category::AssistantText, tokens);
        }
        "function_call" | "custom_tool_call" => {
            let name = payload.get("name").and_then(Value::as_str).unwrap_or("");
            let input = payload
                .get("arguments")
                .or_else(|| payload.get("input"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if let Some(id) = payload.get("call_id").and_then(Value::as_str) {
                calls.insert(id.to_owned(), (name.to_owned(), input.to_owned()));
            }
            push_item(
                pending,
                Category::ToolCalls,
                estimate_tokens(name) + estimate_tokens(input),
            );
        }
        "function_call_output" | "custom_tool_call_output" => {
            let call = payload
                .get("call_id")
                .and_then(Value::as_str)
                .and_then(|id| calls.get(id));
            for text in output_texts(payload.get("output")) {
                let category = classify_tool_output(text, call);
                push_item(context, category, estimate_tokens(text));
            }
        }
        "compaction" => push_item(context, Category::CompactionSummary, 0),
        _ => push_item(
            context,
            Category::Unknown,
            estimate_tokens(&payload.to_string()),
        ),
    }
}

fn ingest_history_item(
    item: &Value,
    context: &mut Vec<Item>,
    calls: &HashMap<String, (String, String)>,
) {
    let mut ignored_pending = Vec::new();
    let mut cloned_calls = calls.clone();
    ingest_response_item(item, context, &mut ignored_pending, &mut cloned_calls);
    append_compact(context, ignored_pending);
}

fn attribute(
    context: &[Item],
    input: u64,
    cached: u64,
    protocol_overhead_hint: Option<u64>,
    totals: &mut BTreeMap<Category, (u64, u64)>,
) {
    if input == 0 {
        return;
    }
    let raw: u64 = context.iter().map(|item| item.tokens).sum();
    let mut sized = if raw > input {
        proportional_sizes(context, input, raw)
    } else {
        context.to_vec()
    };
    if raw < input {
        let mut extra = input - raw;
        if let Some(index) = sized
            .iter()
            .rposition(|item| item.category == Category::CompactionSummary)
        {
            // Tool schemas and protocol framing are not written to rollouts.
            // Keep the pre-compaction baseline separate instead of charging
            // all opaque tokens to the encrypted summary.
            let overhead = protocol_overhead_hint.unwrap_or(0).min(extra);
            extra -= overhead;
            sized[index].tokens += extra;
            if overhead > 0 {
                sized.insert(
                    0,
                    Item {
                        category: Category::Unknown,
                        tokens: overhead,
                    },
                );
            }
        } else {
            sized.insert(
                0,
                Item {
                    category: Category::Unknown,
                    tokens: extra,
                },
            );
        }
    }
    let mut cached_left = cached;
    for item in sized {
        let item_cached = item.tokens.min(cached_left);
        cached_left -= item_cached;
        let total = totals.entry(item.category).or_default();
        total.0 += item.tokens;
        total.1 += item_cached;
    }
}

fn update_overhead_hint(context: &[Item], input: u64, hint: &mut Option<u64>) {
    if hint.is_some()
        || context
            .iter()
            .any(|item| item.category == Category::CompactionSummary)
    {
        return;
    }
    let raw: u64 = context.iter().map(|item| item.tokens).sum();
    *hint = Some(input.saturating_sub(raw));
}

fn proportional_sizes(context: &[Item], input: u64, raw: u64) -> Vec<Item> {
    let mut result: Vec<Item> = context
        .iter()
        .map(|item| Item {
            category: item.category,
            tokens: item.tokens.saturating_mul(input) / raw,
        })
        .collect();
    let assigned: u64 = result.iter().map(|item| item.tokens).sum();
    for item in result.iter_mut().take((input - assigned) as usize) {
        item.tokens += 1;
    }
    result
}

fn append_compact(target: &mut Vec<Item>, items: impl IntoIterator<Item = Item>) {
    for item in items {
        push_item(target, item.category, item.tokens);
    }
}

fn push_item(items: &mut Vec<Item>, category: Category, tokens: u64) {
    if tokens == 0 && category != Category::CompactionSummary {
        return;
    }
    if let Some(last) = items.last_mut().filter(|last| last.category == category) {
        last.tokens += tokens;
    } else {
        items.push(Item { category, tokens });
    }
}

fn content_texts(value: Option<&Value>) -> Vec<&str> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.get("text").and_then(Value::as_str))
        .collect()
}

fn output_texts(value: Option<&Value>) -> Vec<&str> {
    match value {
        Some(Value::String(text)) => vec![text],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect(),
        _ => Vec::new(),
    }
}

fn text_fields(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::String(text)) => estimate_tokens(text),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.get("text")
                    .and_then(Value::as_str)
                    .map(estimate_tokens)
                    .unwrap_or(0)
            })
            .sum(),
        _ => 0,
    }
}

fn looks_like_instructions(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("agents.md")
        || lower.contains("<environment_context>")
        || lower.contains("<permissions instructions>")
        || lower.contains("<skills_instructions>")
}

fn classify_tool_output(text: &str, call: Option<&(String, String)>) -> Category {
    if !looks_like_code(text) {
        return Category::ToolOutputOther;
    }
    let is_reader = call.is_some_and(|(name, input)| {
        let haystack = format!("{name} {input}").to_ascii_lowercase();
        [
            "read", "view", "cat ", "sed ", "rg ", "grep ", "git diff", "get_file",
        ]
        .iter()
        .any(|needle| haystack.contains(needle))
    });
    if is_reader {
        Category::RepositoryCode
    } else {
        Category::ToolOutputCode
    }
}

fn looks_like_code(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().take(400).collect();
    if lines.len() < 2 {
        return false;
    }
    let code_lines = lines
        .iter()
        .filter(|line| {
            let line = line.trim();
            line.starts_with("diff --git")
                || line.starts_with("@@ ")
                || line.starts_with("fn ")
                || line.starts_with("pub ")
                || line.starts_with("def ")
                || line.starts_with("class ")
                || line.starts_with("import ")
                || line.starts_with("use ")
                || line.starts_with("const ")
                || line.starts_with("let ")
                || line.starts_with("function ")
                || line.starts_with("{")
                || line.starts_with("}")
                || (line.contains('=') && (line.ends_with(';') || line.contains("=>")))
        })
        .count();
    code_lines >= 2 && code_lines * 5 >= lines.len()
}

fn estimate_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    if chars == 0 { 0 } else { chars.div_ceil(4) }
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.to_utc())
}

fn percent(value: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        value as f64 * 100.0 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_tokens_follow_the_context_prefix() {
        let context = vec![
            Item {
                category: Category::Instructions,
                tokens: 10,
            },
            Item {
                category: Category::UserPrompts,
                tokens: 10,
            },
            Item {
                category: Category::ToolCalls,
                tokens: 10,
            },
        ];
        let mut totals = BTreeMap::new();
        attribute(&context, 30, 15, None, &mut totals);
        assert_eq!(totals[&Category::Instructions], (10, 10));
        assert_eq!(totals[&Category::UserPrompts], (10, 5));
        assert_eq!(totals[&Category::ToolCalls], (10, 0));
    }

    #[test]
    fn opaque_compaction_absorbs_unobserved_context() {
        let context = vec![
            Item {
                category: Category::Instructions,
                tokens: 10,
            },
            Item {
                category: Category::CompactionSummary,
                tokens: 0,
            },
        ];
        let mut totals = BTreeMap::new();
        attribute(&context, 40, 35, Some(5), &mut totals);
        assert_eq!(totals[&Category::CompactionSummary], (25, 20));
        assert_eq!(totals[&Category::Unknown], (5, 5));
    }
}
