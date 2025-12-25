//! Gateway gRPC service implementation
//!
//! This implements the GatewayService trait and routes requests
//! to internal services via InProcess calls.

use tonic::{Request, Response, Status};

use super::gateway_server::proto::gateway_service_server::GatewayService;
use super::gateway_server::proto::{
    CreateTimecardRequest, CreateTimecardResponse,
    GetTimecardRequest, GetTimecardResponse,
    HealthCheckRequest, HealthCheckResponse,
};

use crate::router::ServiceRouter;

/// Gateway service implementation
pub struct GatewayServiceImpl {
    router: ServiceRouter,
}

impl GatewayServiceImpl {
    pub fn new() -> Self {
        Self {
            router: ServiceRouter::new(),
        }
    }
}

impl Default for GatewayServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl GatewayService for GatewayServiceImpl {
    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let response = HealthCheckResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
        };
        Ok(Response::new(response))
    }

    async fn get_timecard(
        &self,
        request: Request<GetTimecardRequest>,
    ) -> Result<Response<GetTimecardResponse>, Status> {
        let req = request.into_inner();

        // Route to timecard service via InProcess call
        match self.router.get_timecard(&req.employee_id, &req.date).await {
            Ok(timecard) => {
                let response = GetTimecardResponse {
                    employee_id: timecard.employee_id,
                    date: timecard.date,
                    clock_in: timecard.clock_in,
                    clock_out: timecard.clock_out,
                };
                Ok(Response::new(response))
            }
            Err(e) => Err(Status::internal(e.to_string())),
        }
    }

    async fn create_timecard(
        &self,
        request: Request<CreateTimecardRequest>,
    ) -> Result<Response<CreateTimecardResponse>, Status> {
        let req = request.into_inner();

        // Route to timecard service via InProcess call
        match self.router.create_timecard(
            &req.employee_id,
            &req.date,
            &req.clock_in,
            &req.clock_out,
        ).await {
            Ok(_) => {
                let response = CreateTimecardResponse {
                    success: true,
                    message: "Timecard created successfully".to_string(),
                };
                Ok(Response::new(response))
            }
            Err(e) => {
                let response = CreateTimecardResponse {
                    success: false,
                    message: e.to_string(),
                };
                Ok(Response::new(response))
            }
        }
    }
}
