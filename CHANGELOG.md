# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-02

The first self-hostable release: nib runs on the Pi as a container, signs users in with Kanidm,
and exposes its editing engine to an LLM over MCP.

### Added

- **Real users via OIDC.** nib is its own OIDC client (`openidconnect` + PKCE, the sibling apps'
  `oidc.rs` shape). `/auth/login` → issuer → `/auth/callback` sets a signed `nib_session` cookie;
  identity is the issuer's `sub`, so a rename or a changed address follows the same account.
  Discovery is lazy and self-healing, which is what makes the two-deploy Kanidm bootstrap safe.
- **A personal, rotatable bearer token per user**, minted at first login. Settings shows it with
  copy + rotate; it's the credential an MCP client presents. Reading or rotating it requires the
  browser session — a leaked token can't read itself back or mint its replacement.
- **`/mcp` bypasses SSO** and authenticates with the bearer alone.
- **Projects can be renamed and deleted** from the projects panel — double-click a row to rename
  in place, right-click for rename/delete (mirroring the Inspector's LAYERS rows). Backed by
  `PATCH`/`DELETE /api/projects/{id}`, both ownership-scoped in SQL; deleting drops the project's
  in-memory session so nothing keeps serving a document whose row is gone. Also on MCP as
  `rename_project` / `delete_project` — the latter requires the project's current name alongside
  its id, so a hallucinated or stale id fails loudly instead of destroying the wrong document.
- **Container packaging**: a 5-stage Dockerfile (the family's `xx` cross-compile → `scratch`, plus
  a wasm-pack stage for the core the SPA links) and an arm64 image published to
  `ghcr.io/eetu/nib`.
- Backend `clippy`/`rustfmt`/test job in CI — the backend was never compiled there before.

### Fixed

- **Cross-tenant session bypass.** `session::open` checked project ownership only on the cold
  path, so once a project was resident in memory any authenticated user could attach to it — read
  and write — via the WebSocket, MCP `open_project`, or any MCP tool. The check now precedes the
  cache lookup. Invisible with one seeded user; a breach the moment real users exist.
- Project sessions are evicted when idle instead of staying resident for the process lifetime.
- Permissive CORS is applied only under dev auth, not in production alongside a session cookie.
- The WebSocket authenticates with the session cookie same-origin (`?token=` remains for
  cross-origin and non-browser clients), keeping the secret out of proxy logs, and closes with a
  real code + reason instead of a bare frame.
- Auth resolves through one primitive with consistent failures, rather than three code paths with
  three different rejection styles.
- Internal errors no longer leak sqlx error text to the client.

### Changed

- The seeded `developer` user and its `nib-dev-token` are dev-only (`NIB_DEV_AUTH`); production
  creates no default account.

[Unreleased]: https://github.com/eetu/nib/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/eetu/nib/releases/tag/v0.1.0
