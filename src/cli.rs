use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Datelike, LocalResult, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::breakdown;
use crate::ingest::{ScanResult, scan_rollouts};
use crate::pricing::Pricing;
use crate::report::{self, GroupBy, PeriodGroup, ReportFormat};

const DEFAULT_TIMEZONE: &str = "Europe/Stockholm";
type DateRange = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

#[derive(Debug, Parser)]
#[command(
    name = "codex-usage-analyzer",
    version,
    about = "Analyze Codex rollout token usage without a database",
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    report: ReportArgs,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Aggregate token usage and estimated costs.
    Report(ReportArgs),
    /// Show the latest token usage snapshot.
    Status(StatusArgs),
    /// Estimate which kinds of content make up model input and cached input.
    Breakdown(BreakdownArgs),
}

#[derive(Clone, Debug, Args)]
struct SourceArgs {
    /// Directory containing rollout-*.jsonl files.
    #[arg(long, env = "CODEX_USAGE_ROLLOUTS")]
    rollouts: Option<PathBuf>,
    /// IANA timezone used for ranges and grouping.
    #[arg(long, default_value = DEFAULT_TIMEZONE)]
    timezone: String,
}

#[derive(Clone, Debug, Args)]
struct RangeArgs {
    /// Relative range such as 7d, 12h, 1m, 30min, or total.
    #[arg(long, conflicts_with_all = ["today", "from", "to"])]
    last: Option<String>,
    /// Analyze local midnight through now.
    #[arg(long, conflicts_with_all = ["last", "from", "to"])]
    today: bool,
    /// Range start (YYYY-MM-DD or ISO-8601).
    #[arg(long)]
    from: Option<String>,
    /// Range end (YYYY-MM-DD or ISO-8601).
    #[arg(long)]
    to: Option<String>,
}

#[derive(Clone, Debug, Args)]
struct ReportArgs {
    #[command(flatten)]
    source: SourceArgs,
    #[command(flatten)]
    range: RangeArgs,
    /// Period used to aggregate rows.
    #[arg(long, value_enum, default_value_t = PeriodArg::Day)]
    group: PeriodArg,
    /// Optional secondary grouping.
    #[arg(long, value_enum)]
    by: Option<GroupArg>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    format: FormatArg,
    /// Write output to a file instead of stdout.
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
}

#[derive(Clone, Debug, Args)]
struct StatusArgs {
    #[command(flatten)]
    source: SourceArgs,
}

#[derive(Clone, Debug, Args)]
struct BreakdownArgs {
    #[command(flatten)]
    source: SourceArgs,
    /// Analyze only model calls from the last N days (for example: 7d).
    #[arg(long)]
    since: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Table)]
    format: FormatArg,
    /// Write output to a file instead of stdout.
    #[arg(long, short = 'o')]
    output: Option<PathBuf>,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum PeriodArg {
    Day,
    Week,
    Month,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum GroupArg {
    Model,
    Directory,
    Session,
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum FormatArg {
    Table,
    Json,
    Csv,
}

pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Command::Report(args)) => run_report(args),
        Some(Command::Status(args)) => run_status(args),
        Some(Command::Breakdown(args)) => run_breakdown(args),
        None => run_report(cli.report),
    }
}

fn run_breakdown(args: BreakdownArgs) -> Result<()> {
    // Validate the timezone for consistency with the other commands, although
    // this relative range is an elapsed duration and therefore UTC-based.
    parse_timezone(&args.source.timezone)?;
    let since = args
        .since
        .as_deref()
        .map(parse_since_days)
        .transpose()?
        .map(|days| Utc::now() - chrono::Duration::days(days))
        .unwrap_or(DateTime::<Utc>::MIN_UTC);
    let root = args
        .source
        .rollouts
        .clone()
        .unwrap_or_else(default_rollouts_dir);
    let result = breakdown::analyze(&root, since)
        .with_context(|| format!("failed to scan {}", root.display()))?;
    if result.invalid_lines > 0 {
        eprintln!(
            "warning: ignored {} malformed JSONL line(s) across {} rollout file(s)",
            result.invalid_lines, result.files
        );
    }
    let output = match args.format {
        FormatArg::Table => render_breakdown_table(&result),
        FormatArg::Json => serde_json::to_string_pretty(&result)?,
        FormatArg::Csv => render_breakdown_csv(&result.rows)?,
    };
    if let Some(path) = args.output {
        std::fs::write(&path, format!("{output}\n"))
            .with_context(|| format!("failed to write {}", path.display()))?;
    } else {
        println!("{output}");
    }
    Ok(())
}

fn parse_since_days(value: &str) -> Result<i64> {
    let normalized = value.trim().to_ascii_lowercase();
    let Some(number) = normalized.strip_suffix('d') else {
        bail!("invalid --since value; expected a positive number of days such as 7d");
    };
    let days: i64 = number.parse().context("invalid --since day count")?;
    if days <= 0 {
        bail!("invalid --since value; day count must be positive");
    }
    Ok(days)
}

fn render_breakdown_table(result: &breakdown::Breakdown) -> String {
    let mut lines = vec![format!(
        "Estimated context composition ({} model calls; input {}, cached {})",
        result.calls,
        format_count(result.input_tokens),
        format_count(result.cached_input_tokens)
    )];
    lines.push(format!(
        "{:<31} {:>14} {:>8} {:>14} {:>8}",
        "Category", "Input", "Input %", "Cached", "Cache %"
    ));
    lines.push(format!(
        "{:-<31} {:-<14} {:-<8} {:-<14} {:-<8}",
        "", "", "", "", ""
    ));
    for row in &result.rows {
        lines.push(format!(
            "{:<31} {:>14} {:>7.1}% {:>14} {:>7.1}%",
            row.category.label(),
            format_count(row.estimated_input_tokens),
            row.input_percent,
            format_count(row.estimated_cached_input_tokens),
            row.cached_percent,
        ));
    }
    lines.push("\nEstimate: reported token totals allocated from recorded context order; encrypted summaries and protocol/tool-schema overhead are inferred.".to_owned());
    lines.join("\n")
}

fn render_breakdown_csv(rows: &[breakdown::BreakdownRow]) -> Result<String> {
    let mut writer = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());
    writer.write_record([
        "category",
        "estimated_input_tokens",
        "estimated_cached_input_tokens",
        "input_percent",
        "cached_percent",
    ])?;
    for row in rows {
        writer.serialize(row)?;
    }
    Ok(String::from_utf8(writer.into_inner()?)?
        .trim_end()
        .to_owned())
}

fn format_count(value: u64) -> String {
    let source = value.to_string();
    source
        .chars()
        .enumerate()
        .flat_map(|(index, character)| {
            let comma = (index > 0 && (source.len() - index).is_multiple_of(3)).then_some(',');
            comma.into_iter().chain(std::iter::once(character))
        })
        .collect()
}

fn run_report(args: ReportArgs) -> Result<()> {
    let timezone = parse_timezone(&args.source.timezone)?;
    let (start, end) = resolve_range(&args.range, timezone)?;
    let scan = scan(&args.source)?;
    emit_scan_warnings(&scan);
    let events = scan.events.into_iter().filter(|event| {
        start.is_none_or(|start| event.captured_at >= start)
            && end.is_none_or(|end| event.captured_at <= end)
    });
    let rows = report::aggregate(
        events,
        match args.group {
            PeriodArg::Day => PeriodGroup::Day,
            PeriodArg::Week => PeriodGroup::Week,
            PeriodArg::Month => PeriodGroup::Month,
        },
        args.by.map(|value| match value {
            GroupArg::Model => GroupBy::Model,
            GroupArg::Directory => GroupBy::Directory,
            GroupArg::Session => GroupBy::Session,
        }),
        timezone,
        &Pricing::default(),
    );
    let output = report::render(
        &rows,
        args.by.is_some(),
        match args.format {
            FormatArg::Table => ReportFormat::Table,
            FormatArg::Json => ReportFormat::Json,
            FormatArg::Csv => ReportFormat::Csv,
        },
    )?;
    if let Some(path) = args.output {
        std::fs::write(&path, format!("{output}\n"))
            .with_context(|| format!("failed to write {}", path.display()))?;
    } else {
        println!("{output}");
    }
    Ok(())
}

fn run_status(args: StatusArgs) -> Result<()> {
    let timezone = parse_timezone(&args.source.timezone)?;
    let scan = scan(&args.source)?;
    emit_scan_warnings(&scan);
    let Some(event) = scan.events.iter().max_by_key(|event| event.captured_at) else {
        println!("No usage data captured yet.");
        return Ok(());
    };
    report::render_status(event, timezone);
    Ok(())
}

fn scan(args: &SourceArgs) -> Result<ScanResult> {
    let root = args.rollouts.clone().unwrap_or_else(default_rollouts_dir);
    scan_rollouts(&root).with_context(|| format!("failed to scan {}", root.display()))
}

fn emit_scan_warnings(scan: &ScanResult) {
    if scan.invalid_lines > 0 {
        eprintln!(
            "warning: ignored {} malformed JSONL line(s) across {} rollout file(s)",
            scan.invalid_lines, scan.files
        );
    }
}

fn default_rollouts_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".codex/sessions")
}

fn parse_timezone(value: &str) -> Result<Tz> {
    value
        .parse::<Tz>()
        .with_context(|| format!("invalid IANA timezone '{value}'"))
}

fn resolve_range(args: &RangeArgs, timezone: Tz) -> Result<DateRange> {
    let now = Utc::now();
    if args.today {
        let local_now = now.with_timezone(&timezone);
        let midnight = timezone
            .with_ymd_and_hms(
                local_now.year(),
                local_now.month(),
                local_now.day(),
                0,
                0,
                0,
            )
            .single()
            .context("local midnight is ambiguous in the selected timezone")?;
        return Ok((Some(midnight.to_utc()), Some(now)));
    }
    if let Some(value) = &args.last {
        return parse_last(value, now, timezone);
    }
    Ok((
        args.from
            .as_deref()
            .map(|value| parse_datetime(value, timezone))
            .transpose()?,
        args.to
            .as_deref()
            .map(|value| parse_datetime(value, timezone))
            .transpose()?,
    ))
}

fn parse_last(value: &str, now: DateTime<Utc>, timezone: Tz) -> Result<DateRange> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized == "total" {
        return Ok((None, None));
    }
    let digit_count = normalized.bytes().take_while(u8::is_ascii_digit).count();
    if digit_count == 0 || digit_count == normalized.len() {
        bail!("invalid --last value; expected total, 7d, 12h, 1m, or 30min");
    }
    let amount: i64 = normalized[..digit_count].parse()?;
    let start = match &normalized[digit_count..] {
        "min" | "mins" | "minute" | "minutes" => now - chrono::Duration::minutes(amount),
        "h" | "hour" | "hours" => now - chrono::Duration::hours(amount),
        "d" | "day" | "days" => now - chrono::Duration::days(amount),
        "m" | "mo" | "mon" | "month" | "months" => subtract_months(now, timezone, amount)?,
        _ => bail!("invalid --last unit; expected d, h, m (month), or min"),
    };
    Ok((Some(start), Some(now)))
}

fn subtract_months(now: DateTime<Utc>, timezone: Tz, months: i64) -> Result<DateTime<Utc>> {
    let local = now.with_timezone(&timezone);
    let absolute = i64::from(local.year()) * 12 + i64::from(local.month0()) - months;
    let year = i32::try_from(absolute.div_euclid(12))?;
    let month = u32::try_from(absolute.rem_euclid(12) + 1)?;
    let max_day = last_day_of_month(year, month)?;
    let date = NaiveDate::from_ymd_opt(year, month, local.day().min(max_day))
        .context("month range is outside the supported calendar")?;
    local_datetime(timezone, date.and_time(local.time()))
}

fn last_day_of_month(year: i32, month: u32) -> Result<u32> {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .context("month range is outside the supported calendar")?;
    Ok((first_next - chrono::Duration::days(1)).day())
}

fn parse_datetime(value: &str, timezone: Tz) -> Result<DateTime<Utc>> {
    let value = value.trim();
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return Ok(parsed.to_utc());
    }
    if let Ok(parsed) = NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f") {
        return local_datetime(timezone, parsed);
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return local_datetime(timezone, date.and_hms_opt(0, 0, 0).context("invalid date")?);
    }
    bail!("invalid date '{value}'; expected YYYY-MM-DD or ISO-8601")
}

fn local_datetime(timezone: Tz, value: NaiveDateTime) -> Result<DateTime<Utc>> {
    match timezone.from_local_datetime(&value) {
        LocalResult::Single(value) => Ok(value.to_utc()),
        LocalResult::Ambiguous(first, _) => Ok(first.to_utc()),
        LocalResult::None => bail!("local time {value} does not exist in {timezone}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minute_and_month_ranges_are_distinct() {
        let tz: Tz = DEFAULT_TIMEZONE.parse().unwrap();
        let now = DateTime::parse_from_rfc3339("2026-09-22T12:00:00Z")
            .unwrap()
            .to_utc();
        let minute = parse_last("30min", now, tz).unwrap().0.unwrap();
        let month = parse_last("1m", now, tz).unwrap().0.unwrap();
        assert_eq!((now - minute).num_minutes(), 30);
        assert!(month < now - chrono::Duration::days(28));
    }
}
