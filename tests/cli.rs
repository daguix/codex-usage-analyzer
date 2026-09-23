use std::process::Command;

#[test]
fn report_matches_fixture_totals() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "report",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--last",
            "total",
            "--by",
            "model",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows[0]["period"], "All");
    assert_eq!(rows[0]["total_tokens"], 155);
    assert_eq!(rows[1]["period"], "All");
    assert_eq!(rows[1]["total_tokens"], 310);
}

#[test]
fn report_can_aggregate_the_entire_range() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "report",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--last",
            "total",
            "--group",
            "all",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);
    assert_eq!(rows[0]["period"], "All");
    assert_eq!(rows[0]["total_tokens"], 465);
    assert!(rows[0].get("duration_samples").is_none());

    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "report",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--last",
            "total",
            "--group",
            "all",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        text.lines().filter(|line| line.starts_with("All ")).count(),
        1
    );
}

#[test]
fn latency_view_groups_by_model() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "latency",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--last",
            "total",
            "--by",
            "model",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows[0]["group"], "gpt-5.2-codex");
    assert_eq!(rows[0]["average_duration_ms"], 5000.0);
    assert_eq!(rows[0]["average_ttft_ms"], 1000.0);
    assert_eq!(rows[1]["group"], "gpt-5.6-luna");
    assert_eq!(rows[1]["average_duration_ms"], 15000.0);
    assert_eq!(rows[1]["average_ttft_ms"], 3000.0);
}

#[test]
fn latency_view_aggregates_the_entire_range() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "latency",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--last",
            "total",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows[0]["period"], "All");
    assert_eq!(rows[0]["duration_samples"], 2);
    assert_eq!(rows[0]["average_duration_ms"], 10000.0);
    assert_eq!(rows[0]["p50_duration_ms"], 5000);
    assert_eq!(rows[0]["p95_duration_ms"], 15000);
    assert_eq!(rows[0]["ttft_samples"], 2);
    assert_eq!(rows[0]["average_ttft_ms"], 2000.0);
    assert_eq!(rows[0]["p50_ttft_ms"], 1000);
    assert_eq!(rows[0]["p95_ttft_ms"], 3000);
}

#[test]
fn report_groups_usage_by_effort() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "report",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--last",
            "total",
            "--by",
            "effort",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows[0]["group"], "high");
    assert_eq!(rows[0]["total_tokens"], 155);
    assert_eq!(rows[1]["group"], "medium");
    assert_eq!(rows[1]["total_tokens"], 310);
}

#[test]
fn report_groups_usage_by_model_and_effort() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "report",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--last",
            "total",
            "--by",
            "model,effort",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let rows: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(rows[0]["group"], "gpt-5.2-codex / high");
    assert_eq!(rows[0]["total_tokens"], 155);
    assert_eq!(rows[1]["group"], "gpt-5.6-luna / medium");
    assert_eq!(rows[1]["total_tokens"], 310);
}

#[test]
fn csv_has_one_header() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "--rollouts",
            "tests/fixtures/rollouts",
            "--last",
            "total",
            "--format",
            "csv",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.matches("period,group,total_tokens").count(), 1);
}

#[test]
fn status_shows_reasoning_output_tokens() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "status",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--timezone",
            "UTC",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Token usage: total=310 input=240 cached=40 output=60 reasoning=10"));
    assert!(text.contains("Context window: 99% left (310 used / 22,000)"));
    assert!(text.contains("5h limit: 75% left (resets 2026-09-21T14:13:20+00:00)"));
    assert!(text.contains("7d limit: 85% left (resets 2026-09-21T14:13:20+00:00)"));
}

#[test]
fn breakdown_last_emits_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "breakdown",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--last",
            "99999d",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = document["rows"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row["estimated_input_tokens"].as_u64().unwrap())
            .sum::<u64>(),
        360
    );
    assert_eq!(document["cached_input_tokens"], 60);
    assert_eq!(document["output_tokens"], 90);
    assert_eq!(document["reasoning_output_tokens"], 15);
}

#[test]
fn breakdown_without_range_analyzes_everything() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "breakdown",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["calls"], 2);
    assert_eq!(document["input_tokens"], 360);
}

#[test]
fn breakdown_applies_start_and_end() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "breakdown",
            "--rollouts",
            "tests/fixtures/breakdown",
            "--from",
            "2026-09-22T10:00:02Z",
            "--to",
            "2026-09-22T10:00:05Z",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(document["calls"], 2);
}

#[test]
fn breakdown_emits_structured_paths_for_real_tool_activity() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "breakdown",
            "--rollouts",
            "tests/fixtures/breakdown",
            "--format",
            "json",
        ])
        .output()
        .expect("binary should run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = document["rows"].as_array().unwrap();
    assert_eq!(document["calls"], 3);
    assert_eq!(document["input_tokens"], 420);
    assert!(document["estimated_code_input_tokens"].as_u64().unwrap() > 0);
    assert!(rows.iter().any(|row| {
        row["category"]["family"] == "tool_calls" && row["category"]["kind"] == "patch_edit_payload"
    }));
    assert!(rows.iter().any(|row| {
        row["category"]["family"] == "tool_outputs"
            && row["category"]["kind"] == "repository_source"
            && row["category"]["source"] == "rg"
    }));
}

#[test]
fn breakdown_table_renders_family_kind_and_source_levels() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "breakdown",
            "--rollouts",
            "tests/fixtures/breakdown",
            "--format",
            "table",
        ])
        .output()
        .expect("binary should run");
    assert!(output.status.success());
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(table.lines().any(|line| line.starts_with("Tool outputs")));
    assert!(
        table
            .lines()
            .any(|line| line.starts_with("  Repository source"))
    );
    assert!(table.lines().any(|line| line.starts_with("    rg")));
}
