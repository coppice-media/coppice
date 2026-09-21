FROM rust:1.97-bookworm AS builder

WORKDIR /build
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /build/target/release/mam-gateway /usr/local/bin/mam-gateway
EXPOSE 8088
ENTRYPOINT ["/usr/local/bin/mam-gateway"]
