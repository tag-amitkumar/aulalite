// crates/backend/src/services/recurrence.rs
//! Pure-function occurrence expansion. No IO, no clock.

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, TimeZone, Timelike, Utc, Weekday};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frequency {
    None,
    Daily,
    Weekly,
    Biweekly,
    Monthly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndKind {
    Count(u32),
    Until(DateTime<Utc>),
    Open,
}

#[derive(Debug, Clone)]
pub struct SeriesSpec {
    pub starts_at: DateTime<Utc>,
    pub duration_minutes: u32,
    pub frequency: Frequency,
    pub byweekday: Vec<Weekday>,
    pub end_kind: EndKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOccurrence {
    pub occurrence_index: u32,
    pub starts_at: DateTime<Utc>,
    pub duration_minutes: u32,
}

pub const OPEN_SERIES_CAP: usize = 52;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExpandError {
    #[error("byweekday is required for weekly/biweekly")]
    MissingByweekday,
    #[error("byweekday is forbidden outside weekly/biweekly")]
    ExtraByweekday,
    #[error("count must be > 0")]
    InvalidCount,
    #[error("until must be > starts_at")]
    InvalidUntil,
}

pub fn expand(spec: &SeriesSpec) -> Result<Vec<NewOccurrence>, ExpandError> {
    // Validate first
    match spec.frequency {
        Frequency::Weekly | Frequency::Biweekly if spec.byweekday.is_empty() => {
            return Err(ExpandError::MissingByweekday);
        }
        Frequency::None | Frequency::Daily | Frequency::Monthly if !spec.byweekday.is_empty() => {
            return Err(ExpandError::ExtraByweekday);
        }
        _ => {}
    }
    match &spec.end_kind {
        EndKind::Count(0) => return Err(ExpandError::InvalidCount),
        EndKind::Until(t) if *t <= spec.starts_at => return Err(ExpandError::InvalidUntil),
        _ => {}
    }

    let cap = match &spec.end_kind {
        EndKind::Count(n) => *n as usize,
        EndKind::Until(_) => usize::MAX,
        EndKind::Open => OPEN_SERIES_CAP,
    };
    let until = match &spec.end_kind {
        EndKind::Until(t) => Some(*t),
        _ => None,
    };

    let starts: Vec<DateTime<Utc>> = match spec.frequency {
        Frequency::None => vec![spec.starts_at],
        Frequency::Daily => daily_iter(spec.starts_at).take(cap.min(400)).collect(),
        Frequency::Weekly => weekly_iter(spec.starts_at, &spec.byweekday, 1, cap),
        Frequency::Biweekly => weekly_iter(spec.starts_at, &spec.byweekday, 2, cap),
        Frequency::Monthly => monthly_iter(spec.starts_at).take(cap.min(400)).collect(),
    };

    let mut out = Vec::new();
    for (idx, ts) in starts.into_iter().enumerate() {
        if let Some(u) = until {
            if ts > u {
                break;
            }
        }
        out.push(NewOccurrence {
            occurrence_index: idx as u32,
            starts_at: ts,
            duration_minutes: spec.duration_minutes,
        });
        if out.len() >= cap {
            break;
        }
    }
    Ok(out)
}

fn daily_iter(start: DateTime<Utc>) -> impl Iterator<Item = DateTime<Utc>> {
    (0u32..).map(move |d| start.checked_add_days(Days::new(d as u64)).unwrap())
}

fn weekly_iter(
    start: DateTime<Utc>,
    byweekday: &[Weekday],
    interval_weeks: u32,
    cap: usize,
) -> Vec<DateTime<Utc>> {
    let mut sorted = byweekday.to_vec();
    sorted.sort_by_key(|w| w.num_days_from_monday());

    let week0 = monday_of(start.date_naive());
    let h = start.hour();
    let m = start.minute();
    let s = start.second();

    let mut out = Vec::<DateTime<Utc>>::new();
    let mut week = 0u32;
    while out.len() < cap.saturating_add(1) && week < 1000 {
        if week.is_multiple_of(interval_weeks) {
            for wd in &sorted {
                let day = week0 + Days::new((week as u64) * 7 + wd.num_days_from_monday() as u64);
                let ts = Utc
                    .with_ymd_and_hms(day.year(), day.month(), day.day(), h, m, s)
                    .single()
                    .unwrap();
                if ts >= start {
                    out.push(ts);
                    if out.len() >= cap {
                        return out;
                    }
                }
            }
        }
        week += 1;
    }
    out
}

fn monday_of(d: NaiveDate) -> NaiveDate {
    let dow = d.weekday().num_days_from_monday();
    d - Days::new(dow as u64)
}

fn monthly_iter(start: DateTime<Utc>) -> impl Iterator<Item = DateTime<Utc>> {
    (0u32..).map(move |m| {
        // Months::new clamps day-of-month to the target month's last day if needed
        // (e.g., Jan 31 + 1 month = Feb 28/29). This matches our spec.
        start.checked_add_months(Months::new(m)).unwrap_or(start)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday::*;

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn none_yields_one_occurrence() {
        let spec = SeriesSpec {
            starts_at: dt(2026, 5, 12, 17, 0),
            duration_minutes: 60,
            frequency: Frequency::None,
            byweekday: vec![],
            end_kind: EndKind::Count(1),
        };
        let out = expand(&spec).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].occurrence_index, 0);
        assert_eq!(out[0].starts_at, dt(2026, 5, 12, 17, 0));
    }

    #[test]
    fn daily_count_3() {
        let spec = SeriesSpec {
            starts_at: dt(2026, 5, 12, 17, 0),
            duration_minutes: 30,
            frequency: Frequency::Daily,
            byweekday: vec![],
            end_kind: EndKind::Count(3),
        };
        let out = expand(&spec).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].starts_at, dt(2026, 5, 12, 17, 0));
        assert_eq!(out[1].starts_at, dt(2026, 5, 13, 17, 0));
        assert_eq!(out[2].starts_at, dt(2026, 5, 14, 17, 0));
    }

    #[test]
    fn weekly_mwf_count_6() {
        // Tuesday May 12, 2026 — pattern Mon/Wed/Fri. First eligible slot is Wed May 13.
        let spec = SeriesSpec {
            starts_at: dt(2026, 5, 12, 17, 0),
            duration_minutes: 60,
            frequency: Frequency::Weekly,
            byweekday: vec![Mon, Wed, Fri],
            end_kind: EndKind::Count(6),
        };
        let out = expand(&spec).unwrap();
        assert_eq!(out.len(), 6);
        assert_eq!(out[0].starts_at, dt(2026, 5, 13, 17, 0)); // Wed
        assert_eq!(out[1].starts_at, dt(2026, 5, 15, 17, 0)); // Fri
        assert_eq!(out[2].starts_at, dt(2026, 5, 18, 17, 0)); // Mon
        assert_eq!(out[3].starts_at, dt(2026, 5, 20, 17, 0)); // Wed
        assert_eq!(out[4].starts_at, dt(2026, 5, 22, 17, 0)); // Fri
        assert_eq!(out[5].starts_at, dt(2026, 5, 25, 17, 0)); // Mon
    }

    #[test]
    fn biweekly_tue_count_3() {
        let spec = SeriesSpec {
            starts_at: dt(2026, 5, 12, 17, 0), // Tue
            duration_minutes: 60,
            frequency: Frequency::Biweekly,
            byweekday: vec![Tue],
            end_kind: EndKind::Count(3),
        };
        let out = expand(&spec).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].starts_at, dt(2026, 5, 12, 17, 0));
        assert_eq!(out[1].starts_at, dt(2026, 5, 26, 17, 0)); // +14 days
        assert_eq!(out[2].starts_at, dt(2026, 6, 9, 17, 0));
    }

    #[test]
    fn monthly_count_3() {
        let spec = SeriesSpec {
            starts_at: dt(2026, 1, 31, 9, 0),
            duration_minutes: 45,
            frequency: Frequency::Monthly,
            byweekday: vec![],
            end_kind: EndKind::Count(3),
        };
        let out = expand(&spec).unwrap();
        assert_eq!(out.len(), 3);
        // Feb 28 fallback (2026 is not a leap year)
        assert_eq!(out[1].starts_at, dt(2026, 2, 28, 9, 0));
        assert_eq!(out[2].starts_at, dt(2026, 3, 31, 9, 0));
    }

    #[test]
    fn until_truncates() {
        let spec = SeriesSpec {
            starts_at: dt(2026, 5, 12, 17, 0),
            duration_minutes: 30,
            frequency: Frequency::Daily,
            byweekday: vec![],
            end_kind: EndKind::Until(dt(2026, 5, 14, 23, 59)),
        };
        let out = expand(&spec).unwrap();
        assert_eq!(out.len(), 3); // 12, 13, 14
    }

    #[test]
    fn open_caps_at_52() {
        let spec = SeriesSpec {
            starts_at: dt(2026, 5, 12, 17, 0),
            duration_minutes: 30,
            frequency: Frequency::Weekly,
            byweekday: vec![Tue],
            end_kind: EndKind::Open,
        };
        let out = expand(&spec).unwrap();
        assert_eq!(out.len(), OPEN_SERIES_CAP);
    }

    #[test]
    fn weekly_without_byweekday_errors() {
        let spec = SeriesSpec {
            starts_at: dt(2026, 5, 12, 17, 0),
            duration_minutes: 30,
            frequency: Frequency::Weekly,
            byweekday: vec![],
            end_kind: EndKind::Count(1),
        };
        assert_eq!(expand(&spec), Err(ExpandError::MissingByweekday));
    }

    #[test]
    fn daily_with_byweekday_errors() {
        let spec = SeriesSpec {
            starts_at: dt(2026, 5, 12, 17, 0),
            duration_minutes: 30,
            frequency: Frequency::Daily,
            byweekday: vec![Mon],
            end_kind: EndKind::Count(1),
        };
        assert_eq!(expand(&spec), Err(ExpandError::ExtraByweekday));
    }
}
