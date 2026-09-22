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
    assert_eq!(rows[0]["total_tokens"], 155);
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
fn breakdown_requires_days_and_emits_json() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args([
            "breakdown",
            "--rollouts",
            "tests/fixtures/rollouts",
            "--since",
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
}

#[test]
fn breakdown_rejects_non_day_ranges() {
    let output = Command::new(env!("CARGO_BIN_EXE_codex-usage-analyzer"))
        .args(["breakdown", "--since", "12h"])
        .output()
        .expect("binary should run");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("expected a positive number of days"));
}
