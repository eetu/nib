-- Phase C: multiuser + project persistence. A project's SVG source lives in the DB (the "image
-- in SQLite"); users own projects and authenticate with an opaque bearer token.
create table users (
    id integer primary key,
    name text not null,
    token text not null unique,
    created_at text not null default (datetime('now'))
);

create table projects (
    id integer primary key,
    user_id integer not null references users (id),
    name text not null,
    svg text not null,
    created_at text not null default (datetime('now')),
    updated_at text not null default (datetime('now'))
);

create index projects_user on projects (user_id);
