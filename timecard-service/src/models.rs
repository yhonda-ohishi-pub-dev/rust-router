//! Timecard models
//!
//! Domain models for timecard management.

use chrono::{NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

/// Timecard entry for a single day
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimecardEntry {
    pub id: Option<i64>,
    pub employee_id: String,
    pub date: NaiveDate,
    pub clock_in: Option<NaiveTime>,
    pub clock_out: Option<NaiveTime>,
    pub break_minutes: Option<i32>,
    pub notes: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl TimecardEntry {
    /// Create a new timecard entry
    pub fn new(employee_id: String, date: NaiveDate) -> Self {
        Self {
            id: None,
            employee_id,
            date,
            clock_in: None,
            clock_out: None,
            break_minutes: None,
            notes: None,
            created_at: None,
            updated_at: None,
        }
    }

    /// Calculate working hours for this entry
    pub fn working_hours(&self) -> Option<f64> {
        match (self.clock_in, self.clock_out) {
            (Some(clock_in), Some(clock_out)) => {
                let duration = clock_out.signed_duration_since(clock_in);
                let break_duration = chrono::Duration::minutes(self.break_minutes.unwrap_or(0) as i64);
                let working_duration = duration - break_duration;
                Some(working_duration.num_minutes() as f64 / 60.0)
            }
            _ => None,
        }
    }
}

/// Timecard representing a collection of entries for an employee
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timecard {
    pub employee_id: String,
    pub entries: Vec<TimecardEntry>,
}

impl Timecard {
    /// Create a new timecard for an employee
    pub fn new(employee_id: String) -> Self {
        Self {
            employee_id,
            entries: Vec::new(),
        }
    }

    /// Calculate total working hours
    pub fn total_working_hours(&self) -> f64 {
        self.entries
            .iter()
            .filter_map(|e| e.working_hours())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_working_hours_calculation() {
        let mut entry = TimecardEntry::new(
            "EMP001".to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        );
        entry.clock_in = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        entry.clock_out = Some(NaiveTime::from_hms_opt(18, 0, 0).unwrap());
        entry.break_minutes = Some(60);

        let hours = entry.working_hours().unwrap();
        assert!((hours - 8.0).abs() < 0.01);
    }

    #[test]
    fn test_working_hours_no_clock_out() {
        let mut entry = TimecardEntry::new(
            "EMP001".to_string(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
        );
        entry.clock_in = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());

        assert!(entry.working_hours().is_none());
    }
}
