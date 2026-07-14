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
build-core:
    wasm-pack build core --target web

# Native core tests (the correctness oracle as the model is ported into Rust).
test-core:
    cargo test -p nib-core

# Install frontend deps. Builds the core first so the link: dep resolves.
install: build-core
    cd frontend && node .yarn/releases/yarn-*.cjs install

# Dev server (:5173).
dev: build-core
    cd frontend && node .yarn/releases/yarn-*.cjs dev

# Production build → frontend/dist.
build: build-core
    cd frontend && node .yarn/releases/yarn-*.cjs build

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
