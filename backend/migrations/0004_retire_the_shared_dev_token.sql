-- Retire the shared `nib-dev-token` from any row still holding it.
--
-- Before OIDC there was one seeded `developer` user, and it was seeded on *every* boot rather than
-- only under dev auth — so a database created by that build carries a row whose bearer is the
-- default `nib-dev-token`, a value published in this repo. Every API surface, `/mcp` included,
-- authenticates by bearer alone, so that row is reachable by anyone who reads the source. Migration
-- 0003 rehomed identity onto `sub` but left the token alone; this finishes the job.
--
-- It also unbreaks `just dev`: the current seed keys on `sub = 'dev'` and then sets that same
-- token, which collides with the pre-OIDC row's unique token and panics the process at boot.
--
-- The row keeps its projects and its id. It is simply no longer reachable by a known credential —
-- and under dev auth the seed re-claims the token for the real `sub = 'dev'` user, so a developer
-- picks up where they left off. A pre-OIDC row's projects stay with that row: merging two identities
-- is a guess about who owned what, and a migration is the wrong place to guess.
update users
set token = 'nib_' || lower(hex(randomblob(32))),
    token_rotated_at = datetime('now')
where token = 'nib-dev-token'
  and (sub is null or sub <> 'dev');
