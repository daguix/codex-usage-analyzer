# Codex Usage Analyzer

A small, database-free Rust CLI that reads Codex `rollout-*.jsonl` files and
reports token usage and estimated costs.

## Build

```bash
cargo build --release
```

The binary is written to `target/release/codex-usage-analyzer`.

## Usage

```bash
# Today's usage
codex-usage-analyzer --today

# Last seven days, broken down by model
codex-usage-analyzer --last 7d --by model

# JSON for all available rollouts
codex-usage-analyzer report --last total --format json

# Latest captured usage snapshot
codex-usage-analyzer status
```

The default rollout directory is `~/.codex/sessions`. Override it with
`--rollouts PATH` or `CODEX_USAGE_ROLLOUTS`.

Supported report options include:

- `--today`, `--last`, `--from`, and `--to`
- `--group day|week|month`
- `--by model|directory|session`
- `--format table|json|csv`
- `--timezone IANA_NAME`
- `--output PATH`

The analyzer reads only session metadata, turn context, and `token_count`
events. It does not store prompts, messages, or tool payloads.
