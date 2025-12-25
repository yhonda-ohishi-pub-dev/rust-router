//! Integration tests for gateway with timecard-service
//!
//! These tests verify the InProcess call integration between
//! gateway and timecard-service.

use gateway_lib::ServiceRouter;

#[tokio::test]
async fn test_create_and_get_timecard() {
    let router = ServiceRouter::new();

    // Create a timecard
    let result = router
        .create_timecard("EMP001", "2024-01-15", "09:00", "18:00")
        .await;
    assert!(result.is_ok(), "Failed to create timecard: {:?}", result.err());

    // Get the timecard
    let timecard = router.get_timecard("EMP001", "2024-01-15").await.unwrap();
    assert_eq!(timecard.employee_id, "EMP001");
    assert_eq!(timecard.clock_in, "09:00");
    assert_eq!(timecard.clock_out, "18:00");
}

#[tokio::test]
async fn test_get_nonexistent_timecard() {
    let router = ServiceRouter::new();

    // Try to get a timecard that doesn't exist
    let result = router.get_timecard("NONEXISTENT", "2024-01-15").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_invalid_time_format() {
    let router = ServiceRouter::new();

    // Try to create a timecard with invalid time format
    let result = router
        .create_timecard("EMP001", "invalid-date", "09:00", "18:00")
        .await;
    assert!(result.is_err());
}
