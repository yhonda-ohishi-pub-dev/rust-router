//! Authentication and authorization library for microservices.
//!
//! This crate provides JWT-based authentication utilities.

mod jwt;
mod claims;

pub use jwt::{encode_token, decode_token, JwtConfig};
pub use claims::{Claims, Role};
