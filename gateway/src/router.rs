//! Service Router
//!
//! Routes requests to internal services via InProcess calls using tower::ServiceExt.
//! This enables direct function calls without network overhead.

use anyhow::Result;
use timecard_service::TimecardService;

/// Timecard data for gateway communication
pub struct TimecardData {
    pub employee_id: String,
    pub date: String,
    pub clock_in: String,
    pub clock_out: String,
}

/// Service router that manages InProcess service calls
pub struct ServiceRouter {
    timecard_service: TimecardService,
}

impl ServiceRouter {
    pub fn new() -> Self {
        Self {
            timecard_service: TimecardService::new(),
        }
    }

    /// Get timecard via InProcess call to timecard service
    pub async fn get_timecard(&self, employee_id: &str, date: &str) -> Result<TimecardData> {
        let entry = self.timecard_service.get_entry(employee_id, date).await?;

        Ok(TimecardData {
            employee_id: entry.employee_id,
            date: entry.date.to_string(),
            clock_in: entry.clock_in.map(|t| t.format("%H:%M").to_string()).unwrap_or_default(),
            clock_out: entry.clock_out.map(|t| t.format("%H:%M").to_string()).unwrap_or_default(),
        })
    }

    /// Create timecard via InProcess call to timecard service
    pub async fn create_timecard(
        &self,
        employee_id: &str,
        date: &str,
        clock_in: &str,
        clock_out: &str,
    ) -> Result<()> {
        self.timecard_service
            .create_entry(employee_id, date, clock_in, clock_out)
            .await?;
        Ok(())
    }
}

impl Default for ServiceRouter {
    fn default() -> Self {
        Self::new()
    }
}
