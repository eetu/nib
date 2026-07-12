# nib task runner. `just` with no args lists recipes.
# Frontend-only for now (a fully client-side SVG editor). A rust-axum backend +
# raspi deploy drop in later without moving the frontend — see CLAUDE.md.
#
# Yarn = the repo-vendored release (pinned by yarnPath in frontend/.yarnrc.yml),
# run via node. No global yarn / corepack needed.

default:
    @just --list

# Install frontend deps.
install:
    cd frontend && node .yarn/releases/yarn-*.cjs install

# Dev server (:5173).
dev:
    cd frontend && node .yarn/releases/yarn-*.cjs dev

# Production build → frontend/dist.
build:
    cd frontend && node .yarn/releases/yarn-*.cjs build

# Typecheck + lint + format check.
validate:
    cd frontend && node .yarn/releases/yarn-*.cjs validate

# Unit tests.
test:
    cd frontend && node .yarn/releases/yarn-*.cjs test

# Autofix lint + format.
fix:
    cd frontend && node .yarn/releases/yarn-*.cjs lint:fix && node .yarn/releases/yarn-*.cjs format:fix
