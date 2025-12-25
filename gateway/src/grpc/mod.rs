//! gRPC module
//!
//! Contains the generated gRPC code and service implementations.

pub mod gateway_server {
    tonic::include_proto!("gateway");
}

pub mod gateway_service;
