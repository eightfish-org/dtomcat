# Use an official Rust image as the base
FROM rust:1.81.0-bookworm

# Set the working directory in the container
WORKDIR /usr/src/myapp

# Copy the Cargo.toml and Cargo.lock files (if available)
COPY Cargo.toml ./
COPY Cargo.lock ./

# Copy your source code
COPY src ./src

# Build your application
RUN cargo build --release

# The binary will be in /usr/src/myapp/target/release/
