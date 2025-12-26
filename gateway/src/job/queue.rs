use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use uuid::Uuid;

use super::state::{JobState, JobStatus};

/// Job queue for managing multiple scrape jobs
#[derive(Debug, Default)]
pub struct JobQueue {
    /// All jobs (keyed by job_id)
    jobs: HashMap<String, JobState>,
    /// Queue of pending job IDs (in order)
    pending: Vec<String>,
    /// Currently running job ID
    current_job_id: Option<String>,
}

impl JobQueue {
    /// Create a new empty job queue
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new job and add it to the queue
    /// Returns the job ID
    pub fn create_job(
        &mut self,
        accounts: Vec<(String, String, String)>, // (user_id, password, name)
        download_path: PathBuf,
        headless: bool,
    ) -> String {
        let job_id = Uuid::new_v4().to_string();
        let job_state = JobState::new(job_id.clone(), accounts, download_path, headless);

        self.jobs.insert(job_id.clone(), job_state);
        self.pending.push(job_id.clone());

        job_id
    }

    /// Get a job by ID
    pub fn get_job(&self, job_id: &str) -> Option<&JobState> {
        self.jobs.get(job_id)
    }

    /// Get a mutable job by ID
    pub fn get_job_mut(&mut self, job_id: &str) -> Option<&mut JobState> {
        self.jobs.get_mut(job_id)
    }

    /// Get the next pending job ID
    pub fn next_pending(&self) -> Option<&String> {
        self.pending.first()
    }

    /// Remove a job from the pending queue
    pub fn mark_started(&mut self, job_id: &str) {
        self.pending.retain(|id| id != job_id);
    }

    /// Get all job IDs
    pub fn all_job_ids(&self) -> Vec<String> {
        self.jobs.keys().cloned().collect()
    }

    /// Get pending job count
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Remove completed jobs older than the specified duration
    pub fn cleanup_old_jobs(&mut self, max_age_secs: u64) {
        let now = Instant::now();
        self.jobs.retain(|_, job| {
            let age = now.duration_since(job.created_at).as_secs();
            age < max_age_secs
        });
    }

    /// Get the currently running job ID
    pub fn current_job_id(&self) -> Option<&String> {
        self.current_job_id.as_ref()
    }

    /// Get the currently running job
    pub fn current_job(&self) -> Option<&JobState> {
        self.current_job_id
            .as_ref()
            .and_then(|id| self.jobs.get(id))
    }

    /// Get the currently running job (mutable)
    pub fn current_job_mut(&mut self) -> Option<&mut JobState> {
        if let Some(id) = self.current_job_id.clone() {
            self.jobs.get_mut(&id)
        } else {
            None
        }
    }

    /// Set a job as currently running
    pub fn set_current_job(&mut self, job_id: &str) {
        self.current_job_id = Some(job_id.to_string());
        if let Some(job) = self.jobs.get_mut(job_id) {
            job.status = JobStatus::Running;
        }
        self.pending.retain(|id| id != job_id);
    }

    /// Clear the current job (when completed or failed)
    pub fn clear_current_job(&mut self) {
        self.current_job_id = None;
    }

    /// Check if there is a running job
    pub fn has_running_job(&self) -> bool {
        self.current_job_id.is_some()
    }

    /// Pop the next pending job and set it as current
    /// Returns the job ID if there was a pending job
    pub fn start_next_job(&mut self) -> Option<String> {
        if self.has_running_job() {
            return None; // Already has a running job
        }

        if let Some(job_id) = self.pending.first().cloned() {
            self.set_current_job(&job_id);
            Some(job_id)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_job() {
        let mut queue = JobQueue::new();
        let accounts = vec![
            ("user1".to_string(), "pass1".to_string(), "User One".to_string()),
        ];

        let job_id = queue.create_job(accounts, PathBuf::from("./downloads"), true);

        assert!(!job_id.is_empty());
        assert!(queue.get_job(&job_id).is_some());
        assert_eq!(queue.pending_count(), 1);
    }

    #[test]
    fn test_mark_started() {
        let mut queue = JobQueue::new();
        let accounts = vec![
            ("user1".to_string(), "pass1".to_string(), "User One".to_string()),
        ];

        let job_id = queue.create_job(accounts, PathBuf::from("./downloads"), true);
        assert_eq!(queue.pending_count(), 1);

        queue.mark_started(&job_id);
        assert_eq!(queue.pending_count(), 0);
    }
}
