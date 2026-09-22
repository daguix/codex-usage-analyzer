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
