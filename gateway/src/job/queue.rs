use std::collections::HashMap;
use std::path::PathBuf;

use uuid::Uuid;

use super::state::JobState;

/// Job queue for managing multiple scrape jobs
#[derive(Debug, Default)]
pub struct JobQueue {
    /// All jobs (keyed by job_id)
    jobs: HashMap<String, JobState>,
    /// Queue of pending job IDs (in order)
    pending: Vec<String>,
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
        let now = std::time::Instant::now();
        self.jobs.retain(|_, job| {
            let age = now.duration_since(job.created_at).as_secs();
            age < max_age_secs
        });
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
