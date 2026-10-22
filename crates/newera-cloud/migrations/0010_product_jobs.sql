-- A furniture product's 3D model being made from its photo by the
-- generator (`deploy/spark-generator/`).
--
-- A job is `queued` until the generator has it, `running` while it works
-- on it, `done` once its model is kept with the product, `failed` when the
-- generator refused it or every attempt was spent. `provider_job` is the
-- generator's own id, so a job asked after survives a restart of this
-- service; `attempts` counts the tries that failed, `next_at` is when the
-- job is next looked at, and `state_since` when the generator was given it
-- or last moved it on (queued to running), so a job it has been running
-- too long is given again. A product has at most one job open.

create table product_jobs (
    id            text primary key,
    account_id    text not null references accounts (id) on delete cascade,
    product_id    text not null references products (id) on delete cascade,
    provider_job  text,
    status        text not null default 'queued'
        check (status in ('queued', 'running', 'done', 'failed')),
    error         text,
    attempts      integer not null default 0,
    next_at       timestamptz not null default now(),
    state_since   timestamptz not null default now(),
    created_at    timestamptz not null default now(),
    updated_at    timestamptz not null default now()
);
create unique index product_jobs_open on product_jobs (product_id)
    where status in ('queued', 'running');
create index product_jobs_due on product_jobs (next_at)
    where status in ('queued', 'running');
create index product_jobs_product on product_jobs (product_id, created_at desc);

-- The job a generated model was kept by: a later try of the same job
-- finds its own model, and any other model kept since — attached by hand,
-- whatever generator it names — stays.
alter table product_assets add column job_id text;
