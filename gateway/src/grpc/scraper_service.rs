use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use chrono::Local;
use tokio::sync::RwLock;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
use tower::Service;

use crate::config::GatewayConfig;
use crate::job::{JobQueue, JobStatus};
use crate::grpc::scraper_server::etc_scraper_server::EtcScraper;
use crate::grpc::scraper_server::{
    DownloadedFile, GetDownloadedFilesRequest, GetDownloadedFilesResponse,
    HealthRequest, HealthResponse, JobStatus as ProtoJobStatus,
    ScrapeMultipleRequest, ScrapeMultipleResponse, ScrapeRequest, ScrapeResponse,
    ScrapeResult, StreamDownloadChunk, StreamDownloadRequest,
};

// scraper-service クレートからインポート
use scraper_service::{
    ScraperService as InternalScraperService,
    ScrapeRequest as InternalScrapeRequest,
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

#[tonic::async_trait]
impl EtcScraper for EtcScraperService {
    /// Health check RPC
    async fn health(
        &self,
        _request: Request<HealthRequest>,
    ) -> Result<Response<HealthResponse>, Status> {
        tracing::info!("Scraper health check requested");

        // Get current job status from the queue
        let queue = self.job_queue.read().await;
        let current_job = if let Some(job) = queue.current_job() {
            let current_account = job
                .current_account_user_id()
                .cloned()
                .unwrap_or_default();

            let started_at = job.started_at
                .map(|t| {
                    let elapsed = t.elapsed().as_secs();
                    format!("{}s ago", elapsed)
                })
                .unwrap_or_default();

            Some(ProtoJobStatus {
                is_running: job.status == JobStatus::Running,
                started_at,
                total_accounts: job.total_count() as i32,
                completed_accounts: job.completed_count() as i32,
                success_count: job.success_count() as i32,
                fail_count: job.fail_count() as i32,
                current_account,
                last_error: job.last_error.clone().unwrap_or_default(),
            })
        } else {
            Some(ProtoJobStatus {
                is_running: false,
                started_at: String::new(),
                total_accounts: 0,
                completed_accounts: 0,
                success_count: 0,
                fail_count: 0,
                current_account: String::new(),
                last_error: String::new(),
            })
        };

        // 最新のセッションフォルダを取得
        let last_session_folder = {
            let queue = self.job_queue.read().await;
            queue.current_job()
                .and_then(|job| job.get_session_folder())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default()
        };

        let response = HealthResponse {
            healthy: true,
            version: self.config.version.clone(),
            current_job,
            last_session_folder,
        };

        Ok(Response::new(response))
    }

    /// Single account scrape RPC (synchronous)
    async fn scrape(
        &self,
        request: Request<ScrapeRequest>,
    ) -> Result<Response<ScrapeResponse>, Status> {
        let req = request.into_inner();

        if req.user_id.is_empty() || req.password.is_empty() {
            return Err(Status::invalid_argument("user_id and password are required"));
        }

        tracing::info!("Scrape requested for user: {}", req.user_id);

        // scraper-service を使用してスクレイピング実行
        let mut scraper = InternalScraperService::new();
        let internal_req = InternalScrapeRequest::new(&req.user_id, &req.password)
            .with_download_path(&self.config.download_path)
            .with_headless(self.config.default_headless);

        match scraper.call(internal_req).await {
            Ok(result) => {
                let csv_content = String::from_utf8_lossy(&result.csv_content).to_string();
                let response = ScrapeResponse {
                    success: true,
                    message: "Scrape completed successfully".to_string(),
                    csv_path: result.csv_path.to_string_lossy().to_string(),
                    csv_content,
                };
                Ok(Response::new(response))
            }
            Err(e) => {
                tracing::error!("Scrape failed for user {}: {}", req.user_id, e);
                let response = ScrapeResponse {
                    success: false,
                    message: format!("Scrape failed: {}", e),
                    csv_path: String::new(),
                    csv_content: String::new(),
                };
                Ok(Response::new(response))
            }
        }
    }

    /// Multiple accounts scrape RPC (async - returns immediately, processes in background)
    async fn scrape_multiple(
        &self,
        request: Request<ScrapeMultipleRequest>,
    ) -> Result<Response<ScrapeMultipleResponse>, Status> {
        let req = request.into_inner();

        if req.accounts.is_empty() {
            return Err(Status::invalid_argument("At least one account is required"));
        }

        let account_count = req.accounts.len();
        tracing::info!("ScrapeMultiple requested with {} accounts (async mode)", account_count);

        // アカウント情報を (user_id, password, name) の形式に変換
        // proto には name がないので user_id を使用
        let accounts: Vec<(String, String, String)> = req
            .accounts
            .iter()
            .map(|a| (a.user_id.clone(), a.password.clone(), a.user_id.clone()))
            .collect();

        // セッションフォルダを作成 (YYYYMMDD_HHMMSS形式)
        let session_folder_name = Local::now().format("%Y%m%d_%H%M%S").to_string();
        let session_folder = self.config.download_path.join(&session_folder_name);

        // ディレクトリを作成
        if let Err(e) = tokio::fs::create_dir_all(&session_folder).await {
            tracing::error!("Failed to create session folder: {}", e);
            return Err(Status::internal(format!("Failed to create session folder: {}", e)));
        }
        tracing::info!("Created session folder: {:?}", session_folder);

        // ジョブを作成してキューに追加
        let job_id = {
            let mut queue = self.job_queue.write().await;
            let job_id = queue.create_job(
                accounts,
                self.config.download_path.clone(),
                true, // headless mode
            );
            // セッションフォルダを設定
            if let Some(job) = queue.get_job_mut(&job_id) {
                job.set_session_folder(session_folder.clone());
            }
            tracing::info!("Created job {} with {} accounts", job_id, account_count);
            job_id
        };

        // バックグラウンドでジョブを処理
        let job_queue = Arc::clone(&self.job_queue);
        tokio::spawn(async move {
            process_job_in_background(job_queue, job_id, session_folder).await;
        });

        // 即座にレスポンスを返す（results は空、処理は Health API でポーリング）
        let response = ScrapeMultipleResponse {
            results: vec![],
            success_count: 0,
            total_count: account_count as i32,
        };

        Ok(Response::new(response))
    }

    /// Get downloaded files
    async fn get_downloaded_files(
        &self,
        _request: Request<GetDownloadedFilesRequest>,
    ) -> Result<Response<GetDownloadedFilesResponse>, Status> {
        let download_path = std::path::Path::new(&self.config.download_path);

        if !download_path.exists() {
            return Ok(Response::new(GetDownloadedFilesResponse {
                files: vec![],
                session_folder: String::new(),
            }));
        }

        let mut files: Vec<DownloadedFile> = vec![];

        // ダウンロードディレクトリ内のファイルを一覧
        let mut entries = tokio::fs::read_dir(download_path).await.map_err(|e| {
            Status::internal(format!("Failed to read download directory: {}", e))
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            Status::internal(format!("Failed to read directory entry: {}", e))
        })? {
            let path = entry.path();
            if path.is_file() {
                // ファイル内容を読み込む
                let content = tokio::fs::read(&path).await.map_err(|e| {
                    Status::internal(format!("Failed to read file: {}", e))
                })?;

                files.push(DownloadedFile {
                    filename: path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default(),
                    content,
                });
            }
        }

        let response = GetDownloadedFilesResponse {
            files,
            session_folder: self.config.download_path.to_string_lossy().to_string(),
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

        // session_folderが空の場合は最新のセッションフォルダを自動選択
        let session_folder = if req.session_folder.is_empty() {
            // まず現在のジョブからセッションフォルダを取得
            let current_session = {
                let queue = self.job_queue.read().await;
                queue.current_job()
                    .and_then(|job| job.get_session_folder())
                    .map(|p| p.to_string_lossy().to_string())
            };

            if let Some(folder) = current_session {
                folder
            } else {
                // ジョブがない場合は、ダウンロードディレクトリ内の最新フォルダを探す
                let download_path = &self.config.download_path;
                match find_latest_session_folder(download_path).await {
                    Some(folder) => folder.to_string_lossy().to_string(),
                    None => {
                        // フォルダがない場合はデフォルトのダウンロードディレクトリを使用
                        download_path.to_string_lossy().to_string()
                    }
                }
            }
        } else {
            req.session_folder
        };

        tracing::info!("StreamDownload requested for folder: {}", session_folder);

        let session_path = std::path::PathBuf::from(&session_folder);
        if !session_path.exists() {
            return Err(Status::not_found(format!("Session folder not found: {}", session_folder)));
        }

        // List files in session folder
        let mut files: Vec<std::path::PathBuf> = vec![];
        let mut entries = tokio::fs::read_dir(&session_path).await.map_err(|e| {
            Status::internal(format!("Failed to read session folder: {}", e))
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            Status::internal(format!("Failed to read directory entry: {}", e))
        })? {
            let path = entry.path();
            if path.is_file() {
                files.push(path);
            }
        }

        if files.is_empty() {
            return Err(Status::not_found("No files in session folder"));
        }

        let total_files = files.len() as i32;

        // Create a stream that sends all files in chunks
        let chunk_size = 32 * 1024; // 32KB chunks
        let stream = async_stream::try_stream! {
            for (file_index, file_path) in files.into_iter().enumerate() {
                let filename = file_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string());

                let content = tokio::fs::read(&file_path).await.map_err(|e| {
                    Status::internal(format!("Failed to read file: {}", e))
                })?;

                let total_size = content.len() as i64;
                let chunks: Vec<_> = content.chunks(chunk_size).collect();
                let total_chunks = chunks.len();

                for (i, chunk) in chunks.into_iter().enumerate() {
                    let offset = (i * chunk_size) as i64;
                    let is_last_chunk = i + 1 == total_chunks;

                    yield StreamDownloadChunk {
                        filename: filename.clone(),
                        data: chunk.to_vec(),
                        offset,
                        total_size,
                        is_last_chunk,
                        file_index: file_index as i32,
                        total_files,
                    };
                }
            }
        };

        Ok(Response::new(Box::pin(stream)))
    }
}

/// バックグラウンドでジョブを処理する関数
async fn process_job_in_background(
    job_queue: Arc<RwLock<JobQueue>>,
    job_id: String,
    session_folder: PathBuf,
) {
    tracing::info!("Starting background job processing for {}", job_id);

    // ジョブを開始状態に設定
    {
        let mut queue = job_queue.write().await;
        queue.set_current_job(&job_id);
        if let Some(job) = queue.get_job_mut(&job_id) {
            job.start();
        }
    }

    // ジョブからアカウント情報を取得
    let (accounts, headless) = {
        let queue = job_queue.read().await;
        if let Some(job) = queue.get_job(&job_id) {
            let accounts: Vec<(String, String)> = job
                .account_order
                .iter()
                .filter_map(|user_id| {
                    job.get_password(user_id).map(|pwd| (user_id.clone(), pwd.clone()))
                })
                .collect();
            (accounts, job.headless)
        } else {
            tracing::error!("Job {} not found", job_id);
            return;
        }
    };

    // 各アカウントを順次処理
    for (idx, (user_id, password)) in accounts.iter().enumerate() {
        tracing::info!("Processing account {}/{}: {}", idx + 1, accounts.len(), user_id);

        // 現在のアカウントインデックスを更新
        {
            let mut queue = job_queue.write().await;
            if let Some(job) = queue.get_job_mut(&job_id) {
                job.current_account_index = idx;
                // アカウントの状態を Running に設定
                if let Some(account) = job.get_account_result_mut(user_id) {
                    account.set_running();
                }
            }
        }

        // スクレイピング実行（セッションフォルダに保存）
        let mut scraper = InternalScraperService::new();
        let internal_req = InternalScrapeRequest::new(user_id, password)
            .with_download_path(&session_folder)
            .with_headless(headless);

        let result = scraper.call(internal_req).await;

        // 結果を更新
        {
            let mut queue = job_queue.write().await;
            if let Some(job) = queue.get_job_mut(&job_id) {
                if let Some(account) = job.get_account_result_mut(user_id) {
                    match result {
                        Ok(scrape_result) => {
                            tracing::info!("Scrape succeeded for {}", user_id);
                            account.set_completed(scrape_result.csv_path);
                        }
                        Err(e) => {
                            let error_msg = format!("Scrape failed: {}", e);
                            tracing::error!("{} for user {}", error_msg, user_id);
                            account.set_failed(error_msg.clone());
                            job.set_last_error(error_msg);
                        }
                    }
                }
                job.update_overall_status();
            }
        }
    }

    // ジョブ完了
    {
        let mut queue = job_queue.write().await;
        if let Some(job) = queue.get_job_mut(&job_id) {
            job.update_overall_status();
            tracing::info!(
                "Job {} completed: {}/{} succeeded",
                job_id,
                job.success_count(),
                job.total_count()
            );
        }
        queue.clear_current_job();
    }
}

/// ダウンロードディレクトリ内の最新のセッションフォルダを探す
/// セッションフォルダは YYYYMMDD_HHMMSS 形式の名前を持つ
async fn find_latest_session_folder(download_path: &std::path::Path) -> Option<PathBuf> {
    if !download_path.exists() {
        return None;
    }

    let mut entries = tokio::fs::read_dir(download_path).await.ok()?;
    let mut folders: Vec<(String, PathBuf)> = Vec::new();

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                // YYYYMMDD_HHMMSS 形式かどうかチェック (15文字)
                if name.len() == 15 && name.chars().nth(8) == Some('_') {
                    folders.push((name.to_string(), path));
                }
            }
        }
    }

    // 名前でソートして最新のものを返す（降順）
    folders.sort_by(|a, b| b.0.cmp(&a.0));
    folders.into_iter().next().map(|(_, path)| path)
}
