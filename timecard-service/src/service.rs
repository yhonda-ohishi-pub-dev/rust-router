//! Timecard service
//!
//! Business logic for timecard management.
//! Implements tower::Service for InProcess calls.

use anyhow::Result;
use chrono::{NaiveDate, NaiveTime};
use thiserror::Error;

use crate::models::TimecardEntry;
use crate::repository::{InMemoryRepository, TimecardRepository};

/// Service errors
#[derive(Error, Debug)]
pub enum ServiceError {
    #[error("Timecard not found for employee {employee_id} on {date}")]
    NotFound { employee_id: String, date: String },

    #[error("Invalid time format: {0}")]
    InvalidTimeFormat(String),

    #[error("Clock out time must be after clock in time")]
    InvalidTimeRange,

    #[error("Repository error: {0}")]
    RepositoryError(String),
}

/// Timecard service for business operations
pub struct TimecardService {
    repository: InMemoryRepository,
}

impl TimecardService {
    /// Create a new timecard service with in-memory repository
    pub fn new() -> Self {
        Self {
            repository: InMemoryRepository::new(),
        }
    }

    /// Get timecard entry for an employee on a specific date
    pub async fn get_entry(
        &self,
        employee_id: &str,
        date: &str,
    ) -> Result<TimecardEntry, ServiceError> {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| ServiceError::InvalidTimeFormat(date.to_string()))?;

        self.repository
            .find_by_employee_and_date(employee_id, date)
            .await
            .map_err(|e| ServiceError::RepositoryError(e.to_string()))?
            .ok_or_else(|| ServiceError::NotFound {
                employee_id: employee_id.to_string(),
                date: date.to_string(),
            })
    }

    /// Clock in for an employee
    pub async fn clock_in(
        &self,
        employee_id: &str,
        date: &str,
        time: &str,
    ) -> Result<TimecardEntry, ServiceError> {
        let parsed_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| ServiceError::InvalidTimeFormat(date.to_string()))?;

        let parsed_time = NaiveTime::parse_from_str(time, "%H:%M")
            .map_err(|_| ServiceError::InvalidTimeFormat(time.to_string()))?;

        // Check if entry exists
        let existing = self
            .repository
            .find_by_employee_and_date(employee_id, parsed_date)
            .await
            .map_err(|e| ServiceError::RepositoryError(e.to_string()))?;

        match existing {
            Some(mut entry) => {
                entry.clock_in = Some(parsed_time);
                self.repository
                    .update(&entry)
                    .await
                    .map_err(|e| ServiceError::RepositoryError(e.to_string()))
            }
            None => {
                let mut entry = TimecardEntry::new(employee_id.to_string(), parsed_date);
                entry.clock_in = Some(parsed_time);
                self.repository
                    .create(&entry)
                    .await
                    .map_err(|e| ServiceError::RepositoryError(e.to_string()))
            }
        }
    }

    /// Clock out for an employee
    pub async fn clock_out(
        &self,
        employee_id: &str,
        date: &str,
        time: &str,
    ) -> Result<TimecardEntry, ServiceError> {
        let parsed_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| ServiceError::InvalidTimeFormat(date.to_string()))?;

        let parsed_time = NaiveTime::parse_from_str(time, "%H:%M")
            .map_err(|_| ServiceError::InvalidTimeFormat(time.to_string()))?;

        // Get existing entry
        let mut entry = self
            .repository
            .find_by_employee_and_date(employee_id, parsed_date)
            .await
            .map_err(|e| ServiceError::RepositoryError(e.to_string()))?
            .ok_or_else(|| ServiceError::NotFound {
                employee_id: employee_id.to_string(),
                date: date.to_string(),
            })?;

        // Validate time range
        if let Some(clock_in) = entry.clock_in {
            if parsed_time <= clock_in {
                return Err(ServiceError::InvalidTimeRange);
            }
        }

        entry.clock_out = Some(parsed_time);
        self.repository
            .update(&entry)
            .await
            .map_err(|e| ServiceError::RepositoryError(e.to_string()))
    }

    /// Create a complete timecard entry
    pub async fn create_entry(
        &self,
        employee_id: &str,
        date: &str,
        clock_in: &str,
        clock_out: &str,
    ) -> Result<TimecardEntry, ServiceError> {
        let parsed_date = NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| ServiceError::InvalidTimeFormat(date.to_string()))?;

        let parsed_clock_in = NaiveTime::parse_from_str(clock_in, "%H:%M")
            .map_err(|_| ServiceError::InvalidTimeFormat(clock_in.to_string()))?;

        let parsed_clock_out = NaiveTime::parse_from_str(clock_out, "%H:%M")
            .map_err(|_| ServiceError::InvalidTimeFormat(clock_out.to_string()))?;

        // Validate time range
        if parsed_clock_out <= parsed_clock_in {
            return Err(ServiceError::InvalidTimeRange);
        }

        let mut entry = TimecardEntry::new(employee_id.to_string(), parsed_date);
        entry.clock_in = Some(parsed_clock_in);
        entry.clock_out = Some(parsed_clock_out);

        self.repository
            .create(&entry)
            .await
            .map_err(|e| ServiceError::RepositoryError(e.to_string()))
    }

    /// Get entries for an employee in a date range
    pub async fn get_entries_in_range(
        &self,
        employee_id: &str,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<TimecardEntry>, ServiceError> {
        let parsed_start = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
            .map_err(|_| ServiceError::InvalidTimeFormat(start_date.to_string()))?;

        let parsed_end = NaiveDate::parse_from_str(end_date, "%Y-%m-%d")
            .map_err(|_| ServiceError::InvalidTimeFormat(end_date.to_string()))?;

        self.repository
            .find_by_employee_and_range(employee_id, parsed_start, parsed_end)
            .await
            .map_err(|e| ServiceError::RepositoryError(e.to_string()))
    }
}

impl Default for TimecardService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_clock_in_and_out() {
        let service = TimecardService::new();

        // Clock in
        let entry = service.clock_in("EMP001", "2024-01-15", "09:00").await.unwrap();
        assert_eq!(entry.employee_id, "EMP001");
        assert!(entry.clock_in.is_some());
        assert!(entry.clock_out.is_none());

        // Clock out
        let entry = service.clock_out("EMP001", "2024-01-15", "18:00").await.unwrap();
        assert!(entry.clock_in.is_some());
        assert!(entry.clock_out.is_some());
    }

    #[tokio::test]
    async fn test_create_entry() {
        let service = TimecardService::new();

        let entry = service
            .create_entry("EMP001", "2024-01-15", "09:00", "18:00")
            .await
            .unwrap();

        assert_eq!(entry.employee_id, "EMP001");
        let hours = entry.working_hours().unwrap();
        assert!((hours - 9.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_invalid_time_range() {
        let service = TimecardService::new();

        let result = service
            .create_entry("EMP001", "2024-01-15", "18:00", "09:00")
            .await;

        assert!(matches!(result, Err(ServiceError::InvalidTimeRange)));
    }
}
