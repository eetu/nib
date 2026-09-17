-- Store a bearer token's HASH, not the token.
--
-- The lever this closes is the backup, not the break-in: `/var/lib/nib` is in the nightly
-- snapshot, so every copy of it held live credentials — and a snapshot gets copied around far more
-- casually than a host gets compromised.
--
-- **Existing tokens are discarded, not converted.** Hashing a secret that has already been sitting
-- in the clear across every backup preserves the exposure while making it look solved; the honest
-- response to "these were stored readably" is to replace them. So `token_hash` starts NULL, which
-- matches no presented token, and each user mints a new one from Settings. Under dev auth the seed
-- re-claims the known dev token at boot, so `just dev` keeps working untouched.
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
-- The table is rebuilt rather than altered because the plaintext column carries an inline
-- `unique`, whose implicit index SQLite will not drop on its own — and leaving the column would
-- keep a `not null unique` field that new rows have nothing meaningful to put in.
create table users_new (
    id integer primary key,
    name text not null,
    token_hash text unique,
    token_hint text not null default '',
    created_at text not null default (datetime('now')),
    sub text,
    email text,
    token_rotated_at text
);

insert into users_new (id, name, token_hash, token_hint, created_at, sub, email, token_rotated_at)
select id, name, null, '', created_at, sub, email, token_rotated_at
from users;

drop table users;

alter table users_new rename to users;

create unique index users_sub on users (sub);
