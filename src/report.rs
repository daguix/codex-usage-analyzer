use std::collections::BTreeMap;

use anyhow::Result;
use chrono::{Datelike, Weekday};
use chrono_tz::Tz;
use serde::Serialize;

use crate::ingest::{RateLimitWindow, UsageEvent};
use crate::pricing::Pricing;

#[derive(Clone, Copy, Debug)]
pub enum PeriodGroup {
    Total,
    Day,
    Week,
    Month,
}

#[derive(Clone, Copy, Debug)]
pub enum GroupBy {
    Model,
    Effort,
    Directory,
    Session,
}

#[derive(Clone, Copy, Debug)]
pub enum ReportFormat {
    Table,
    Json,
    Csv,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ReportRow {
    pub period: String,
    pub group: String,
    pub total_tokens: u64,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub input_cost: f64,
    pub cached_input_cost: f64,
    pub output_cost: f64,
    pub estimated_cost: f64,
}

pub fn aggregate(
    events: impl Iterator<Item = UsageEvent>,
    period: PeriodGroup,
    by: &[GroupBy],
    timezone: Tz,
    pricing: &Pricing,
) -> Vec<ReportRow> {
    let mut rows: BTreeMap<(String, String), ReportRow> = BTreeMap::new();
    for event in events {
        let local = event.captured_at.with_timezone(&timezone);
        let period_key = match period {
            PeriodGroup::Total => "Total".to_owned(),
            PeriodGroup::Day => local.format("%Y-%m-%d").to_string(),
            PeriodGroup::Week => {
                let offset = match local.weekday() {
                    Weekday::Mon => 0,
                    Weekday::Tue => 1,
                    Weekday::Wed => 2,
                    Weekday::Thu => 3,
                    Weekday::Fri => 4,
                    Weekday::Sat => 5,
                    Weekday::Sun => 6,
                };
                (local - chrono::Duration::days(offset))
                    .format("%Y-%m-%d")
                    .to_string()
            }
            PeriodGroup::Month => local.format("%Y-%m").to_string(),
        };
        let group_key = if by.is_empty() {
            "all".to_owned()
        } else {
            by.iter()
                .map(|group| {
                    match group {
                        GroupBy::Model => event.model.as_deref(),
                        GroupBy::Effort => event.effort.as_deref(),
                        GroupBy::Directory => event.directory.as_deref(),
                        GroupBy::Session => event.session_id.as_deref(),
                    }
                    .unwrap_or("<unknown>")
                })
                .collect::<Vec<_>>()
                .join(" / ")
        };
        let row = rows
            .entry((period_key.clone(), group_key.clone()))
            .or_insert_with(|| ReportRow {
                period: period_key,
                group: group_key,
                ..ReportRow::default()
            });
        row.total_tokens += event.total_tokens;
        row.input_tokens += event.input_tokens;
        row.cached_input_tokens += event.cached_input_tokens;
        row.output_tokens += event.output_tokens;
        row.reasoning_output_tokens += event.reasoning_output_tokens;
        if let Some(rates) = event
            .model
            .as_deref()
            .and_then(|model| pricing.rates_for(model))
        {
            let non_cached = event.input_tokens.saturating_sub(event.cached_input_tokens);
            row.input_cost += non_cached as f64 * rates.input / 1_000_000.0;
            row.cached_input_cost += event.cached_input_tokens as f64 * rates.cached / 1_000_000.0;
            row.output_cost += event.output_tokens as f64 * rates.output / 1_000_000.0;
            row.estimated_cost = row.input_cost + row.cached_input_cost + row.output_cost;
        }
    }
    rows.into_values().collect()
}

pub fn render(rows: &[ReportRow], include_group: bool, format: ReportFormat) -> Result<String> {
    match format {
        ReportFormat::Table => Ok(render_table(rows, include_group)),
        ReportFormat::Json => Ok(serde_json::to_string_pretty(rows)?),
        ReportFormat::Csv => render_csv(rows),
    }
}

fn render_csv(rows: &[ReportRow]) -> Result<String> {
    let mut writer = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(Vec::new());
    writer.write_record([
        "period",
        "group",
        "total_tokens",
        "input_tokens",
        "cached_input_tokens",
        "output_tokens",
        "reasoning_output_tokens",
        "input_cost",
        "cached_input_cost",
        "output_cost",
        "estimated_cost",
    ])?;
    for row in rows {
        writer.serialize(row)?;
    }
    let bytes = writer.into_inner()?;
    Ok(String::from_utf8(bytes)?.trim_end_matches('\n').to_owned())
}

fn render_table(rows: &[ReportRow], include_group: bool) -> String {
    let show_summary = rows.len() != 1 || rows[0].period != "Total" || include_group;
    let mut headers = vec!["Period".to_owned()];
    if include_group {
        headers.push("Group".to_owned());
    }
    headers.extend(
        [
            "Total",
            "Input",
            "Cached",
            "Output",
            "Reasoning",
            "Input cost",
            "Cached cost",
            "Output cost",
            "Est. cost",
        ]
        .map(str::to_owned),
    );

    let values: Vec<Vec<String>> = rows
        .iter()
        .map(|row| row_values(row, include_group))
        .collect();
    let total_values = row_values(&summarize(rows), include_group);
    let mut widths: Vec<usize> = headers.iter().map(String::len).collect();
    for row in values.iter().chain(std::iter::once(&total_values)) {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(value.len());
        }
    }
    let format_row = |row: &[String]| {
        row.iter()
            .enumerate()
            .map(|(index, value)| format!("{value:<width$}", width = widths[index]))
            .collect::<Vec<_>>()
            .join("  ")
    };
    let separator: Vec<String> = widths.iter().map(|width| "-".repeat(*width)).collect();
    let mut lines = vec![format_row(&headers), format_row(&separator)];
    lines.extend(values.iter().map(|row| format_row(row)));
    if !rows.is_empty() && show_summary {
        lines.push(format_row(&separator));
    }
    if show_summary {
        lines.push(format_row(&total_values));
    }
    lines.join("\n")
}

fn summarize(rows: &[ReportRow]) -> ReportRow {
    let mut total = ReportRow {
        period: "Total".to_owned(),
        ..ReportRow::default()
    };
    for row in rows {
        total.total_tokens += row.total_tokens;
        total.input_tokens += row.input_tokens;
        total.cached_input_tokens += row.cached_input_tokens;
        total.output_tokens += row.output_tokens;
        total.reasoning_output_tokens += row.reasoning_output_tokens;
        total.input_cost += row.input_cost;
        total.cached_input_cost += row.cached_input_cost;
        total.output_cost += row.output_cost;
        total.estimated_cost += row.estimated_cost;
    }
    total
}

fn row_values(row: &ReportRow, include_group: bool) -> Vec<String> {
    let mut values = vec![row.period.clone()];
    if include_group {
        values.push(row.group.clone());
    }
    values.extend([
        format_integer(row.total_tokens),
        format_integer(row.input_tokens),
        format_integer(row.cached_input_tokens),
        format_integer(row.output_tokens),
        format_integer(row.reasoning_output_tokens),
        format!("${:.2}", row.input_cost),
        format!("${:.2}", row.cached_input_cost),
        format!("${:.2}", row.output_cost),
        format!("${:.2}", row.estimated_cost),
    ]);
    values
}

fn format_integer(value: u64) -> String {
    let source = value.to_string();
    let mut result = String::with_capacity(source.len() + source.len() / 3);
    for (index, character) in source.chars().enumerate() {
        if index > 0 && (source.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(character);
    }
    result
}

pub fn render_status(event: &UsageEvent, timezone: Tz) {
    println!(
        "Captured: {}",
        event
            .captured_at
            .with_timezone(&timezone)
            .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
    );
    if let Some(model) = &event.model {
        println!("Model: {model}");
    }
    if let Some(directory) = &event.directory {
        println!("Directory: {directory}");
    }
    if let Some(session) = &event.session_id {
        println!("Session: {session}");
    }
    if let Some(version) = &event.codex_version {
        println!("Codex version: {version}");
    }
    println!(
        "Token usage: total={} input={} cached={} output={} reasoning={}",
        format_integer(event.total_tokens),
        format_integer(event.input_tokens),
        format_integer(event.cached_input_tokens),
        format_integer(event.output_tokens),
        format_integer(event.reasoning_output_tokens),
    );
    if let Some(total) = event.context_window.filter(|total| *total > 0) {
        let used = event.total_tokens;
        let used_percent = used
            .saturating_mul(100)
            .checked_div(total)
            .unwrap_or_default();
        let left = 100_u64.saturating_sub(used_percent);
        println!(
            "Context window: {left}% left ({} used / {})",
            format_integer(used),
            format_integer(total)
        );
    }
    if let Some(limit) = &event.primary_limit {
        render_rate_limit(limit, "Primary", timezone);
    }
    if let Some(limit) = &event.secondary_limit {
        render_rate_limit(limit, "Secondary", timezone);
    }
}

fn render_rate_limit(limit: &RateLimitWindow, fallback: &str, timezone: Tz) {
    let label = limit
        .window_minutes
        .map(format_window_duration)
        .unwrap_or_else(|| fallback.to_owned());
    if let Some(resets_at) = limit.resets_at {
        println!(
            "{label} limit: {}% left (resets {})",
            limit.percent_left,
            resets_at
                .with_timezone(&timezone)
                .format("%Y-%m-%dT%H:%M:%S%:z")
        );
    } else {
        println!("{label} limit: {}% left", limit.percent_left);
    }
}

fn format_window_duration(minutes: u64) -> String {
    if minutes > 0 && minutes.is_multiple_of(1_440) {
        format!("{}d", minutes / 1_440)
    } else if minutes > 0 && minutes.is_multiple_of(60) {
        format!("{}h", minutes / 60)
    } else {
        format!("{minutes}min")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_format_matches_python_style() {
        assert_eq!(format_integer(1_234_567), "1,234,567");
    }

    #[test]
    fn rate_limit_windows_use_their_actual_duration() {
        assert_eq!(format_window_duration(300), "5h");
        assert_eq!(format_window_duration(10_080), "7d");
        assert_eq!(format_window_duration(90), "90min");
    }
}
