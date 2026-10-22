-- A finish product's texture: its swatch picture, read and made ready for
-- the editor (a JPEG at most 1024 px on its longer side). A product has at
-- most one; `basis` is the link it was made from, so a new swatch link
-- makes it again and the same link does not.

create unique index product_assets_one_texture on product_assets (product_id)
    where kind = 'texture';
