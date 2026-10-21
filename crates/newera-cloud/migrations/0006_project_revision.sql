-- One copy of a project, written by the editor tab and by the engine: each
-- save names the revision it started from and makes the next one, and a save
-- from a revision that is no longer the stored one is refused.
alter table projects add column revision bigint not null default 0;
