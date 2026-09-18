-- Store a bearer token's HASH, not the token.
--
-- The lever this closes is the backup, not the break-in: `/var/lib/nib` is in the nightly
-- snapshot, so every copy of it held live credentials — and a snapshot gets copied around far more
-- casually than a host gets compromised.
--
-- **Existing tokens are destroyed, not converted.** Hashing a secret that has already been sitting
-- in the clear across every backup preserves the exposure while making it look solved; the honest
-- response to "these were stored readably" is to replace them. So the plaintext is overwritten
-- with random bytes here and `token_hash` starts NULL, which matches no presented token — each
-- user mints a new one from Settings. Under dev auth the seed re-claims the known dev token at
-- boot, so `just dev` keeps working untouched.
--
-- SHA-256, unsalted, no KDF — deliberately. Work factors and salts exist to make *low-entropy*
-- secrets expensive to guess; a token here is 32 random bytes minted by the server, so there is no
-- dictionary to run and nothing a KDF would buy except latency on every authenticated request. An
-- unsalted digest also keeps the lookup a single indexed read.
--
-- `token_hint` is the token's first characters, kept in the clear so the UI can say *which* token
-- a client is configured with without being able to reveal it. The rest is 56 hex characters, so
-- the hint is worth nothing as a credential.
--
-- The old column is **renamed and overwritten rather than dropped.** Dropping it would mean
-- rebuilding the table, because the inline `unique` from migration 0001 carries an implicit index
-- SQLite will not drop on its own — and rebuilding means `drop table users`, which violates
-- `projects.user_id`'s foreign key the moment any project exists. `PRAGMA foreign_keys` cannot be
-- turned off from inside a migration (SQLite ignores it within a transaction, and sqlx wraps each
-- migration in one), and deferring the check only moves the same failure to COMMIT. Renaming
-- touches no foreign key, needs no data movement, and leaves the column honestly labelled; its
-- `not null unique` is satisfied by the random value, which is why the overwrite is per-row random
-- rather than a constant.
alter table users rename column token to retired_plaintext_token;

update users set retired_plaintext_token = lower(hex(randomblob(16)));

alter table users add column token_hash text;
alter table users add column token_hint text not null default '';

create unique index if not exists users_token_hash on users (token_hash);
