//! gRPC-Web over DataChannel handler
//!
//! Handles gRPC-Web protocol messages received over WebRTC DataChannel.
//!
//! ## Request Format
//! ```text
//! [path_len(4)][path(N)][headers_len(4)][headers_json(M)][grpc_frames]
//! ```
//!
//! ## Response Format
//! ```text
//! [headers_len(4)][headers_json(N)][data_frames...][trailer_frame]
//! ```
//!
//! ## gRPC-Web Frame Format
//! ```text
//! [flags(1)][length(4)][data(N)]
//! ```
//! - flags: 0x00 = data, 0x01 = trailer

use std::collections::HashMap;
use std::sync::Arc;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use tokio::sync::Mutex;
use tonic::body::BoxBody;
use tonic::Status;
use tower::Service;

/// gRPC status codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum StatusCode {
    Ok = 0,
    Cancelled = 1,
    Unknown = 2,
    InvalidArgument = 3,
    DeadlineExceeded = 4,
    NotFound = 5,
    AlreadyExists = 6,
    PermissionDenied = 7,
    ResourceExhausted = 8,
    FailedPrecondition = 9,
    Aborted = 10,
    OutOfRange = 11,
    Unimplemented = 12,
    Internal = 13,
    Unavailable = 14,
    DataLoss = 15,
    Unauthenticated = 16,
}

/// Parsed gRPC request from DataChannel
#[derive(Debug)]
pub struct GrpcRequest {
    pub path: String,
    pub headers: HashMap<String, String>,
    pub message: Vec<u8>,
}

/// gRPC response to send back via DataChannel
#[derive(Debug)]
pub struct GrpcResponse {
    pub headers: HashMap<String, String>,
    pub messages: Vec<Vec<u8>>,
    pub status: StatusCode,
    pub status_message: Option<String>,
}

impl GrpcResponse {
    /// Create a successful response with a message
    pub fn ok(message: Vec<u8>) -> Self {
        Self {
            headers: HashMap::new(),
            messages: vec![message],
            status: StatusCode::Ok,
            status_message: None,
        }
    }

    /// Create an error response
    pub fn error(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            headers: HashMap::new(),
            messages: vec![],
            status,
            status_message: Some(message.into()),
        }
    }

    /// Create an unimplemented response
    pub fn unimplemented(method: &str) -> Self {
        Self::error(StatusCode::Unimplemented, format!("Method not implemented: {}", method))
    }
}

/// Parse multiple gRPC frames from response body
///
/// gRPC frame format:
/// - flags (1 byte): 0x00 = data frame, 0x01 = trailer frame
/// - length (4 bytes): big-endian u32
/// - data (N bytes): message payload
///
/// Returns a vector of message payloads (data frames only, excludes trailers)
fn parse_grpc_frames(data: &[u8]) -> Vec<Vec<u8>> {
    let mut messages = Vec::new();
    let mut offset = 0;

    while offset + 5 <= data.len() {
        let flags = data[offset];
        let msg_len = u32::from_be_bytes([
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
            data[offset + 4],
        ]) as usize;

        offset += 5;

        if offset + msg_len > data.len() {
            // Incomplete frame, take what we have
            if flags == 0x00 && offset < data.len() {
                messages.push(data[offset..].to_vec());
            }
            break;
        }

        // Only include data frames (0x00), skip trailer frames (0x01)
        if flags == 0x00 {
            messages.push(data[offset..offset + msg_len].to_vec());
        }

        offset += msg_len;
    }

    messages
}

/// Parse a gRPC-Web request from raw DataChannel data
pub fn parse_request(data: &[u8]) -> Result<GrpcRequest, String> {
    if data.len() < 8 {
        return Err("Request too short".to_string());
    }

    let mut offset = 0;

    // Read path length (big-endian u32)
    let path_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    offset += 4;

    if offset + path_len > data.len() {
        return Err(format!("Path length {} exceeds data length", path_len));
    }

    // Read path
    let path = String::from_utf8(data[offset..offset + path_len].to_vec())
        .map_err(|e| format!("Invalid path UTF-8: {}", e))?;
    offset += path_len;

    if offset + 4 > data.len() {
        return Err("Missing headers length".to_string());
    }

    // Read headers length (big-endian u32)
    let headers_len = u32::from_be_bytes([
        data[offset], data[offset + 1], data[offset + 2], data[offset + 3]
    ]) as usize;
    offset += 4;

    if offset + headers_len > data.len() {
        return Err(format!("Headers length {} exceeds data length", headers_len));
    }

    // Read headers JSON
    let headers_json = String::from_utf8(data[offset..offset + headers_len].to_vec())
        .map_err(|e| format!("Invalid headers UTF-8: {}", e))?;
    offset += headers_len;

    let headers: HashMap<String, String> = serde_json::from_str(&headers_json)
        .map_err(|e| format!("Invalid headers JSON: {}", e))?;

    // Rest is gRPC-Web frames
    let frames_data = &data[offset..];

    // Parse gRPC-Web data frame to extract message
    let message = if frames_data.len() >= 5 {
        let flags = frames_data[0];
        let msg_len = u32::from_be_bytes([
            frames_data[1], frames_data[2], frames_data[3], frames_data[4]
        ]) as usize;

        if flags == 0x00 && frames_data.len() >= 5 + msg_len {
            frames_data[5..5 + msg_len].to_vec()
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    Ok(GrpcRequest {
        path,
        headers,
        message,
    })
}

/// Stream message flags for streaming RPC over DataChannel
pub const STREAM_FLAG_DATA: u8 = 0x00;
pub const STREAM_FLAG_END: u8 = 0x01;

/// Encode a stream message for DataChannel
/// Format: [requestId_len(4)][requestId(N)][flag(1)][data...]
pub fn encode_stream_message(request_id: &str, flag: u8, data: &[u8]) -> Vec<u8> {
    let request_id_bytes = request_id.as_bytes();
    let mut result = Vec::with_capacity(4 + request_id_bytes.len() + 1 + data.len());

    // Write request ID length (big-endian u32)
    result.extend_from_slice(&(request_id_bytes.len() as u32).to_be_bytes());

    // Write request ID
    result.extend_from_slice(request_id_bytes);

    // Write flag
    result.push(flag);

    // Write data
    result.extend_from_slice(data);

    result
}

/// Encode a single gRPC data frame
fn encode_grpc_data_frame(message: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(5 + message.len());
    // flags = 0x00 (data frame)
    result.push(0x00);
    // length (big-endian u32)
    result.extend_from_slice(&(message.len() as u32).to_be_bytes());
    // message data
    result.extend_from_slice(message);
    result
}

/// Encode a trailer frame with status
fn encode_trailer_frame(status: StatusCode, status_message: Option<&str>) -> Vec<u8> {
    let mut trailers = Vec::new();
    trailers.push(format!("grpc-status: {}", status as u32));
    if let Some(msg) = status_message {
        trailers.push(format!("grpc-message: {}", msg));
    }
    let trailer_text = trailers.join("\r\n") + "\r\n";
    let trailer_bytes = trailer_text.as_bytes();

    let mut result = Vec::with_capacity(5 + trailer_bytes.len());
    // flags = 0x01 (trailer frame)
    result.push(0x01);
    // length (big-endian u32)
    result.extend_from_slice(&(trailer_bytes.len() as u32).to_be_bytes());
    // trailer data
    result.extend_from_slice(trailer_bytes);
    result
}

/// Encode a gRPC response to DataChannel format
pub fn encode_response(response: &GrpcResponse) -> Vec<u8> {
    let mut result = Vec::new();

    // Encode headers as JSON
    let headers_json = serde_json::to_string(&response.headers).unwrap_or_else(|_| "{}".to_string());
    let headers_bytes = headers_json.as_bytes();

    // Write headers length (big-endian u32)
    let headers_len = headers_bytes.len() as u32;
    result.extend_from_slice(&headers_len.to_be_bytes());

    // Write headers
    result.extend_from_slice(headers_bytes);

    // Write data frames
    for message in &response.messages {
        // flags = 0x00 (data frame)
        result.push(0x00);
        // length (big-endian u32)
        let msg_len = message.len() as u32;
        result.extend_from_slice(&msg_len.to_be_bytes());
        // message data
        result.extend_from_slice(message);
    }

    // Write trailer frame
    let mut trailers = Vec::new();
    trailers.push(format!("grpc-status: {}", response.status as u32));
    if let Some(ref msg) = response.status_message {
        trailers.push(format!("grpc-message: {}", msg));
    }
    let trailer_text = trailers.join("\r\n") + "\r\n";
    let trailer_bytes = trailer_text.as_bytes();

    // flags = 0x01 (trailer frame)
    result.push(0x01);
    // length (big-endian u32)
    let trailer_len = trailer_bytes.len() as u32;
    result.extend_from_slice(&trailer_len.to_be_bytes());
    // trailer data
    result.extend_from_slice(trailer_bytes);

    result
}

/// Handler trait for gRPC methods
pub trait GrpcHandler: Send + Sync {
    /// Handle a gRPC request and return a response
    fn handle(&self, request: &GrpcRequest) -> GrpcResponse;
}

/// Default handler that routes to registered methods
pub struct GrpcRouter {
    handlers: HashMap<String, Box<dyn Fn(&GrpcRequest) -> GrpcResponse + Send + Sync>>,
}

impl GrpcRouter {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }

    /// Register a handler for a method path
    pub fn register<F>(&mut self, path: &str, handler: F)
    where
        F: Fn(&GrpcRequest) -> GrpcResponse + Send + Sync + 'static,
    {
        self.handlers.insert(path.to_string(), Box::new(handler));
    }

    /// Handle a request
    pub fn handle(&self, request: &GrpcRequest) -> GrpcResponse {
        if let Some(handler) = self.handlers.get(&request.path) {
            handler(request)
        } else {
            GrpcResponse::unimplemented(&request.path)
        }
    }
}

impl Default for GrpcRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Process raw DataChannel data and return response bytes
pub fn process_request(data: &[u8], router: &GrpcRouter) -> Vec<u8> {
    match parse_request(data) {
        Ok(request) => {
            tracing::info!("gRPC request: {} (headers: {:?})", request.path, request.headers);
            let mut response = router.handle(&request);

            // Copy x-request-id from request to response headers
            if let Some(request_id) = request.headers.get("x-request-id") {
                response.headers.insert("x-request-id".to_string(), request_id.clone());
            }

            encode_response(&response)
        }
        Err(e) => {
            tracing::error!("Failed to parse gRPC request: {}", e);
            let response = GrpcResponse::error(StatusCode::Internal, e);
            encode_response(&response)
        }
    }
}

/// Bridge to tonic gRPC services
///
/// This allows routing P2P DataChannel requests to tonic-generated services.
pub struct TonicServiceBridge<S> {
    service: Arc<Mutex<S>>,
}

impl<S> TonicServiceBridge<S>
where
    S: Service<http::Request<BoxBody>, Response = http::Response<BoxBody>> + Send + 'static,
    S::Future: Send,
    S::Error: std::fmt::Debug,
{
    pub fn new(service: S) -> Self {
        Self {
            service: Arc::new(Mutex::new(service)),
        }
    }

    /// Call the tonic service with a gRPC request
    pub async fn call(&self, request: &GrpcRequest) -> GrpcResponse {
        // Build gRPC frame from message
        let mut grpc_body = Vec::new();
        grpc_body.push(0x00); // flags = data frame
        let msg_len = request.message.len() as u32;
        grpc_body.extend_from_slice(&msg_len.to_be_bytes());
        grpc_body.extend_from_slice(&request.message);

        // Build HTTP request
        let uri = format!("http://localhost{}", request.path);
        // Use map_err to convert Infallible to Status for BoxBody compatibility
        let body = BoxBody::new(
            Full::new(Bytes::from(grpc_body))
                .map_err(|_: std::convert::Infallible| Status::internal("body error"))
        );

        let mut http_req = http::Request::builder()
            .method("POST")
            .uri(&uri)
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(body)
            .unwrap();

        // Copy headers from request
        for (key, value) in &request.headers {
            if let Ok(header_value) = http::HeaderValue::from_str(value) {
                if let Ok(header_name) = http::HeaderName::from_bytes(key.as_bytes()) {
                    http_req.headers_mut().insert(header_name, header_value);
                }
            }
        }

        // Call the service
        let mut service = self.service.lock().await;
        match service.call(http_req).await {
            Ok(response) => self.parse_http_response(response).await,
            Err(e) => {
                tracing::error!("Service call failed: {:?}", e);
                GrpcResponse::error(StatusCode::Internal, format!("Service call failed: {:?}", e))
            }
        }
    }

    async fn parse_http_response(&self, response: http::Response<BoxBody>) -> GrpcResponse {
        let (parts, body) = response.into_parts();

        // Extract response headers
        let mut headers = HashMap::new();
        for (key, value) in parts.headers.iter() {
            if let Ok(v) = value.to_str() {
                headers.insert(key.to_string(), v.to_string());
            }
        }

        // Read body
        let body_bytes = match body.collect().await {
            Ok(collected) => collected.to_bytes().to_vec(),
            Err(e) => {
                tracing::error!("Failed to read response body: {:?}", e);
                return GrpcResponse::error(StatusCode::Internal, "Failed to read response body");
            }
        };

        // Parse gRPC status from trailers or headers
        let status = headers
            .get("grpc-status")
            .and_then(|s| s.parse::<u32>().ok())
            .map(|code| match code {
                0 => StatusCode::Ok,
                1 => StatusCode::Cancelled,
                2 => StatusCode::Unknown,
                3 => StatusCode::InvalidArgument,
                4 => StatusCode::DeadlineExceeded,
                5 => StatusCode::NotFound,
                6 => StatusCode::AlreadyExists,
                7 => StatusCode::PermissionDenied,
                8 => StatusCode::ResourceExhausted,
                9 => StatusCode::FailedPrecondition,
                10 => StatusCode::Aborted,
                11 => StatusCode::OutOfRange,
                12 => StatusCode::Unimplemented,
                13 => StatusCode::Internal,
                14 => StatusCode::Unavailable,
                15 => StatusCode::DataLoss,
                16 => StatusCode::Unauthenticated,
                _ => StatusCode::Unknown,
            })
            .unwrap_or(StatusCode::Ok);

        let status_message = headers.get("grpc-message").cloned();

        // Extract messages from gRPC frames (supports multiple frames for streaming)
        let messages = parse_grpc_frames(&body_bytes);

        GrpcResponse {
            headers,
            messages,
            status,
            status_message,
        }
    }
}

impl<S> Clone for TonicServiceBridge<S> {
    fn clone(&self) -> Self {
        Self {
            service: self.service.clone(),
        }
    }
}

/// Response type for gRPC processing
pub enum GrpcProcessResult {
    /// Unary response - single response bytes
    Unary(Vec<u8>),
    /// Streaming response - multiple stream messages to send individually
    Streaming(Vec<Vec<u8>>),
}

/// Process raw DataChannel data using tonic service bridge and return response
pub async fn process_request_with_service<S>(data: &[u8], bridge: &TonicServiceBridge<S>) -> GrpcProcessResult
where
    S: Service<http::Request<BoxBody>, Response = http::Response<BoxBody>> + Send + 'static,
    S::Future: Send,
    S::Error: std::fmt::Debug,
{
    match parse_request(data) {
        Ok(request) => {
            tracing::info!(
                "gRPC request: {} (headers: {:?})",
                request.path,
                request.headers
            );

            // Check if this is a streaming request
            let is_streaming = request.path.contains("StreamDownload");
            let request_id = request.headers.get("x-request-id").cloned();

            let mut response = bridge.call(&request).await;

            // Copy x-request-id from request to response headers
            if let Some(ref req_id) = request_id {
                response
                    .headers
                    .insert("x-request-id".to_string(), req_id.clone());
            }

            if is_streaming {
                // For streaming, return individual stream messages
                if let Some(req_id) = request_id {
                    if req_id.starts_with("stream-") {
                        return encode_streaming_response(&req_id, &response);
                    }
                }
                // Fallback to unary if no stream- prefix
                tracing::warn!("StreamDownload request without stream- prefix, falling back to unary");
            }

            GrpcProcessResult::Unary(encode_response(&response))
        }
        Err(e) => {
            tracing::error!("Failed to parse gRPC request: {}", e);
            let response = GrpcResponse::error(StatusCode::Internal, e);
            GrpcProcessResult::Unary(encode_response(&response))
        }
    }
}

/// Encode a streaming response as multiple stream messages
fn encode_streaming_response(request_id: &str, response: &GrpcResponse) -> GrpcProcessResult {
    let mut messages = Vec::new();

    // Send each message as a DATA stream message
    for msg in &response.messages {
        let grpc_frame = encode_grpc_data_frame(msg);
        let stream_msg = encode_stream_message(request_id, STREAM_FLAG_DATA, &grpc_frame);
        tracing::debug!("Encoded stream DATA message ({} bytes)", stream_msg.len());
        messages.push(stream_msg);
    }

    // Send END message with trailer
    let trailer_frame = encode_trailer_frame(response.status, response.status_message.as_deref());
    let end_msg = encode_stream_message(request_id, STREAM_FLAG_END, &trailer_frame);
    tracing::debug!("Encoded stream END message ({} bytes)", end_msg.len());
    messages.push(end_msg);

    tracing::info!(
        "Encoded streaming response: {} messages (status: {:?})",
        messages.len(),
        response.status
    );

    GrpcProcessResult::Streaming(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_request() {
        // Build a test request
        let path = "/scraper.ETCScraper/Health";
        let headers = r#"{"x-request-id":"test-123"}"#;
        let message = vec![0x0a, 0x05, 0x68, 0x65, 0x6c, 0x6c, 0x6f]; // protobuf message

        let mut data = Vec::new();
        // path length
        data.extend_from_slice(&(path.len() as u32).to_be_bytes());
        // path
        data.extend_from_slice(path.as_bytes());
        // headers length
        data.extend_from_slice(&(headers.len() as u32).to_be_bytes());
        // headers
        data.extend_from_slice(headers.as_bytes());
        // gRPC frame: flags(1) + length(4) + data
        data.push(0x00); // data frame
        data.extend_from_slice(&(message.len() as u32).to_be_bytes());
        data.extend_from_slice(&message);

        let request = parse_request(&data).unwrap();
        assert_eq!(request.path, "/scraper.ETCScraper/Health");
        assert_eq!(request.headers.get("x-request-id"), Some(&"test-123".to_string()));
        assert_eq!(request.message, message);
    }

    #[test]
    fn test_encode_response() {
        let response = GrpcResponse::ok(vec![0x0a, 0x02, 0x6f, 0x6b]);
        let encoded = encode_response(&response);

        // Should have: headers_len(4) + headers + data_frame + trailer_frame
        assert!(encoded.len() > 10);

        // First 4 bytes are headers length
        let headers_len = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]) as usize;
        assert!(headers_len < encoded.len());
    }

    #[test]
    fn test_router() {
        let mut router = GrpcRouter::new();
        router.register("/test.Service/Method", |_req| {
            GrpcResponse::ok(vec![0x01, 0x02, 0x03])
        });

        let request = GrpcRequest {
            path: "/test.Service/Method".to_string(),
            headers: HashMap::new(),
            message: vec![],
        };

        let response = router.handle(&request);
        assert_eq!(response.status, StatusCode::Ok);
        assert_eq!(response.messages.len(), 1);
    }

    #[test]
    fn test_router_unimplemented() {
        let router = GrpcRouter::new();
        let request = GrpcRequest {
            path: "/unknown.Service/Method".to_string(),
            headers: HashMap::new(),
            message: vec![],
        };

        let response = router.handle(&request);
        assert_eq!(response.status, StatusCode::Unimplemented);
    }

    #[test]
    fn test_parse_grpc_frames_single() {
        // Single data frame: [0x00][len=4][data]
        let mut data = Vec::new();
        data.push(0x00); // data frame
        data.extend_from_slice(&4u32.to_be_bytes());
        data.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);

        let messages = parse_grpc_frames(&data);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], vec![0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn test_parse_grpc_frames_multiple() {
        // Multiple data frames (streaming response)
        let mut data = Vec::new();

        // Frame 1
        data.push(0x00);
        data.extend_from_slice(&3u32.to_be_bytes());
        data.extend_from_slice(&[0x0a, 0x0b, 0x0c]);

        // Frame 2
        data.push(0x00);
        data.extend_from_slice(&2u32.to_be_bytes());
        data.extend_from_slice(&[0x0d, 0x0e]);

        // Frame 3
        data.push(0x00);
        data.extend_from_slice(&4u32.to_be_bytes());
        data.extend_from_slice(&[0x0f, 0x10, 0x11, 0x12]);

        let messages = parse_grpc_frames(&data);
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0], vec![0x0a, 0x0b, 0x0c]);
        assert_eq!(messages[1], vec![0x0d, 0x0e]);
        assert_eq!(messages[2], vec![0x0f, 0x10, 0x11, 0x12]);
    }

    #[test]
    fn test_parse_grpc_frames_with_trailer() {
        // Data frame followed by trailer frame (should skip trailer)
        let mut data = Vec::new();

        // Data frame
        data.push(0x00);
        data.extend_from_slice(&3u32.to_be_bytes());
        data.extend_from_slice(&[0x01, 0x02, 0x03]);

        // Trailer frame (should be ignored)
        data.push(0x01);
        let trailer = b"grpc-status: 0\r\n";
        data.extend_from_slice(&(trailer.len() as u32).to_be_bytes());
        data.extend_from_slice(trailer);

        let messages = parse_grpc_frames(&data);
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], vec![0x01, 0x02, 0x03]);
    }

    #[test]
    fn test_parse_grpc_frames_empty() {
        let data: Vec<u8> = Vec::new();
        let messages = parse_grpc_frames(&data);
        assert!(messages.is_empty());
    }

    #[test]
    fn test_encode_stream_message() {
        let request_id = "stream-1735312345678-1";
        let data = vec![0x01, 0x02, 0x03, 0x04];

        let encoded = encode_stream_message(request_id, STREAM_FLAG_DATA, &data);

        // Verify format: [requestId_len(4)][requestId(N)][flag(1)][data...]
        let request_id_len = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]) as usize;
        assert_eq!(request_id_len, request_id.len());

        let decoded_request_id = String::from_utf8(encoded[4..4 + request_id_len].to_vec()).unwrap();
        assert_eq!(decoded_request_id, request_id);

        let flag = encoded[4 + request_id_len];
        assert_eq!(flag, STREAM_FLAG_DATA);

        let decoded_data = &encoded[4 + request_id_len + 1..];
        assert_eq!(decoded_data, data.as_slice());
    }

    #[test]
    fn test_encode_stream_message_end() {
        let request_id = "stream-1735312345678-2";
        let trailer_data = b"grpc-status: 0\r\n";

        let encoded = encode_stream_message(request_id, STREAM_FLAG_END, trailer_data);

        let request_id_len = u32::from_be_bytes([encoded[0], encoded[1], encoded[2], encoded[3]]) as usize;
        let flag = encoded[4 + request_id_len];
        assert_eq!(flag, STREAM_FLAG_END);
    }

    #[test]
    fn test_encode_streaming_response() {
        let response = GrpcResponse {
            headers: HashMap::new(),
            messages: vec![
                vec![0x0a, 0x01, 0x01],  // Message 1
                vec![0x0a, 0x02, 0x02],  // Message 2
            ],
            status: StatusCode::Ok,
            status_message: None,
        };

        let result = encode_streaming_response("stream-test-123", &response);

        if let GrpcProcessResult::Streaming(messages) = result {
            // Should have 2 DATA messages + 1 END message
            assert_eq!(messages.len(), 3);

            // Verify all messages have stream- prefix format
            for msg in &messages {
                let request_id_len = u32::from_be_bytes([msg[0], msg[1], msg[2], msg[3]]) as usize;
                let request_id = String::from_utf8(msg[4..4 + request_id_len].to_vec()).unwrap();
                assert!(request_id.starts_with("stream-"));
            }

            // Last message should be END flag
            let last_msg = &messages[2];
            let request_id_len = u32::from_be_bytes([last_msg[0], last_msg[1], last_msg[2], last_msg[3]]) as usize;
            let flag = last_msg[4 + request_id_len];
            assert_eq!(flag, STREAM_FLAG_END);
        } else {
            panic!("Expected Streaming result");
        }
    }
}
