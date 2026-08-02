-- Real user identity: a user is now an OIDC subject (kanidm in production), not the single seeded
-- `developer` row. `sub` is the issuer's stable subject claim and the identity we key on; `email`
-- is the human-facing label. `token` stays the per-user bearer an MCP client presents — it's now
-- minted per user at first login and rotatable from the UI, rather than shared.
--
-- The unique index on `sub` is plain, not partial: SQLite treats NULLs as distinct in a UNIQUE
-- index, so pre-OIDC rows (which carry no subject) coexist happily and `on conflict(sub)` still
-- works as the upsert target for real logins.
alter table users add column sub text;
alter table users add column email text;
alter table users add column token_rotated_at text;

create unique index users_sub on users (sub);
