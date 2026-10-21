-- An installation that is someone's own (`NEWERA_ALLOWED_EMAILS`): the
-- addresses it lets in are its owners, on a plan whose limits never get in
-- the way of personal use.
insert into plans (code, name, limits) values
    ('owner', 'Owner', '{"draft_renders_per_day": 1000000, "hq_renders_per_month": 1000000, "projects": 1000000, "storage_mb": 1000000}');

-- A plan the installation gives the account rather than one it pays for: it
-- applies over any subscription. No value: the subscription's, else free.
alter table accounts add column given_plan text references plans (code);
