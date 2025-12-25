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
}
