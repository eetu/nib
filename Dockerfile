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
WORKDIR /app
# Copied before the tools so the wasm-bindgen CLI version can be read out of the lockfile below.
COPY Cargo.toml Cargo.lock ./
# Both tools follow `uname -m`, because this stage runs on the *build* platform — x86_64 in CI, but
# aarch64 on an Apple Silicon machine, where a hardcoded arch makes the whole image unbuildable.
# wasm-pack would fetch wasm-bindgen itself, except it only knows how to fetch an x86_64 one; taking
# it from the release here instead is what makes the stage arch-agnostic. Its version comes from
# Cargo.lock, so the CLI can't drift from the crate the core is compiled against (wasm-pack rejects
# a mismatch), and a `cargo update` needs no edit here.
#
# Archive members are named explicitly rather than globbed: alpine's tar is BusyBox, which has no
# --wildcards. Each archive's directory name is deterministic from its version.
RUN apk add --no-cache curl musl-dev \
    && ARCH="$(uname -m)" \
    && WBG_VERSION="$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | grep '^version' | head -1 | cut -d'"' -f2)" \
    && ASSET="wasm-pack-${WASM_PACK_VERSION}-${ARCH}-unknown-linux-musl" \
    && curl -sSfL "https://github.com/rustwasm/wasm-pack/releases/download/${WASM_PACK_VERSION}/${ASSET}.tar.gz" \
    | tar -xz --strip-components=1 -C /usr/local/bin "${ASSET}/wasm-pack" \
    && WBG_ASSET="wasm-bindgen-${WBG_VERSION}-${ARCH}-unknown-linux-musl" \
    && curl -sSfL "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${WBG_VERSION}/${WBG_ASSET}.tar.gz" \
    | tar -xz --strip-components=1 -C /usr/local/bin "${WBG_ASSET}/wasm-bindgen" \
    && wasm-pack --version \
    && wasm-bindgen --version \
    && rustup target add wasm32-unknown-unknown
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

# --- Stage 5: Fonts for the runtime image ---
# `scratch` ships none, and without a face the server can neither outline a label (MCP
# `outline_text`) nor draw one in a preview (`render_document`) — both would degrade to "no font
# found" on the deployed instance while working fine on a developer's machine. DejaVu covers wide
# Unicode; Liberation is metric-compatible with Arial/Times/Courier, the names real SVGs ask for,
# so an outlined label keeps the advances (and therefore the centring) it was authored with.
# Font files are architecture-independent, hence $BUILDPLATFORM.
#
# Curated, not wholesale: the two packages together are 14.8 MB, most of it faces nobody asks for
# (Condensed cuts, ExtraLight, a TeX math face, X11 encodings). Keeping the four Liberation
# families and DejaVu's Sans styles plus one Serif/Mono covers Latin/Greek/Cyrillic in both roman
# and italic at roughly half the weight.
FROM --platform=$BUILDPLATFORM alpine:3 AS fonts
RUN apk add --no-cache font-dejavu font-liberation \
    && mkdir -p /fonts \
    && cp /usr/share/fonts/liberation/*.ttf /fonts/ \
    && cd /usr/share/fonts/dejavu \
    && cp DejaVuSans.ttf DejaVuSans-Bold.ttf DejaVuSans-Oblique.ttf DejaVuSans-BoldOblique.ttf \
          DejaVuSerif.ttf DejaVuSansMono.ttf /fonts/ \
    && du -sh /fonts

# --- Stage 6: Runtime (scratch + static musl binary + dist + CA certs + fonts) ---
FROM scratch AS runner
WORKDIR /app
LABEL org.opencontainers.image.description="nib — a direct-manipulation SVG path editor with an MCP surface"
LABEL org.opencontainers.image.source="https://github.com/eetu/nib"

# CA roots are needed at runtime: the OIDC discovery + token exchange talk to kanidm over HTTPS.
COPY --from=backend-build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/
COPY --from=backend-build /nib-backend ./nib-backend
COPY --from=frontend-build /app/frontend/dist ./dist
# fontdb scans /usr/share/fonts recursively, so the faces land where it already looks. For brand
# faces, mount a directory and point NIB_FONT_DIR at it — no rebuild needed.
COPY --from=fonts /fonts /usr/share/fonts/nib

ENV NIB_DIST=./dist
ENV NIB_DB=sqlite:/data/nib.db
ENV NIB_PORT=3009

# The process binds 127.0.0.1; the quadlet runs with Network=host, so Traefik on the same host is
# the only way in (the mcp-chat pattern).
USER 1000
EXPOSE 3009
CMD ["./nib-backend"]
