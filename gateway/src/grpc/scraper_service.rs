use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::RwLock;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::config::GatewayConfig;
use crate::job::{JobQueue, JobStatus as InternalJobStatus};
use crate::grpc::gateway_server::proto::etc_scraper_server::EtcScraper;
use crate::grpc::gateway_server::proto::{
    AccountResult as ProtoAccountResult, GetDownloadedFilesRequest,
    GetDownloadedFilesResponse, ScraperHealthRequest, ScraperHealthResponse, JobStatus as ProtoJobStatus,
    ScrapeMultipleRequest, ScrapeMultipleResponse, ScrapeRequest, ScrapeResponse,
    StreamDownloadChunk, StreamDownloadRequest,
};

/// ETC Scraper gRPC service implementation
pub struct EtcScraperService {
    config: GatewayConfig,
    job_queue: Arc<RwLock<JobQueue>>,
}

impl EtcScraperService {
    /// Create a new EtcScraperService
    pub fn new(config: GatewayConfig, job_queue: Arc<RwLock<JobQueue>>) -> Self {
        Self { config, job_queue }
    }
}

/// Convert internal job status to proto job status
fn to_proto_status(status: InternalJobStatus) -> i32 {
    match status {
        InternalJobStatus::Queued => ProtoJobStatus::Queued as i32,
        InternalJobStatus::Running => ProtoJobStatus::Running as i32,
        InternalJobStatus::Completed => ProtoJobStatus::Completed as i32,
        InternalJobStatus::Failed => ProtoJobStatus::Failed as i32,
    }
}

#[tonic::async_trait]
impl EtcScraper for EtcScraperService {
    /// Health check RPC
    async fn health(
        &self,
        _request: Request<ScraperHealthRequest>,
    ) -> Result<Response<ScraperHealthResponse>, Status> {
        tracing::info!("Scraper health check requested");

        let response = ScraperHealthResponse {
            healthy: true,
            version: self.config.version.clone(),
            message: "ETC Scraper is running".to_string(),
        };

        Ok(Response::new(response))
    }

    /// Single account scrape RPC (synchronous)
    async fn scrape(
        &self,
        request: Request<ScrapeRequest>,
    ) -> Result<Response<ScrapeResponse>, Status> {
        let req = request.into_inner();

        let account = req.account.ok_or_else(|| {
            Status::invalid_argument("Account is required")
        })?;

        tracing::info!(
            "Scrape requested for account: {} ({})",
            account.name,
            account.user_id
        );

        // TODO: Integrate with scraper-service via InProcess call
        // For now, return a stub response
        let response = ScrapeResponse {
            success: false,
            message: "Scraper service not yet integrated".to_string(),
            csv_path: String::new(),
            csv_content: Vec::new(),
        };

        Ok(Response::new(response))
    }

    /// Multiple accounts scrape RPC (asynchronous job)
    async fn scrape_multiple(
        &self,
        request: Request<ScrapeMultipleRequest>,
    ) -> Result<Response<ScrapeMultipleResponse>, Status> {
        let req = request.into_inner();

        if req.accounts.is_empty() {
            return Err(Status::invalid_argument("At least one account is required"));
        }

        let download_path = if req.download_path.is_empty() {
            self.config.download_path.clone()
        } else {
            std::path::PathBuf::from(&req.download_path)
        };

        // Convert proto accounts to internal format
        let accounts: Vec<(String, String, String)> = req
            .accounts
            .into_iter()
            .map(|a| (a.user_id, a.password, a.name))
            .collect();

        let account_count = accounts.len();

        // Create and queue the job
        let job_id = {
            let mut queue = self.job_queue.write().await;
            queue.create_job(accounts, download_path, req.headless)
        };

        tracing::info!(
            "Created scrape job {} with {} accounts",
            job_id,
            account_count
        );

        let response = ScrapeMultipleResponse {
            job_id,
            message: format!("Job queued with {} accounts", account_count),
        };

        Ok(Response::new(response))
    }

    /// Get downloaded files for a job
    async fn get_downloaded_files(
        &self,
        request: Request<GetDownloadedFilesRequest>,
    ) -> Result<Response<GetDownloadedFilesResponse>, Status> {
        let req = request.into_inner();

        let queue = self.job_queue.read().await;
        let job_state = queue
            .get_job(&req.job_id)
            .ok_or_else(|| Status::not_found(format!("Job not found: {}", req.job_id)))?;

        let results: Vec<ProtoAccountResult> = job_state
            .accounts
            .values()
            .map(|a| ProtoAccountResult {
                user_id: a.user_id.clone(),
                name: a.name.clone(),
                status: to_proto_status(a.status),
                csv_path: a.csv_path.as_ref().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
                error_message: a.error_message.clone().unwrap_or_default(),
            })
            .collect();

        let response = GetDownloadedFilesResponse {
            job_id: job_state.job_id.clone(),
            overall_status: to_proto_status(job_state.status),
            results,
            completed_count: job_state.completed_count() as i32,
            total_count: job_state.total_count() as i32,
        };

        Ok(Response::new(response))
    }

    /// Stream type for StreamDownload RPC
    type StreamDownloadStream =
        Pin<Box<dyn Stream<Item = Result<StreamDownloadChunk, Status>> + Send>>;

    /// Stream download file content
    async fn stream_download(
        &self,
        request: Request<StreamDownloadRequest>,
    ) -> Result<Response<Self::StreamDownloadStream>, Status> {
        let req = request.into_inner();

        let queue = self.job_queue.read().await;
        let job_state = queue
            .get_job(&req.job_id)
            .ok_or_else(|| Status::not_found(format!("Job not found: {}", req.job_id)))?;

        // Find the file to stream
        let csv_path = if req.user_id.is_empty() {
            // Stream first available file
            job_state
                .accounts
                .values()
                .find_map(|a| a.csv_path.clone())
                .ok_or_else(|| Status::not_found("No downloaded files available"))?
        } else {
            // Stream specific account's file
            job_state
                .get_account_result(&req.user_id)
                .and_then(|a| a.csv_path.clone())
                .ok_or_else(|| {
                    Status::not_found(format!("No file for account: {}", req.user_id))
                })?
        };

        let filename = csv_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "download.csv".to_string());

        // Read file and create stream
        let content = tokio::fs::read(&csv_path).await.map_err(|e| {
            Status::internal(format!("Failed to read file: {}", e))
        })?;

        // Create a stream that sends the content in chunks
        let chunk_size = 32 * 1024; // 32KB chunks
        let chunks: Vec<_> = content
            .chunks(chunk_size)
            .enumerate()
            .map(|(i, chunk)| {
                let is_last = (i + 1) * chunk_size >= content.len();
                Ok(StreamDownloadChunk {
                    data: chunk.to_vec(),
                    filename: if i == 0 { filename.clone() } else { String::new() },
                    is_last,
                })
            })
            .collect();

        let stream = tokio_stream::iter(chunks);

        Ok(Response::new(Box::pin(stream)))
    }
}
