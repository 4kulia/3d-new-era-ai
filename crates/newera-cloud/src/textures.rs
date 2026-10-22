//! The texture a finish product is drawn with, kept beside the product in
//! `product_assets`: one per product, made from its swatch picture.
//!
//! What makes a finish's look is `newera_shop::look`: a swatch picture of
//! known size, or the shop's colour. The picture is fetched from the link
//! the product names (`swatch_image`), read the way the editor reads
//! images, and kept as a JPEG of at most [`TEXTURE_SIDE`] px on its longer
//! side, with the link it was made from (`basis`): the same link is not
//! fetched again, a new one makes it again. In a project it is
//! `products/<id>/finish.jpg`, drawn as a material image tiled at the
//! sample's real size ([`material`]).

use std::io::Cursor;

use sqlx::PgPool;

use crate::accounts::Plan;
use crate::models::Links;
use crate::secret;

/// The largest swatch picture fetched. A chosen bound, not a measured one:
/// a shop's product photo is a few hundred kB to a few MB.
pub const PICTURE_LIMIT: usize = 10 * 1024 * 1024;

/// The longer side of a kept texture, px. A chosen bound: the editor's 3D
/// view draws every image at 512 px, and the photo renderer gets twice that.
pub const TEXTURE_SIDE: u32 = 1024;

/// JPEG quality of a kept texture. A chosen value.
const QUALITY: u8 = 90;

/// Where a finish's texture sits in a project.
#[must_use]
pub fn texture_path(product: &str) -> String {
    format!("products/{product}/finish.jpg")
}

fn too_big(size: usize) -> String {
    format!(
        "the swatch picture is {} MB, over the {} MB it may be",
        size.div_ceil(1024 * 1024),
        PICTURE_LIMIT / 1024 / 1024
    )
}

/// A picture made ready to draw: decoded the way the editor decodes images,
/// brought down to [`TEXTURE_SIDE`] on its longer side, laid on white where
/// it is see-through, as a JPEG.
///
/// # Errors
///
/// When the bytes are no image the editor reads.
pub fn derive(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let image = newera_core::images::decode(bytes)
        .map_err(|e| format!("not a picture the editor can read: {e}"))?;
    let image = if image.width().max(image.height()) > TEXTURE_SIDE {
        image.resize(
            TEXTURE_SIDE,
            TEXTURE_SIDE,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    // A JPEG keeps no alpha: what is see-through is laid on white.
    let rgba = image.to_rgba8();
    let flat = image::RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let over = |c: u8| {
            let mixed = (u32::from(c) * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255;
            u8::try_from(mixed).unwrap_or(u8::MAX)
        };
        image::Rgb([over(r), over(g), over(b)])
    });
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, QUALITY)
        .encode_image(&flat)
        .map_err(|e| format!("the picture could not be written as a texture: {e}"))?;
    Ok(out)
}

/// Width and height of an image, px, from its header.
#[must_use]
pub fn dimensions(bytes: &[u8]) -> Option<[u32; 2]> {
    let (w, h) = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    Some([w, h])
}

/// The size `[across, up]` in cm a picture of `px` is drawn at, for a
/// sample of `size`. A wallpaper's repeat hangs one way: the roll's width
/// across, the repeat up. A tile or plank lies the way its picture does:
/// the longer side of the sample along the longer side of the picture, and
/// as given when the picture is square.
#[must_use]
pub fn drawn_size(size: [f64; 2], hangs: bool, px: [u32; 2]) -> [f64; 2] {
    let [a, b] = size;
    if hangs {
        return size;
    }
    let (long, short) = (a.max(b), a.min(b));
    match px[0].cmp(&px[1]) {
        std::cmp::Ordering::Greater => [long, short],
        std::cmp::Ordering::Less => [short, long],
        std::cmp::Ordering::Equal => size,
    }
}

/// The size `[across, up]` in cm the surfaces drawn with a finish's
/// texture take: the product's sample size (`newera_shop::look`), laid the
/// way the texture `bytes` lies ([`drawn_size`]), when the texture was made
/// from the swatch the product names now (`basis`, the link it was made
/// from). `None` when the product has no sized swatch, or the texture is
/// not of it. Every size a texture is drawn at is decided here: when a
/// swatch or size changes (`edit_products`), and when a project is opened
/// (`crate::current`).
#[must_use]
pub fn drawn_tile(
    fields: &newera_shop::product::Fields,
    basis: Option<&str>,
    bytes: &[u8],
) -> Option<[f64; 2]> {
    match newera_shop::look::appearance(fields).look {
        Some(newera_shop::look::Look::Picture { url, size, hangs }) if basis == Some(&*url) => {
            dimensions(bytes).map(|px| drawn_size(size, hangs, px))
        }
        _ => None,
    }
}

/// The look a surface takes from the product's texture drawn at `tile` cm,
/// in the editor's material text.
#[must_use]
pub fn material(product: &str, tile: [f64; 2]) -> String {
    newera_core::Material {
        image: Some(texture_path(product)),
        tile: Some(tile),
        ..newera_core::Material::default()
    }
    .to_string()
}

/// The look a surface takes from the shop's colour.
#[must_use]
pub fn colour(rgb: [u8; 3]) -> String {
    newera_core::Material::paint(rgb).to_string()
}

/// Why a swatch gave no texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unused {
    /// The link gives no picture the editor reads; the words say why.
    Picture(String),
    /// The plan's storage has no room for it.
    NoRoom(String),
}

impl std::fmt::Display for Unused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Picture(why) => f.write_str(why),
            Self::NoRoom(why) => write!(f, "the texture was not kept: {why}"),
        }
    }
}

/// The link the texture kept for `product` was made from: `None` when it
/// has no texture, `Some(None)` for one kept without its link.
///
/// # Errors
///
/// When the database fails.
pub async fn basis(db: &PgPool, product: &str) -> sqlx::Result<Option<Option<String>>> {
    sqlx::query_scalar(
        "select basis from product_assets where product_id = $1 and kind = 'texture'",
    )
    .bind(product)
    .fetch_optional(db)
    .await
}

/// The texture kept for `product`, and the link it was made from.
///
/// # Errors
///
/// When the database fails.
pub async fn kept(db: &PgPool, product: &str) -> sqlx::Result<Option<(Vec<u8>, Option<String>)>> {
    sqlx::query_as(
        "select data, basis from product_assets where product_id = $1 and kind = 'texture'",
    )
    .bind(product)
    .fetch_optional(db)
    .await
}

/// The texture of `product` made from the swatch at `url`: the one kept
/// when it was made from that link, else fetched, made and kept in place
/// of any other — it counts against the plan's storage, in place of the
/// one it replaces. `Ok(Err(why))` for a link that gives no picture, or no
/// room for it.
///
/// # Errors
///
/// When the database fails or the HTTP client cannot be built.
pub async fn for_swatch(
    db: &PgPool,
    links: &Links,
    account: &str,
    plan: &Plan,
    product: &str,
    url: &str,
) -> anyhow::Result<Result<Vec<u8>, Unused>> {
    if let Some((bytes, basis)) = kept(db, product).await?
        && basis.as_deref() == Some(url)
    {
        return Ok(Ok(bytes));
    }
    let picture =
        match crate::models::fetch(links, url, PICTURE_LIMIT, "swatch picture", too_big).await? {
            Ok(bytes) => bytes,
            Err(why) => return Ok(Err(Unused::Picture(why))),
        };
    let made = tokio::task::spawn_blocking(move || derive(&picture)).await?;
    let bytes = match made {
        Ok(bytes) => bytes,
        Err(why) => return Ok(Err(Unused::Picture(format!("{url}: {why}")))),
    };
    let used = crate::engine::kept_bytes(db, account, None, Some((product, "texture"))).await?;
    let size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
    if let Some(why) = crate::engine::no_room(plan, used.saturating_add(size)) {
        return Ok(Err(Unused::NoRoom(why)));
    }
    sqlx::query(
        "insert into product_assets
             (id, product_id, kind, name, content_type, data, size, source, basis)
         values ($1, $2, 'texture', 'finish.jpg', 'image/jpeg', $3, $4, 'shop', $5)
         on conflict (product_id) where kind = 'texture' do update set
             data = excluded.data, size = excluded.size, basis = excluded.basis,
             created_at = now()",
    )
    .bind(secret::id())
    .bind(product)
    .bind(&bytes)
    .bind(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
    .bind(url)
    .execute(db)
    .await?;
    Ok(Ok(bytes))
}

#[cfg(test)]
mod tests {
    fn picture(w: u32, h: u32, format: image::ImageFormat) -> Vec<u8> {
        let image = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([
                u8::try_from(x % 256).unwrap(),
                u8::try_from(y % 256).unwrap(),
                90,
            ])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    #[test]
    fn a_picture_becomes_a_jpeg_no_larger_than_the_side() {
        let big = super::derive(&picture(2400, 1200, image::ImageFormat::Png)).unwrap();
        assert!(big.starts_with(&[0xff, 0xd8]), "a JPEG");
        assert_eq!(super::dimensions(&big), Some([1024, 512]));
        let small = super::derive(&picture(300, 600, image::ImageFormat::WebP)).unwrap();
        assert_eq!(super::dimensions(&small), Some([300, 600]));
        let not = super::derive(b"<html>no</html>").unwrap_err();
        assert!(
            not.starts_with("not a picture the editor can read: "),
            "{not}"
        );
    }

    /// A see-through swatch is laid on white, not on black: a JPEG keeps no
    /// alpha.
    #[test]
    fn a_see_through_picture_is_laid_on_white() {
        let image = image::RgbaImage::from_fn(4, 2, |x, _| {
            if x < 2 {
                image::Rgba([255, 0, 0, 0])
            } else {
                image::Rgba([0, 0, 0, 128])
            }
        });
        let mut png = std::io::Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let made = super::derive(&png.into_inner()).unwrap();
        let back = image::load_from_memory(&made).unwrap().to_rgb8();
        let clear = back.get_pixel(0, 0).0;
        assert!(clear.iter().all(|c| *c > 235), "{clear:?}");
        let half = back.get_pixel(3, 1).0;
        assert!(half.iter().all(|c| (110..=145).contains(c)), "{half:?}");
    }

    #[test]
    fn a_tile_lies_the_way_its_picture_does_and_a_wallpaper_hangs() {
        let size = [30.0, 60.0];
        assert_eq!(super::drawn_size(size, false, [800, 400]), [60.0, 30.0]);
        assert_eq!(super::drawn_size(size, false, [400, 800]), [30.0, 60.0]);
        assert_eq!(
            super::drawn_size([60.0, 30.0], false, [400, 800]),
            [30.0, 60.0]
        );
        assert_eq!(super::drawn_size(size, false, [500, 500]), [30.0, 60.0]);
        assert_eq!(
            super::drawn_size([53.0, 64.0], true, [800, 300]),
            [53.0, 64.0]
        );
    }

    #[test]
    fn the_look_is_the_editor_s_material_text() {
        assert_eq!(
            super::material("p1", [60.0, 30.0]),
            "img:products/p1/finish.jpg 60x30"
        );
        assert_eq!(
            super::material("p1", [33.3, 33.3]),
            "img:products/p1/finish.jpg 33.3"
        );
        assert_eq!(super::colour([0xf2, 0xef, 0xe6]), "#f2efe6");
        let parsed: newera_core::Material = super::material("p1", [60.0, 30.0]).parse().unwrap();
        assert_eq!(
            (parsed.image.as_deref(), parsed.tile),
            (Some("products/p1/finish.jpg"), Some([60.0, 30.0]))
        );
    }
}
