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
    ToolOutputBuildTest,
    ToolOutputSearchListing,
    ToolOutputVersionControl,
    ToolOutputPatchEdit,
    ToolOutputWeb,
    ToolOutputUiMedia,
    ToolOutputProcessControl,
    ToolOutputDataAnalysis,
    ToolOutputSystemInfo,
    ToolOutputDiagnostics,
    ToolOutputShell,
    ToolOutputOther,
    UserPrompts,
    AssistantText,
    AssistantReasoning,
    ToolCalls,
    CompactionSummary,
    Unknown,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::Instructions => "instructions / AGENTS.md",
            Self::RepositoryCode => "source / repository code",
            Self::ToolOutputCode => "tool outputs: other code",
            Self::ToolOutputBuildTest => "tool outputs: build / test / lint",
            Self::ToolOutputSearchListing => "tool outputs: search / listings",
            Self::ToolOutputVersionControl => "tool outputs: version control",
            Self::ToolOutputPatchEdit => "tool outputs: patches / edits",
            Self::ToolOutputWeb => "tool outputs: web / external data",
            Self::ToolOutputUiMedia => "tool outputs: UI / media",
            Self::ToolOutputProcessControl => "tool outputs: process control",
            Self::ToolOutputDataAnalysis => "tool outputs: data / analysis",
            Self::ToolOutputSystemInfo => "tool outputs: system / environment",
            Self::ToolOutputDiagnostics => "tool outputs: errors / diagnostics",
            Self::ToolOutputShell => "tool outputs: other shell",
            Self::ToolOutputOther => "tool outputs: other",
            Self::UserPrompts => "user prompts",
            Self::AssistantText => "assistant text",
            Self::AssistantReasoning => "assistant reasoning",
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
    pub estimated_output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub input_percent: f64,
    pub cached_percent: f64,
    pub output_percent: f64,
    pub reasoning_percent: f64,
}

#[derive(Debug, Default, Serialize)]
pub struct Breakdown {
    pub rows: Vec<BreakdownRow>,
    pub files: usize,
    pub calls: usize,
    pub invalid_lines: usize,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
}

#[derive(Clone, Copy, Debug)]
struct Item {
    category: Category,
    tokens: u64,
}

#[derive(Debug, Default)]
struct FileBreakdown {
    totals: BTreeMap<Category, CategoryTotals>,
    calls: usize,
    invalid_lines: usize,
    input_tokens: u64,
    cached_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct CategoryTotals {
    input: u64,
    cached_input: u64,
    output: u64,
    reasoning_output: u64,
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
    let mut totals = BTreeMap::<Category, CategoryTotals>::new();
    for file in parsed {
        let file = file?;
        result.calls += file.calls;
        result.invalid_lines += file.invalid_lines;
        result.input_tokens += file.input_tokens;
        result.cached_input_tokens += file.cached_input_tokens;
        result.output_tokens += file.output_tokens;
        result.reasoning_output_tokens += file.reasoning_output_tokens;
        for (category, value) in file.totals {
            let total = totals.entry(category).or_default();
            total.input += value.input;
            total.cached_input += value.cached_input;
            total.output += value.output;
            total.reasoning_output += value.reasoning_output;
        }
    }
    result.rows = Category::all()
        .into_iter()
        .map(|category| {
            let total = totals.get(&category).copied().unwrap_or_default();
            BreakdownRow {
                category,
                estimated_input_tokens: total.input,
                estimated_cached_input_tokens: total.cached_input,
                estimated_output_tokens: total.output,
                reasoning_output_tokens: total.reasoning_output,
                input_percent: percent(total.input, result.input_tokens),
                cached_percent: percent(total.cached_input, result.cached_input_tokens),
                output_percent: percent(total.output, result.output_tokens),
                reasoning_percent: percent(total.reasoning_output, result.reasoning_output_tokens),
            }
        })
        .filter(|row| {
            row.estimated_input_tokens > 0
                || row.estimated_cached_input_tokens > 0
                || row.estimated_output_tokens > 0
                || row.reasoning_output_tokens > 0
        })
        .collect();
    result
        .rows
        .sort_by_key(|row| std::cmp::Reverse(row.estimated_cached_input_tokens));
    Ok(result)
}

impl Category {
    fn all() -> [Self; 21] {
        [
            Self::Instructions,
            Self::RepositoryCode,
            Self::ToolOutputCode,
            Self::ToolOutputBuildTest,
            Self::ToolOutputSearchListing,
            Self::ToolOutputVersionControl,
            Self::ToolOutputPatchEdit,
            Self::ToolOutputWeb,
            Self::ToolOutputUiMedia,
            Self::ToolOutputProcessControl,
            Self::ToolOutputDataAnalysis,
            Self::ToolOutputSystemInfo,
            Self::ToolOutputDiagnostics,
            Self::ToolOutputShell,
            Self::ToolOutputOther,
            Self::UserPrompts,
            Self::AssistantText,
            Self::AssistantReasoning,
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
                    let output = number(usage, "output_tokens");
                    let reasoning = number(usage, "reasoning_output_tokens");
                    update_overhead_hint(&context, input, &mut protocol_overhead_hint);
                    attribute(
                        &context,
                        input,
                        cached,
                        protocol_overhead_hint,
                        &mut result.totals,
                    );
                    attribute_output(
                        &pending,
                        output,
                        reasoning,
                        reasoning_is_included(usage),
                        &mut result.totals,
                    );
                    result.calls += 1;
                    result.input_tokens += input;
                    result.cached_input_tokens += cached;
                    result.output_tokens += output;
                    result.reasoning_output_tokens += reasoning;
                }
                materialize_reasoning(&mut pending, number(usage, "reasoning_output_tokens"));
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
                    let output = number(usage, "output_tokens");
                    let reasoning = number(usage, "reasoning_output_tokens");
                    update_overhead_hint(&context, input, &mut protocol_overhead_hint);
                    attribute(
                        &context,
                        input,
                        cached,
                        protocol_overhead_hint,
                        &mut result.totals,
                    );
                    attribute_output(
                        &pending,
                        output,
                        reasoning,
                        reasoning_is_included(usage),
                        &mut result.totals,
                    );
                    result.calls += 1;
                    result.input_tokens += input;
                    result.cached_input_tokens += cached;
                    result.output_tokens += output;
                    result.reasoning_output_tokens += reasoning;
                }
                materialize_reasoning(&mut pending, number(usage, "reasoning_output_tokens"));
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
            push_item(pending, Category::AssistantReasoning, tokens);
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
    totals: &mut BTreeMap<Category, CategoryTotals>,
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
        total.input += item.tokens;
        total.cached_input += item_cached;
    }
}

fn attribute_output(
    pending: &[Item],
    output: u64,
    reasoning: u64,
    reasoning_included: bool,
    totals: &mut BTreeMap<Category, CategoryTotals>,
) {
    totals
        .entry(Category::AssistantReasoning)
        .or_default()
        .reasoning_output += reasoning;
    let reasoning_in_output = if reasoning_included {
        reasoning.min(output)
    } else {
        0
    };
    totals
        .entry(Category::AssistantReasoning)
        .or_default()
        .output += reasoning_in_output;
    let remaining = output - reasoning_in_output;
    if remaining == 0 {
        return;
    }
    let visible: Vec<Item> = pending
        .iter()
        .copied()
        .filter(|item| item.category != Category::AssistantReasoning && item.tokens > 0)
        .collect();
    let raw: u64 = visible.iter().map(|item| item.tokens).sum();
    if raw == 0 {
        totals.entry(Category::Unknown).or_default().output += remaining;
        return;
    }
    for item in proportional_sizes(&visible, remaining, raw) {
        totals.entry(item.category).or_default().output += item.tokens;
    }
}

fn materialize_reasoning(pending: &mut Vec<Item>, reasoning: u64) {
    if reasoning == 0 {
        return;
    }
    if let Some(item) = pending
        .iter_mut()
        .rfind(|item| item.category == Category::AssistantReasoning)
    {
        item.tokens = item.tokens.max(reasoning);
    } else {
        push_item(pending, Category::AssistantReasoning, reasoning);
    }
}

fn reasoning_is_included(usage: &Value) -> bool {
    let input = number(usage, "input_tokens");
    let output = number(usage, "output_tokens");
    let total = number(usage, "total_tokens");
    total == 0 || total == input.saturating_add(output)
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
    if tokens == 0
        && category != Category::CompactionSummary
        && category != Category::AssistantReasoning
    {
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
    let invocation = call
        .map(|(name, input)| format!("{name} {input}").to_ascii_lowercase())
        .unwrap_or_default();
    let is_reader = contains_any(
        &invocation,
        &[
            "read_file",
            "get_file",
            "open_file",
            "list_files",
            "cmd:cat ",
            "cmd:\"cat ",
            "cmd:sed ",
            "cmd:\"sed ",
            "cmd:rg ",
            "cmd:\"rg ",
            "cmd:grep ",
            "cmd:\"grep ",
            "cmd:head ",
            "cmd:\"head ",
            "cmd:tail ",
            "cmd:\"tail ",
            " cat ",
            " sed ",
            " rg ",
            " grep ",
            " head ",
            " tail ",
        ],
    );
    if is_reader && looks_like_code(text) {
        Category::RepositoryCode
    } else if contains_any(
        &invocation,
        &["apply_patch", "write_file", "edit_file", "replace_in_file"],
    ) || text.contains("diff --git ")
    {
        Category::ToolOutputPatchEdit
    } else if is_build_or_test(&invocation) {
        Category::ToolOutputBuildTest
    } else if is_version_control(&invocation) {
        Category::ToolOutputVersionControl
    } else if contains_any(
        &invocation,
        &[
            "web__run",
            "search_query",
            "image_query",
            "cmd:curl ",
            "cmd:\"curl ",
            " curl ",
            "cmd:wget ",
            "cmd:\"wget ",
            " wget ",
            "createbrowsertab",
            "gettab(",
            "cua.",
        ],
    ) {
        Category::ToolOutputWeb
    } else if contains_any(
        &invocation,
        &["view_image", "imagegen", "emitimage", "generatedimage"],
    ) {
        Category::ToolOutputUiMedia
    } else if is_reader
        || contains_any(
            &invocation,
            &[
                "cmd:find ",
                "cmd:\"find ",
                " find ",
                "cmd:ls ",
                "cmd:\"ls ",
                " ls ",
                "cmd:pwd",
                "cmd:\"pwd",
                "list_mcp_resources",
            ],
        )
    {
        Category::ToolOutputSearchListing
    } else if contains_any(
        &invocation,
        &["write_stdin", "tools.wait", "wait_agent", "yield_control"],
    ) {
        Category::ToolOutputProcessControl
    } else if contains_any(
        &invocation,
        &[
            "cmd:jq ",
            "cmd:\"jq ",
            " jq ",
            "cmd:python ",
            "cmd:\"python ",
            " python -c ",
            "cmd:perl ",
            "cmd:\"perl ",
            " perl ",
            "dataframe",
            "query_database",
        ],
    ) {
        Category::ToolOutputDataAnalysis
    } else if contains_any(
        &invocation,
        &[
            "cmd:env",
            "cmd:\"env",
            "cmd:which ",
            "cmd:\"which ",
            "cmd:du ",
            "cmd:\"du ",
            "cmd:df ",
            "cmd:\"df ",
            "cmd:ps ",
            "cmd:\"ps ",
            "cmd:stat ",
            "cmd:\"stat ",
            "nvidia-smi",
            "rustc --version",
            "node --version",
        ],
    ) {
        Category::ToolOutputSystemInfo
    } else if looks_like_diagnostic(text) {
        Category::ToolOutputDiagnostics
    } else if invocation.contains("exec_command") {
        Category::ToolOutputShell
    } else if looks_like_code(text) {
        Category::ToolOutputCode
    } else {
        Category::ToolOutputOther
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn is_build_or_test(invocation: &str) -> bool {
    contains_any(
        invocation,
        &[
            "cargo test",
            "cargo build",
            "cargo check",
            "cargo clippy",
            "cargo fmt",
            "npm test",
            "npm run",
            "pnpm test",
            "pnpm run",
            "yarn test",
            "yarn run",
            "pytest",
            "python -m pytest",
            "go test",
            "go build",
            "dotnet test",
            "dotnet build",
            "mvn test",
            "gradle test",
            "./gradlew",
            "make test",
            "cmake --build",
            "swift test",
            "rustc ",
        ],
    )
}

fn is_version_control(invocation: &str) -> bool {
    contains_any(
        invocation,
        &[
            "cmd:git ",
            "cmd:\"git ",
            " git status",
            " git diff",
            " git show",
            " git log",
            " git add",
            " git commit",
        ],
    )
}

fn looks_like_diagnostic(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    contains_any(
        &lower,
        &[
            "\"iserror\":true",
            "fatal:",
            "error:",
            "traceback (most recent call last)",
            "permission denied",
            "command not found",
            "no such file or directory",
        ],
    ) || nonzero_exit_code(&lower)
}

fn nonzero_exit_code(text: &str) -> bool {
    let Some((_, suffix)) = text.split_once("\"exit_code\":") else {
        return false;
    };
    let value = suffix
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    !value.is_empty() && value != "0"
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
        assert_eq!(totals[&Category::Instructions].input, 10);
        assert_eq!(totals[&Category::Instructions].cached_input, 10);
        assert_eq!(totals[&Category::UserPrompts].input, 10);
        assert_eq!(totals[&Category::UserPrompts].cached_input, 5);
        assert_eq!(totals[&Category::ToolCalls].input, 10);
        assert_eq!(totals[&Category::ToolCalls].cached_input, 0);
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
        assert_eq!(totals[&Category::CompactionSummary].input, 25);
        assert_eq!(totals[&Category::CompactionSummary].cached_input, 20);
        assert_eq!(totals[&Category::Unknown].input, 5);
        assert_eq!(totals[&Category::Unknown].cached_input, 5);
    }

    #[test]
    fn tool_outputs_are_split_by_invocation() {
        let build = (
            "exec".to_owned(),
            "tools.exec_command({cmd:\"cargo test\"})".to_owned(),
        );
        let search = (
            "exec".to_owned(),
            "tools.exec_command({cmd:\"rg -n foo src\"})".to_owned(),
        );
        let git = (
            "exec".to_owned(),
            "tools.exec_command({cmd:\"git status\"})".to_owned(),
        );
        assert_eq!(
            classify_tool_output("test result: ok", Some(&build)),
            Category::ToolOutputBuildTest
        );
        assert_eq!(
            classify_tool_output("src/main.rs:12:foo", Some(&search)),
            Category::ToolOutputSearchListing
        );
        assert_eq!(
            classify_tool_output("On branch main", Some(&git)),
            Category::ToolOutputVersionControl
        );
    }

    #[test]
    fn output_and_reasoning_are_attributed_separately() {
        let pending = vec![
            Item {
                category: Category::AssistantText,
                tokens: 10,
            },
            Item {
                category: Category::ToolCalls,
                tokens: 30,
            },
            Item {
                category: Category::AssistantReasoning,
                tokens: 0,
            },
        ];
        let mut totals = BTreeMap::new();
        attribute_output(&pending, 100, 20, true, &mut totals);
        assert_eq!(totals[&Category::AssistantReasoning].output, 20);
        assert_eq!(totals[&Category::AssistantReasoning].reasoning_output, 20);
        assert_eq!(totals[&Category::AssistantText].output, 20);
        assert_eq!(totals[&Category::ToolCalls].output, 60);
    }
}
