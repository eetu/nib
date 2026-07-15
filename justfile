# nib task runner. `just` with no args lists recipes.
#
# Two build inputs: the Rust `core/` crate (nib-core) compiled to WASM, and the
# SvelteKit frontend that consumes it via a `link:` dep (frontend imports `nib-core`,
# resolved to ../core/pkg). The frontend recipes build the core first. See CLAUDE.md
# and the roadmap — the model/geometry/op engine lives in the Rust core.
#
# Yarn = the repo-vendored release (pinned by yarnPath in frontend/.yarnrc.yml),
# run via node. No global yarn / corepack needed.

default:
    @just --list

# Build the Rust core → WASM (core/pkg), consumed by the frontend link: dep.
# (dev/install use this directly — unoptimized + fast; `build` adds opt-core.)
build-core:
    wasm-pack build core --target web

# Size-optimize the core .wasm with binaryen. Skipped if wasm-opt is absent (it's a
# size nicety, not required). Modern rustc emits post-MVP wasm features, so
# --all-features tells wasm-opt to accept them; wasm-pack's bundled wasm-opt is too
# old, hence a standalone binaryen (`brew install binaryen`, or apt in CI).
opt-core: build-core
    if command -v wasm-opt >/dev/null 2>&1; then wasm-opt -Oz --all-features core/pkg/nib_core_bg.wasm -o core/pkg/nib_core_bg.wasm; else echo "wasm-opt (binaryen) not found — shipping the unoptimized core .wasm"; fi

# Native core tests (the correctness oracle as the model is ported into Rust).
test-core:
    cargo test -p nib-core

# Backend tests (native — the axum server links nib-core directly).
test-backend:
    cargo test -p nib-backend

# Install frontend deps. Builds the core first so the link: dep resolves.
install: build-core
    cd frontend && node .yarn/releases/yarn-*.cjs install

# Dev server (:5173).
dev: build-core
    cd frontend && node .yarn/releases/yarn-*.cjs dev

# Production build → frontend/dist (size-optimized core .wasm).
build: opt-core
    cd frontend && node .yarn/releases/yarn-*.cjs build

# Phase C backend (:4321): build the SPA, then serve it + the .svg documents API. Links
# nib-core natively — the same engine the browser drives via WASM. NIB_PORT/NIB_DOCS override.
backend: build
    NIB_DIST=frontend/dist NIB_DOCS=docs cargo run -p nib-backend

# Typecheck + lint + format check.
validate:
    cd frontend && node .yarn/releases/yarn-*.cjs validate

# Unit tests (frontend + core).
test: test-core
    cd frontend && node .yarn/releases/yarn-*.cjs test

# Browser smoke tests (Playwright) — drives the real app against a production build.
test-e2e: build
    cd frontend && node .yarn/releases/yarn-*.cjs test:e2e

# Autofix lint + format.
fix:
    cd frontend && node .yarn/releases/yarn-*.cjs lint:fix && node .yarn/releases/yarn-*.cjs format:fix
