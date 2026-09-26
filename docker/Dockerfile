# ------------------------------------------------------------------------------
# PDFium Stage
# ------------------------------------------------------------------------------

FROM debian:bookworm-slim AS pdfium

ARG TARGETARCH

# Update and install curl and tar
RUN --mount=type=cache,target=/var/cache/apt --mount=type=cache,target=/var/lib/apt \
    apt-get update && apt-get install -y curl tar

# Download and extract the PDFium binaries
RUN set -ex; \
    mkdir -p pdfium; \
    if [ "$TARGETARCH" = "amd64" ]; then \
    curl -sLo pdfium.tgz https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/7763/pdfium-linux-x64.tgz; \
    elif [ "$TARGETARCH" = "arm64" ]; then \
    curl -sLo pdfium.tgz https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/7763/pdfium-linux-arm64.tgz; \
    fi; \
    tar -xzvf pdfium.tgz -C ./pdfium; \
    rm pdfium.tgz

# We need to validate the sha of the libpdfium.so file:
# amd64 -> 727145701ce5bb0344c389071b7aa8f1e15ae81a
# arm64 -> c23942df3dc35f95a7bfa75fc52a480e77b5b0ca
RUN set -ex; \
    if [ "$TARGETARCH" = "amd64" ]; then \
    echo "727145701ce5bb0344c389071b7aa8f1e15ae81a /pdfium/lib/libpdfium.so" | sha1sum -c -; \
    elif [ "$TARGETARCH" = "arm64" ]; then \
    echo "c23942df3dc35f95a7bfa75fc52a480e77b5b0ca /pdfium/lib/libpdfium.so" | sha1sum -c -; \
    else \
    echo "Unsupported architecture: $TARGETARCH"; \
    exit 1; \
    fi

# ------------------------------------------------------------------------------
# Frontend Build Stage
# ------------------------------------------------------------------------------

FROM oven/bun:1.4.1-alpine AS frontend
ARG TARGETARCH

WORKDIR /app

COPY . .

# Git is required by dependencies sourced directly from repositories.
RUN apk add --no-cache git

RUN --mount=type=cache,target=/root/.bun/install/cache,id=bun-${TARGETARCH} \
    bun install --frozen-lockfile && \
    bun run home build && \
    bun run editor build && \
    test -s ./home/build/index.html && \
    test -s ./editor/build/index.html

# ------------------------------------------------------------------------------
# Shared Rust Builder Base
# ------------------------------------------------------------------------------

FROM rust:1.97-bookworm AS rust-builder-base

ARG GIT_REV
ARG BUILD_CHANNEL
ARG TARGETARCH

ENV GIT_REV=${GIT_REV}
ENV BUILD_CHANNEL=${BUILD_CHANNEL}

RUN --mount=type=cache,target=/var/cache/apt --mount=type=cache,target=/var/lib/apt \
    apt-get update && apt-get install -y \
    build-essential \
    cmake \
    wget \
    git \
    libssl-dev \
    pkg-config \
    curl \
    libsqlite3-dev;

WORKDIR /app

COPY . .

# ------------------------------------------------------------------------------
# Full Rust Build Stage
# ------------------------------------------------------------------------------

FROM rust-builder-base AS full-builder

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=${TARGETARCH} \
    --mount=type=cache,target=/app/target,id=${TARGETARCH} \
    ./docker/build_server.sh && \
    cp ./target/release/stump_server ./stump_server

# ------------------------------------------------------------------------------
# Headless Rust Build Stage
# ------------------------------------------------------------------------------

FROM rust-builder-base AS headless-builder

RUN --mount=type=cache,target=/usr/local/cargo/registry,id=${TARGETARCH} \
    --mount=type=cache,target=/app/target,id=${TARGETARCH} \
    ./docker/build_server.sh --no-default-features --features headless && \
    cp ./target/release/stump_server ./stump_server

# ------------------------------------------------------------------------------
# Shared Runtime Base
# ------------------------------------------------------------------------------

FROM gcr.io/distroless/cc-debian12:debug AS runtime-base

ARG PORT=10801

LABEL org.opencontainers.image.title="Coppice" \
    org.opencontainers.image.description="A free and open source comics, manga and digital book server with OPDS support" \
    org.opencontainers.image.source="https://github.com/stumpapp/stump" \
    org.opencontainers.image.url="https://stumpapp.dev" \
    org.opencontainers.image.documentation="https://stumpapp.dev/docs" \
    org.opencontainers.image.licenses="MIT" \
    org.opencontainers.image.vendor="stumpapp"

RUN [ "/busybox/ln", "-s", "/busybox/sh", "/bin/sh" ]
RUN ln -s /busybox/env /usr/bin/env

# TODO(distroless): Ensure locales aren't messed up
# RUN apt-get update && apt-get install -y locales-all && rm -rf /var/lib/apt/lists/*; \
#     mkdir -p config && mkdir -p data && mkdir -p app

COPY --from=pdfium /pdfium /opt/pdfium
COPY --chmod=755 docker/entrypoint.sh /entrypoint.sh

# Default Stump environment variables
ENV STUMP_CONFIG_DIR=/config \
    STUMP_PROFILE=release \
    STUMP_PORT=${PORT} \
    STUMP_IN_DOCKER=true \
    PDFIUM_PATH=/opt/pdfium/lib/libpdfium.so \
    API_VERSION=v1

WORKDIR /app

EXPOSE ${PORT}

ENTRYPOINT ["/entrypoint.sh"]

# ------------------------------------------------------------------------------
# Headless Final Stage
# ------------------------------------------------------------------------------

FROM runtime-base AS headless

COPY --from=headless-builder /app/stump_server /app/stump

ENV STUMP_ENABLE_WEBUI=false \
    STUMP_RUNTIME_WORKER_THREADS=2 \
    STUMP_RUNTIME_MAX_BLOCKING_THREADS=8 \
    STUMP_DB_MAX_CONNECTIONS=2 \
    STUMP_SQLITE_STATEMENT_CACHE_CAPACITY=32

# ------------------------------------------------------------------------------
# Full Final Stage (default)
# ------------------------------------------------------------------------------

FROM runtime-base AS full

COPY --from=full-builder /app/stump_server /app/stump
COPY --from=frontend /app/home/build /app/home
COPY --from=frontend /app/editor/build /app/editor

ENV STUMP_HOME_APP_DIR=/app/home \
    INGEST_EDITOR_DIR=/app/editor
