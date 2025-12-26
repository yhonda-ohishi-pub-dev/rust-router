use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

/// Job status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobStatus {
    /// Job is queued and waiting to be processed
    Queued,
    /// Job is currently running
    Running,
    /// Job completed successfully
    Completed,
    /// Job failed with an error
    Failed,
}

impl Default for JobStatus {
    fn default() -> Self {
        Self::Queued
    }
}

/// Result for a single account in a job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountResult {
    /// Account user ID
    pub user_id: String,
    /// Account display name
    pub name: String,
    /// Current status
    pub status: JobStatus,
    /// Path to downloaded CSV file (if successful)
    pub csv_path: Option<PathBuf>,
    /// Error message (if failed)
    pub error_message: Option<String>,
}

impl AccountResult {
    /// Create a new queued account result
    pub fn new(user_id: String, name: String) -> Self {
        Self {
            user_id,
            name,
            status: JobStatus::Queued,
            csv_path: None,
            error_message: None,
        }
    }

    /// Mark as running
    pub fn set_running(&mut self) {
        self.status = JobStatus::Running;
    }

    /// Mark as completed with CSV path
    pub fn set_completed(&mut self, csv_path: PathBuf) {
        self.status = JobStatus::Completed;
        self.csv_path = Some(csv_path);
    }

    /// Mark as failed with error message
    pub fn set_failed(&mut self, error: String) {
        self.status = JobStatus::Failed;
        self.error_message = Some(error);
    }
}

/// Job state for a multi-account scrape job
#[derive(Debug, Clone)]
pub struct JobState {
    /// Unique job ID
    pub job_id: String,
    /// Overall job status
    pub status: JobStatus,
    /// Results for each account (keyed by user_id)
    pub accounts: HashMap<String, AccountResult>,
    /// Order of accounts (for sequential processing)
    pub account_order: Vec<String>,
    /// Passwords for each account (keyed by user_id, for background processing)
    pub passwords: HashMap<String, String>,
    /// Job creation time
    pub created_at: Instant,
    /// Job start time (when processing began)
    pub started_at: Option<Instant>,
    /// Index of currently processing account
    pub current_account_index: usize,
    /// Download base path
    pub download_path: PathBuf,
    /// Session folder path (YYYYMMDD_HHMMSS format)
    pub session_folder: Option<PathBuf>,
    /// Run in headless mode
    pub headless: bool,
    /// Last error message
    pub last_error: Option<String>,
}

impl JobState {
    /// Create a new job state
    pub fn new(
        job_id: String,
        accounts: Vec<(String, String, String)>, // (user_id, password, name)
        download_path: PathBuf,
        headless: bool,
    ) -> Self {
        let mut account_map = HashMap::new();
        let mut account_order = Vec::new();
        let mut passwords = HashMap::new();

        for (user_id, password, name) in accounts {
            account_order.push(user_id.clone());
            account_map.insert(user_id.clone(), AccountResult::new(user_id.clone(), name));
            passwords.insert(user_id, password);
        }

        Self {
            job_id,
            status: JobStatus::Queued,
            accounts: account_map,
            account_order,
            passwords,
            created_at: Instant::now(),
            started_at: None,
            current_account_index: 0,
            download_path,
            session_folder: None,
            headless,
            last_error: None,
        }
    }

    /// Mark job as started
    pub fn start(&mut self) {
        self.status = JobStatus::Running;
        self.started_at = Some(Instant::now());
    }

    /// Set the session folder path
    pub fn set_session_folder(&mut self, folder: PathBuf) {
        self.session_folder = Some(folder);
    }

    /// Get session folder path
    pub fn get_session_folder(&self) -> Option<&PathBuf> {
        self.session_folder.as_ref()
    }

    /// Get the current account user_id being processed
    pub fn current_account_user_id(&self) -> Option<&String> {
        self.account_order.get(self.current_account_index)
    }

    /// Get password for an account
    pub fn get_password(&self, user_id: &str) -> Option<&String> {
        self.passwords.get(user_id)
    }

    /// Move to the next account
    pub fn advance_to_next_account(&mut self) {
        self.current_account_index += 1;
    }

    /// Set the last error message
    pub fn set_last_error(&mut self, error: String) {
        self.last_error = Some(error);
    }

    /// Get success count
    pub fn success_count(&self) -> usize {
        self.accounts
            .values()
            .filter(|a| a.status == JobStatus::Completed)
            .count()
    }

    /// Get fail count
    pub fn fail_count(&self) -> usize {
        self.accounts
            .values()
            .filter(|a| a.status == JobStatus::Failed)
            .count()
    }

    /// Get the number of completed accounts
    pub fn completed_count(&self) -> usize {
        self.accounts
            .values()
            .filter(|a| a.status == JobStatus::Completed || a.status == JobStatus::Failed)
            .count()
    }

    /// Get the total number of accounts
    pub fn total_count(&self) -> usize {
        self.accounts.len()
    }

    /// Check if all accounts are processed
    pub fn is_complete(&self) -> bool {
        self.completed_count() == self.total_count()
    }

    /// Update overall status based on account results
    pub fn update_overall_status(&mut self) {
        if self.is_complete() {
            // Check if any account failed
            let has_failures = self
                .accounts
                .values()
                .any(|a| a.status == JobStatus::Failed);

            if has_failures {
                self.status = JobStatus::Failed;
            } else {
                self.status = JobStatus::Completed;
            }
        } else if self.accounts.values().any(|a| a.status == JobStatus::Running) {
            self.status = JobStatus::Running;
        }
    }

    /// Get result for a specific account
    pub fn get_account_result(&self, user_id: &str) -> Option<&AccountResult> {
        self.accounts.get(user_id)
    }

    /// Get mutable result for a specific account
    pub fn get_account_result_mut(&mut self, user_id: &str) -> Option<&mut AccountResult> {
        self.accounts.get_mut(user_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_state_creation() {
        let accounts = vec![
            ("user1".to_string(), "pass1".to_string(), "User One".to_string()),
            ("user2".to_string(), "pass2".to_string(), "User Two".to_string()),
        ];

        let state = JobState::new(
            "job-123".to_string(),
            accounts,
            PathBuf::from("./downloads"),
            true,
        );

        assert_eq!(state.job_id, "job-123");
        assert_eq!(state.status, JobStatus::Queued);
        assert_eq!(state.total_count(), 2);
        assert_eq!(state.completed_count(), 0);
        assert!(!state.is_complete());
    }

    #[test]
    fn test_account_result_transitions() {
        let mut result = AccountResult::new("user1".to_string(), "User One".to_string());
        assert_eq!(result.status, JobStatus::Queued);

        result.set_running();
        assert_eq!(result.status, JobStatus::Running);

        result.set_completed(PathBuf::from("./test.csv"));
        assert_eq!(result.status, JobStatus::Completed);
        assert!(result.csv_path.is_some());
    }
}
