# Codex Usage Analyzer

A small, database-free Rust CLI that reads Codex `rollout-*.jsonl` files and
reports token usage and estimated costs.
The separate latency view reports end-to-end turn duration and time to first
token (TTFT).

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

# Last seven days, broken down by model and reasoning effort
codex-usage-analyzer --last 7d --by model,effort

# JSON for all available rollouts
codex-usage-analyzer report --last total --format json

# Latest captured usage snapshot
codex-usage-analyzer status

# Latency statistics for the last seven days, broken down by model
codex-usage-analyzer latency --last 7d --by model

# Estimated composition of input and cached-input context over seven days
codex-usage-analyzer breakdown --last 7d

# All available rollouts
codex-usage-analyzer breakdown
```

The default rollout directory is `~/.codex/sessions`. Override it with
`--rollouts PATH` or `CODEX_USAGE_ROLLOUTS`.

Supported report options include:

- `--today`, `--last`, `--from`, and `--to`
- `--group all|day|week|month` (default: `all`)
- `--by model|effort|directory|session`, with comma-separated dimensions such as `--by model,effort`
- `--format table|json|csv`
- `--timezone IANA_NAME`
- `--output PATH`

`latency` accepts the same range, grouping, format, timezone, and output options
as `report`. It shows sample counts, averages, medians, and p95 values. Latency
fields are emitted in milliseconds in JSON and CSV; the table uses
human-readable durations. Older rollouts may not contain latency measurements,
so missing values are excluded from the sample counts and aggregates.

`breakdown` reads context items but does not store them. It allocates the exact
reported input, cached-input, output, and reasoning-output totals across
categories using the recorded context order. Tokenization, encrypted
compaction summaries, model-injected tool schemas, and protocol overhead make
the category split an estimate.
`reasoning_output_tokens` is shown separately; depending on the rollout schema,
it may be a subset of `output_tokens` rather than an additional token count.
Without a range option, every available rollout is analyzed. `breakdown` accepts
the same `--last`, `--today`, `--from`, and `--to` options as reports.
Use `--format table|json|csv` and `--output PATH` as with reports. Code-looking
output from file-reading/search commands is classified as repository source.
The table groups results hierarchically by family, content kind, and source;
for example, `Tool outputs` → `Repository source` → `rg`. JSON exposes the same
three-part path as a structured `category` object, while CSV keeps separate
`family`, `kind`, and `source` columns.
Detected-code counts are metrics on every hierarchy level. They scan every
observable text category, including fenced Markdown, diff hunks, compiler
excerpts, prompts, assistant messages, and otherwise mixed tool output. Opaque
protocol overhead and the unobserved portion of encrypted compaction summaries
cannot be classified.
Tool-output kinds include repository source, build/test/lint, search/listings,
version control, patches/edits, web/external data, UI/media, process control,
data/analysis, system/environment, diagnostics, generic shell, and
uncategorized output. Sources such as `rg`, `grep`, `find`, `ls`, `sed`, and
`cat` remain individually attributable below those kinds.
For patch/edit calls, the call wrapper and metadata are accounted separately
from the actual patch or replacement-code payload; the tool's confirmation is
reported as a third, distinct result category.
