# syntax=docker/dockerfile:1
#
# The sibling 4-stage shape (xx cross-compile → scratch), plus one stage no other app in the
# family needs: nib's `core/` crate compiles to WASM for the browser, and `frontend/` consumes
# that output as a `link:../core/pkg` dependency — so wasm-pack has to run *before* yarn install.

# --- Cross-compilation helper ---
FROM --platform=$BUILDPLATFORM tonistiigi/xx AS xx

# --- Stage 1: Build the WASM core (native; .wasm is platform-independent) ---
# Pinned to $BUILDPLATFORM deliberately: this output ships to the browser, so it must never be
# cross-compiled for the container's target. wasm-pack comes from its prebuilt musl release —
# `cargo install wasm-pack` would rebuild it from source on every cache miss.
FROM --platform=$BUILDPLATFORM rust:1-alpine AS wasm-build
ARG WASM_PACK_VERSION=v0.15.0
RUN apk add --no-cache curl musl-dev \
    && curl -sSfL "https://github.com/rustwasm/wasm-pack/releases/download/${WASM_PACK_VERSION}/wasm-pack-${WASM_PACK_VERSION}-x86_64-unknown-linux-musl.tar.gz" \
    | tar -xz --strip-components=1 -C /usr/local/bin --wildcards '*/wasm-pack' \
    && rustup target add wasm32-unknown-unknown
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY core core
# The backend member must exist for the workspace to load, but isn't built here.
COPY backend/Cargo.toml backend/Cargo.toml
RUN mkdir -p backend/src && printf 'fn main() {}\n' > backend/src/main.rs \
    && wasm-pack build core --target web

# --- Stage 2: Build the SPA ---
# Vendored yarn (no corepack). node version matches frontend/.node-version.
FROM --platform=$BUILDPLATFORM node:26-alpine AS frontend-build
# `link:../core/pkg` resolves relative to /app/frontend, so the core lands beside it.
COPY --from=wasm-build /app/core/pkg /app/core/pkg
WORKDIR /app/frontend
COPY frontend/package.json frontend/yarn.lock frontend/.yarnrc.yml ./
COPY frontend/.yarn/releases ./.yarn/releases
RUN node .yarn/releases/yarn-*.cjs install --immutable --network-timeout 1000000
COPY frontend/ .
# Connected mode on, and BASE_PATH deliberately unset: the backend serves /api, /ws and /mcp at
# the root, so a Pages-style base would send the SPA's calls to the index.html fallback.
ENV VITE_NIB_BACKEND=1
RUN node .yarn/releases/yarn-*.cjs build

# --- Stage 3: Warm the cross-compiled dependency cache with stub sources ---
FROM --platform=$BUILDPLATFORM rust:1-alpine AS workspace-deps
COPY --from=xx / /
RUN apk add --no-cache clang lld musl-dev curl
ARG TARGETPLATFORM
# gcc/musl-dev for the target: sqlx pulls libsqlite3-sys with `bundled`, the only C in the tree.
RUN xx-apk add --no-cache musl-dev gcc
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY core/Cargo.toml core/Cargo.toml
COPY backend/Cargo.toml backend/Cargo.toml
RUN mkdir -p core/src backend/src \
    && : > core/src/lib.rs \
    && printf 'fn main() {}\n' > backend/src/main.rs \
    && xx-cargo build --release -p nib-backend

# --- Stage 4: Build the backend against real sources ---
FROM workspace-deps AS backend-build
ARG TARGETPLATFORM
COPY core/src ./core/src
COPY backend/src ./backend/src
# `sqlx::migrate!` embeds these into the binary at compile time.
COPY backend/migrations ./backend/migrations
# `touch` so cargo notices the stub→real swap.
RUN touch core/src/lib.rs backend/src/main.rs \
    && xx-cargo build --release -p nib-backend \
    && cp target/*/release/nib-backend /nib-backend

# --- Stage 5: Runtime (scratch + static musl binary + dist + CA certs) ---
FROM scratch AS runner
WORKDIR /app
LABEL org.opencontainers.image.description="nib — a direct-manipulation SVG path editor with an MCP surface"
LABEL org.opencontainers.image.source="https://github.com/eetu/nib"

# CA roots are needed at runtime: the OIDC discovery + token exchange talk to kanidm over HTTPS.
COPY --from=backend-build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=backend-build /nib-backend ./nib-backend
COPY --from=frontend-build /app/frontend/dist ./dist

ENV NIB_DIST=./dist
ENV NIB_DB=sqlite:/data/nib.db
ENV NIB_PORT=3009

# The process binds 127.0.0.1; the quadlet runs with Network=host, so Traefik on the same host is
# the only way in (the mcp-chat pattern). No font packages: resvg never loads system fonts.
USER 1000
EXPOSE 3009
CMD ["./nib-backend"]
