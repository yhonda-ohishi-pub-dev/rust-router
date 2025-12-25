# Build stage
FROM rust:1.75-slim as builder

WORKDIR /app

# Install protobuf compiler
RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

# Copy shared-lib first (for caching)
COPY shared-lib /app/shared-lib

# Copy timecard-service
COPY timecard-service /app/timecard-service

# Copy gateway
COPY gateway /app/gateway

WORKDIR /app/gateway

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/gateway/target/release/gateway /app/gateway

# Expose gRPC port
EXPOSE 50051

# Run the gateway
CMD ["/app/gateway"]
