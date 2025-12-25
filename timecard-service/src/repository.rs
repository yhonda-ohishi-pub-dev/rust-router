//! Timecard repository
//!
//! Database operations for timecard management.

use anyhow::Result;
use chrono::NaiveDate;
use thiserror::Error;

use crate::models::TimecardEntry;

/// Repository errors
#[derive(Error, Debug)]
pub enum RepositoryError {
    #[error("Entry not found: {0}")]
    NotFound(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),
}

/// Timecard repository trait for database operations
#[allow(async_fn_in_trait)]
pub trait TimecardRepository: Send + Sync {
    /// Find timecard entry by employee ID and date
    async fn find_by_employee_and_date(
        &self,
        employee_id: &str,
        date: NaiveDate,
    ) -> Result<Option<TimecardEntry>>;

    /// Find all entries for an employee in a date range
    async fn find_by_employee_and_range(
        &self,
        employee_id: &str,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<TimecardEntry>>;

    /// Create a new timecard entry
    async fn create(&self, entry: &TimecardEntry) -> Result<TimecardEntry>;

    /// Update an existing timecard entry
    async fn update(&self, entry: &TimecardEntry) -> Result<TimecardEntry>;

    /// Delete a timecard entry
    async fn delete(&self, id: i64) -> Result<()>;
}

/// In-memory repository for testing and development
pub struct InMemoryRepository {
    entries: std::sync::RwLock<Vec<TimecardEntry>>,
    next_id: std::sync::atomic::AtomicI64,
}

impl InMemoryRepository {
    pub fn new() -> Self {
        Self {
            entries: std::sync::RwLock::new(Vec::new()),
            next_id: std::sync::atomic::AtomicI64::new(1),
        }
    }
}

impl Default for InMemoryRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl TimecardRepository for InMemoryRepository {
    async fn find_by_employee_and_date(
        &self,
        employee_id: &str,
        date: NaiveDate,
    ) -> Result<Option<TimecardEntry>> {
        let entries = self.entries.read().unwrap();
        Ok(entries
            .iter()
            .find(|e| e.employee_id == employee_id && e.date == date)
            .cloned())
    }

    async fn find_by_employee_and_range(
        &self,
        employee_id: &str,
        start_date: NaiveDate,
        end_date: NaiveDate,
    ) -> Result<Vec<TimecardEntry>> {
        let entries = self.entries.read().unwrap();
        Ok(entries
            .iter()
            .filter(|e| {
                e.employee_id == employee_id && e.date >= start_date && e.date <= end_date
            })
            .cloned()
            .collect())
    }

    async fn create(&self, entry: &TimecardEntry) -> Result<TimecardEntry> {
        let mut entries = self.entries.write().unwrap();
        let mut new_entry = entry.clone();
        new_entry.id = Some(
            self.next_id
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        );
        new_entry.created_at = Some(chrono::Utc::now());
        new_entry.updated_at = Some(chrono::Utc::now());
        entries.push(new_entry.clone());
        Ok(new_entry)
    }

    async fn update(&self, entry: &TimecardEntry) -> Result<TimecardEntry> {
        let mut entries = self.entries.write().unwrap();
        if let Some(id) = entry.id {
            if let Some(existing) = entries.iter_mut().find(|e| e.id == Some(id)) {
                existing.clock_in = entry.clock_in;
                existing.clock_out = entry.clock_out;
                existing.break_minutes = entry.break_minutes;
                existing.notes = entry.notes.clone();
                existing.updated_at = Some(chrono::Utc::now());
                return Ok(existing.clone());
            }
        }
        Err(RepositoryError::NotFound(format!("Entry with id {:?}", entry.id)).into())
    }

    async fn delete(&self, id: i64) -> Result<()> {
        let mut entries = self.entries.write().unwrap();
        let len_before = entries.len();
        entries.retain(|e| e.id != Some(id));
        if entries.len() == len_before {
            return Err(RepositoryError::NotFound(format!("Entry with id {}", id)).into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    #[tokio::test]
    async fn test_create_and_find() {
        let repo = InMemoryRepository::new();
        let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let mut entry = TimecardEntry::new("EMP001".to_string(), date);
        entry.clock_in = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());

        let created = repo.create(&entry).await.unwrap();
        assert!(created.id.is_some());

        let found = repo.find_by_employee_and_date("EMP001", date).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().employee_id, "EMP001");
    }

    #[tokio::test]
    async fn test_update() {
        let repo = InMemoryRepository::new();
        let date = NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();
        let entry = TimecardEntry::new("EMP001".to_string(), date);

        let mut created = repo.create(&entry).await.unwrap();
        created.clock_in = Some(NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        created.clock_out = Some(NaiveTime::from_hms_opt(18, 0, 0).unwrap());

        let updated = repo.update(&created).await.unwrap();
        assert!(updated.clock_in.is_some());
        assert!(updated.clock_out.is_some());
    }
}
