-- An account's library of shop products, read through AnySite.
--
-- A product is one article in one shop's market: importing a link to an
-- article already here returns it, without a fetch. Its fields are one
-- JSON object, each field with the evidence it was read from (adapter,
-- text, market, agent); a field nobody could read is absent.

create table products (
    id          text primary key,
    account_id  text not null references accounts (id) on delete cascade,
    shop        text not null,
    market      text not null,
    sku         text not null,
    url         text not null,
    kind        text not null check (kind in ('furniture', 'finish')),
    fields      jsonb not null default '{}',
    -- The AnySite calls each fetch made: [{endpoint, outcome, credits}].
    fetches     jsonb not null default '[]',
    fetched_at  timestamptz not null default now(),
    created_at  timestamptz not null default now(),
    updated_at  timestamptz not null default now()
);
create unique index products_identity on products (account_id, shop, market, sku);
create index products_account on products (account_id, updated_at desc);

-- Every price a product was seen at, read from the shop or set by the
-- agent: what the estimate's prices were on a day.
create table product_prices (
    product_id    text not null references products (id) on delete cascade,
    price         numeric not null,
    currency      text not null,
    vat_included  boolean,
    sale_unit     text,
    origin        text not null check (origin in ('adapter', 'text', 'agent')),
    seen_at       timestamptz not null default now()
);
create index product_prices_product on product_prices (product_id, seen_at desc);

-- The files a product brings into a project: its 3D model, a texture, a
-- photo.
create table product_assets (
    id            text primary key,
    product_id    text not null references products (id) on delete cascade,
    kind          text not null check (kind in ('model', 'texture', 'image')),
    name          text not null,
    content_type  text not null,
    data          bytea not null,
    size          integer not null,
    created_at    timestamptz not null default now()
);
create index product_assets_product on product_assets (product_id);
