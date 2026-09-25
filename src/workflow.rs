use std::collections::BTreeMap;

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Serialize;

use crate::ingest::LatencyEvent;
use crate::report::{GroupBy, PeriodGroup, ReportFormat};

#[derive(Debug, Serialize)]
pub struct WorkflowRow {
    pub period: String,
    pub group: String,
    pub agent_hours: f64,
    pub wall_clock_active_hours: f64,
    pub effective_parallelism: f64,
}

pub fn aggregate(
    events: impl IntoIterator<Item = LatencyEvent>,
    range_start: Option<DateTime<Utc>>,
    range_end: Option<DateTime<Utc>>,
    timezone: Tz,
    period: PeriodGroup,
    by: &[GroupBy],
) -> Vec<WorkflowRow> {
    let mut groups = BTreeMap::<(String, String), Vec<(DateTime<Utc>, DateTime<Utc>)>>::new();
    for event in events {
        let Some(duration_ms) = event
            .duration_ms
            .and_then(|value| i64::try_from(value).ok())
        else {
            continue;
        };
        let Some(start) = event
            .captured_at
            .checked_sub_signed(Duration::milliseconds(duration_ms))
        else {
            continue;
        };
        let mut cursor = range_start.map_or(start, |limit| start.max(limit));
        let end = range_end.map_or(event.captured_at, |limit| event.captured_at.min(limit));
        let group = if by.is_empty() {
            "all".to_owned()
        } else {
            by.iter()
                .map(|dimension| {
                    match dimension {
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
        while cursor < end {
            let local_day = cursor.with_timezone(&timezone).date_naive();
            let boundary = next_day_start(local_day, timezone).unwrap_or(end);
            let segment_end = end.min(boundary);
            if segment_end <= cursor {
                break;
            }
            let period_key = match period {
                PeriodGroup::All => "All".to_owned(),
                PeriodGroup::Day => local_day.to_string(),
                PeriodGroup::Week => (local_day
                    - Duration::days(i64::from(local_day.weekday().num_days_from_monday())))
                .to_string(),
                PeriodGroup::Month => local_day.format("%Y-%m").to_string(),
            };
            groups
                .entry((period_key, group.clone()))
                .or_default()
                .push((cursor, segment_end));
            cursor = segment_end;
        }
    }
    groups
        .into_iter()
        .map(|((period, group), mut intervals)| {
            let agent_ms: i64 = intervals
                .iter()
                .map(|(start, end)| (*end - *start).num_milliseconds())
                .sum();
            intervals.sort_unstable();
            let mut wall_ms = 0_i64;
            let mut active_end = None;
            for (start, end) in intervals {
                let uncovered_start = active_end.map_or(start, |previous| start.max(previous));
                if end > uncovered_start {
                    wall_ms += (end - uncovered_start).num_milliseconds();
                }
                active_end = Some(active_end.map_or(end, |previous| end.max(previous)));
            }
            WorkflowRow {
                period,
                group,
                agent_hours: agent_ms as f64 / 3_600_000.0,
                wall_clock_active_hours: wall_ms as f64 / 3_600_000.0,
                effective_parallelism: agent_ms as f64 / wall_ms as f64,
            }
        })
        .collect()
}

fn next_day_start(day: NaiveDate, timezone: Tz) -> Option<DateTime<Utc>> {
    let mut date = day.succ_opt()?;
    loop {
        for minute in 0..1_440 {
            let local = date.and_hms_opt(minute / 60, minute % 60, 0)?;
            if let Some(value) = timezone.from_local_datetime(&local).earliest() {
                return Some(value.to_utc());
            }
        }
        date = date.succ_opt()?;
    }
}

pub fn render(rows: &[WorkflowRow], format: ReportFormat) -> Result<String> {
    match format {
        ReportFormat::Json => Ok(serde_json::to_string_pretty(rows)?),
        ReportFormat::Csv => {
            let mut writer = csv::WriterBuilder::new()
                .has_headers(false)
                .from_writer(Vec::new());
            writer.write_record([
                "period",
                "group",
                "agent_hours",
                "wall_clock_active_hours",
                "effective_parallelism",
            ])?;
            for row in rows {
                writer.serialize(row)?;
            }
            Ok(String::from_utf8(writer.into_inner()?)?
                .trim_end_matches('\n')
                .to_owned())
        }
        ReportFormat::Table => {
            let headers = [
                "Period",
                "Group",
                "Agent-hours",
                "Active wall-hours",
                "Effective parallelism",
            ];
            let values: Vec<[String; 5]> = rows
                .iter()
                .map(|row| {
                    [
                        row.period.clone(),
                        row.group.clone(),
                        format!("{:.3}", row.agent_hours),
                        format!("{:.3}", row.wall_clock_active_hours),
                        format!("{:.2}x", row.effective_parallelism),
                    ]
                })
                .collect();
            let mut widths = headers.map(str::len);
            for row in &values {
                for (index, value) in row.iter().enumerate() {
                    widths[index] = widths[index].max(value.chars().count());
                }
            }
            let format_row = |row: &[&str; 5]| {
                row.iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if index < 2 {
                            format!("{value:<width$}", width = widths[index])
                        } else {
                            format!("{value:>width$}", width = widths[index])
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            let mut lines = vec![format_row(&headers)];
            let separators = widths.map(|width| "-".repeat(width));
            lines.push(separators.join("  "));
            for row in &values {
                lines.push(format_row(&row.each_ref().map(String::as_str)));
            }
            Ok(lines.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::*;

    fn event(end: &str, duration_ms: u64) -> LatencyEvent {
        LatencyEvent {
            captured_at: DateTime::parse_from_rfc3339(end).unwrap().to_utc(),
            duration_ms: Some(duration_ms),
            ..LatencyEvent::default()
        }
    }

    #[test]
    fn overlapping_turns_count_once_in_active_wall_time() {
        let rows = aggregate(
            [
                event("2026-09-22T02:00:00Z", 7_200_000),
                event("2026-09-22T03:00:00Z", 7_200_000),
            ],
            None,
            None,
            chrono_tz::UTC,
            PeriodGroup::All,
            &[],
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].agent_hours, 4.0);
        assert_eq!(rows[0].wall_clock_active_hours, 3.0);
        assert!((rows[0].effective_parallelism - 4.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn splits_at_local_midnight_and_clips_to_range() {
        let start = DateTime::parse_from_rfc3339("2026-09-22T21:30:00Z")
            .unwrap()
            .to_utc();
        let end = DateTime::parse_from_rfc3339("2026-09-22T22:30:00Z")
            .unwrap()
            .to_utc();
        let rows = aggregate(
            [event("2026-09-22T23:00:00Z", 7_200_000)],
            Some(start),
            Some(end),
            chrono_tz::Europe::Paris,
            PeriodGroup::Day,
            &[],
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].period, "2026-09-22");
        assert_eq!(rows[0].agent_hours, 0.5);
        assert_eq!(rows[1].period, "2026-09-23");
        assert_eq!(rows[1].agent_hours, 0.5);
    }

    #[test]
    fn daylight_saving_day_uses_elapsed_hours() {
        let rows = aggregate(
            [event("2026-03-29T22:00:00Z", 82_800_000)],
            None,
            None,
            chrono_tz::Europe::Paris,
            PeriodGroup::Day,
            &[],
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].period, "2026-03-29");
        assert_eq!(rows[0].agent_hours, 23.0);
        assert_eq!(rows[0].wall_clock_active_hours, 23.0);
    }

    #[test]
    fn table_keeps_numeric_columns_aligned_with_long_groups() {
        let rows = vec![
            WorkflowRow {
                period: "All".to_owned(),
                group: "/home/user/projects/a-long-project-name".to_owned(),
                agent_hours: 2.0,
                wall_clock_active_hours: 1.0,
                effective_parallelism: 2.0,
            },
            WorkflowRow {
                period: "All".to_owned(),
                group: "short".to_owned(),
                agent_hours: 3.0,
                wall_clock_active_hours: 2.0,
                effective_parallelism: 1.5,
            },
        ];
        let table = render(&rows, ReportFormat::Table).unwrap();
        let lines: Vec<_> = table.lines().collect();
        assert_eq!(lines[0].len(), lines[1].len());
        assert_eq!(lines[0].len(), lines[2].len());
        assert_eq!(lines[0].len(), lines[3].len());
        assert_eq!(lines[2].find("2.000"), lines[3].find("3.000"));
        assert_eq!(lines[2].find("2.00x"), lines[3].find("1.50x"));
    }
}
