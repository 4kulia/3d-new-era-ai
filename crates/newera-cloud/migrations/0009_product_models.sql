-- Where a product's model came from.
--
-- A product has at most one model, a GLB: the shop's own file, one made
-- from its photo by a generator (named in `generator`), both attached by
-- link, or a placeholder the service built from its size, colour and type
-- until either exists. A placeholder is built once, the first time the
-- product is placed, and kept as it is — a later change of size or colour
-- does not rebuild it; an attached model replaces it. `basis` records what
-- it was built from: its shape, size and colour.

alter table product_assets
    add column source text not null default 'shop'
        check (source in ('shop', 'generated', 'placeholder')),
    -- What made a generated model, e.g. `TRELLIS`.
    add column generator text,
    add column basis text;

create unique index product_assets_one_model on product_assets (product_id)
    where kind = 'model';
