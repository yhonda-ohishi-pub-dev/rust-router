//! PDF Generator gRPC service implementation

use std::path::PathBuf;
use tonic::{Request, Response, Status};
use tower::Service;

use crate::grpc::pdf_server::{
    GeneratePdfRequest, GeneratePdfResponse,
    PrintPdfRequest, PrintPdfResponse,
    PdfHealthRequest, PdfHealthResponse,
    Item as ProtoItem, Ryohi as ProtoRyohi,
    pdf_generator_server::PdfGenerator,
};

// print-pdf-service からインポート
use print_pdf_service::{
    PdfService as InternalPdfService,
    PdfRequest as InternalPdfRequest,
    Item as InternalItem,
    Ryohi as InternalRyohi,
    SumatraPrinter,
};

/// PDF Generator gRPC service implementation
pub struct PdfGeneratorService {
    output_path: PathBuf,
}

impl PdfGeneratorService {
    /// Create a new PdfGeneratorService
    pub fn new() -> Self {
        Self {
            output_path: std::env::temp_dir().join("gateway-pdf"),
        }
    }

    /// Create with custom output path
    pub fn with_output_path(output_path: PathBuf) -> Self {
        Self { output_path }
    }
}

impl Default for PdfGeneratorService {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert proto Item to internal Item
fn convert_item(proto_item: &ProtoItem) -> InternalItem {
    InternalItem {
        car: proto_item.car.clone(),
        name: proto_item.name.clone(),
        purpose: if proto_item.purpose.is_empty() {
            None
        } else {
            Some(proto_item.purpose.clone())
        },
        start_date: if proto_item.start_date.is_empty() {
            None
        } else {
            Some(proto_item.start_date.clone())
        },
        end_date: if proto_item.end_date.is_empty() {
            None
        } else {
            Some(proto_item.end_date.clone())
        },
        price: proto_item.price,
        tax: if proto_item.tax == 0.0 {
            None
        } else {
            Some(proto_item.tax)
        },
        description: None, // proto には含まれていない
        ryohi: proto_item.ryohi.iter().map(convert_ryohi).collect(),
        office: if proto_item.office.is_empty() {
            None
        } else {
            Some(proto_item.office.clone())
        },
        pay_day: if proto_item.pay_day.is_empty() {
            None
        } else {
            Some(proto_item.pay_day.clone())
        },
    }
}

/// Convert proto Ryohi to internal Ryohi
fn convert_ryohi(proto_ryohi: &ProtoRyohi) -> InternalRyohi {
    InternalRyohi {
        date: if proto_ryohi.date.is_empty() {
            None
        } else {
            Some(proto_ryohi.date.clone())
        },
        date_ar: None,
        dest: if proto_ryohi.dest.is_empty() {
            None
        } else {
            Some(proto_ryohi.dest.clone())
        },
        dest_ar: None,
        detail: proto_ryohi.detail.clone(),
        kukan: if proto_ryohi.kukan.is_empty() {
            None
        } else {
            Some(proto_ryohi.kukan.clone())
        },
        kukan_sprit: None,
        price: if proto_ryohi.price == 0 {
            None
        } else {
            Some(proto_ryohi.price)
        },
        price_ar: None,
        vol: if proto_ryohi.vol == 0.0 {
            None
        } else {
            Some(proto_ryohi.vol)
        },
        vol_ar: None,
        // 印刷用フィールドはデフォルト (PDF生成時に自動設定される)
        print_detail: None,
        print_detail_row: None,
        print_kukan: None,
        print_kukan_row: None,
        max_row: None,
        page_count: None,
    }
}

#[tonic::async_trait]
impl PdfGenerator for PdfGeneratorService {
    /// Generate PDF only
    async fn generate_pdf(
        &self,
        request: Request<GeneratePdfRequest>,
    ) -> Result<Response<GeneratePdfResponse>, Status> {
        let req = request.into_inner();

        if req.items.is_empty() {
            return Err(Status::invalid_argument("At least one item is required"));
        }

        tracing::info!("GeneratePdf requested with {} items", req.items.len());

        // Convert proto items to internal items
        let items: Vec<InternalItem> = req.items.iter().map(convert_item).collect();

        // Determine output path
        let output_path = if req.output_path.is_empty() {
            self.output_path.join(format!(
                "ryohi_{}.pdf",
                chrono::Local::now().format("%Y%m%d_%H%M%S")
            ))
        } else {
            PathBuf::from(&req.output_path)
        };

        // Ensure output directory exists
        if let Some(parent) = output_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                tracing::error!("Failed to create output directory: {}", e);
                return Ok(Response::new(GeneratePdfResponse {
                    success: false,
                    message: format!("Failed to create output directory: {}", e),
                    pdf_path: String::new(),
                    pdf_content: vec![],
                }));
            }
        }

        // Create PDF using internal service
        let mut service = InternalPdfService::new();
        let internal_req = InternalPdfRequest::new(items)
            .with_output_path(&output_path);

        match service.call(internal_req).await {
            Ok(result) => {
                // Read PDF content
                let pdf_content = tokio::fs::read(&result.pdf_path)
                    .await
                    .unwrap_or_default();

                Ok(Response::new(GeneratePdfResponse {
                    success: true,
                    message: "PDF generated successfully".to_string(),
                    pdf_path: result.pdf_path.to_string_lossy().to_string(),
                    pdf_content,
                }))
            }
            Err(e) => {
                tracing::error!("PDF generation failed: {}", e);
                Ok(Response::new(GeneratePdfResponse {
                    success: false,
                    message: format!("PDF generation failed: {}", e),
                    pdf_path: String::new(),
                    pdf_content: vec![],
                }))
            }
        }
    }

    /// Generate PDF and print
    async fn print_pdf(
        &self,
        request: Request<PrintPdfRequest>,
    ) -> Result<Response<PrintPdfResponse>, Status> {
        let req = request.into_inner();

        if req.items.is_empty() {
            return Err(Status::invalid_argument("At least one item is required"));
        }

        tracing::info!(
            "PrintPdf requested with {} items, printer: {:?}",
            req.items.len(),
            if req.printer_name.is_empty() {
                "default"
            } else {
                &req.printer_name
            }
        );

        // Convert proto items to internal items
        let items: Vec<InternalItem> = req.items.iter().map(convert_item).collect();

        // Generate PDF
        let output_path = self.output_path.join(format!(
            "ryohi_{}.pdf",
            chrono::Local::now().format("%Y%m%d_%H%M%S")
        ));

        // Ensure output directory exists
        if let Some(parent) = output_path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                tracing::error!("Failed to create output directory: {}", e);
                return Ok(Response::new(PrintPdfResponse {
                    success: false,
                    message: format!("Failed to create output directory: {}", e),
                    pdf_path: String::new(),
                }));
            }
        }

        // Create PDF using internal service with print flag
        let mut service = InternalPdfService::new();
        let mut internal_req = InternalPdfRequest::new(items)
            .with_output_path(&output_path)
            .with_print(true);

        if !req.printer_name.is_empty() {
            internal_req = internal_req.with_printer_name(&req.printer_name);
        }

        match service.call(internal_req).await {
            Ok(result) => {
                Ok(Response::new(PrintPdfResponse {
                    success: true,
                    message: "PDF generated and printed successfully".to_string(),
                    pdf_path: result.pdf_path.to_string_lossy().to_string(),
                }))
            }
            Err(e) => {
                tracing::error!("PDF print failed: {}", e);
                Ok(Response::new(PrintPdfResponse {
                    success: false,
                    message: format!("PDF print failed: {}", e),
                    pdf_path: String::new(),
                }))
            }
        }
    }

    /// Health check
    async fn health(
        &self,
        _request: Request<PdfHealthRequest>,
    ) -> Result<Response<PdfHealthResponse>, Status> {
        tracing::debug!("PDF health check requested");

        // Check if SumatraPDF is available
        let sumatra_available = {
            let mut printer = SumatraPrinter::new();
            printer.find_sumatra().is_ok()
        };

        Ok(Response::new(PdfHealthResponse {
            healthy: true,
            version: env!("CARGO_PKG_VERSION").to_string(),
            sumatra_available,
        }))
    }
}
