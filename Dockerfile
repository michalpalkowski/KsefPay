FROM rust:1.92-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY ksef-core/ ksef-core/
COPY ksef-server/ ksef-server/

RUN cargo build --release -p ksef-server

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ksef-server /usr/local/bin/ksef-server
COPY ksef-server/templates/ /app/templates/
COPY ksef-server/assets/ /app/assets/

WORKDIR /app
EXPOSE 3000

CMD ["ksef-server"]
