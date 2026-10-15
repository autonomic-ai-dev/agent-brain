# Apple Container / OCI build for agent-brain compile + extras tests
FROM rust:1.88-bookworm

RUN apt-get update && apt-get install -y --no-install-recommends \
    protobuf-compiler libssl-dev pkg-config cmake clang \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /work
# Cache deps: copy manifests first when possible; full tree for simplicity
COPY . .

# Default: slim serve binary (no extras). Override with --build-arg FEATURES=extras
ARG FEATURES=
RUN cargo check -p agent-brain --features "${FEATURES}"
