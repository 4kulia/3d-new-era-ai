-- What a project costs: the lines an estimate adds to what the project
-- buys (labour, delivery, glue, primer), and the estimates kept as they were
-- on a day.
--
-- Money is numeric, never a float: an amount is read and written as its
-- decimal text.

create table estimate_extra (
    id          text primary key,
    account_id  text not null references accounts (id) on delete cascade,
    project_id  text not null references projects (id) on delete cascade,
    title       text not null,
    shop        text,
    quantity    numeric not null check (quantity > 0),
    unit        text not null,
    unit_price  numeric not null check (unit_price >= 0),
    currency    text not null,
    -- Whether the price includes VAT; null: not said.
    vat_included boolean,
    note        text,
    created_at  timestamptz not null default now()
);
create index estimate_extra_project on estimate_extra (project_id, created_at);

-- A saved estimate: its lines and totals as computed when it was saved.
-- Nothing changes one once it is written; it is only read or removed.
create table estimates (
    id            text primary key,
    account_id    text not null references accounts (id) on delete cascade,
    project_id    text not null references projects (id) on delete cascade,
    project_name  text not null,
    name          text not null,
    body          jsonb not null,
    created_at    timestamptz not null default now()
);
create unique index estimates_name on estimates (project_id, lower(name));
create index estimates_account on estimates (account_id, created_at desc);
