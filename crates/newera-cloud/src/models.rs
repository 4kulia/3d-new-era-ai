//! The 3D model a shop product is drawn with, kept beside the product in
//! `product_assets`: one per product, a GLB.
//!
//! It comes from one of two places, and says which (`Source`):
//!
//! - a file attached by link with `edit_products action=attach_model`
//!   ([`download`], then [`keep`]): the shop's own, or one a generator made
//!   from the product's photo, named by the agent;
//! - neither yet: a placeholder built here the first time the product is
//!   placed ([`for_placement`]), from the built-in catalog piece its type
//!   names (a plain box when none does) at the shop's width, depth and
//!   height and in its colour. It keeps that shape for good, so the file
//!   under the pieces placed with it never changes; an attached model
//!   replaces it.
//!
//! Every file is read through the editor's own import before it is kept, so
//! what is kept is a model `place` can load.

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use newera_shop::product::Fields;
use sqlx::PgPool;

use crate::secret;

/// The largest model kept. A chosen bound, not a measured one: a furniture
/// GLB with its textures is a few MB, and this one travels to an open editor
/// tab inside a socket message (as base64, a third larger) and into every
/// saved copy of the project.
pub const MODEL_LIMIT: usize = 20 * 1024 * 1024;

/// How long a model's download may take. A chosen bound: the call that asks
/// for it is waiting.
const DOWNLOAD_WAIT: Duration = Duration::from_secs(60);

/// Where a product's model came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The shop's own file.
    Shop,
    /// Made by a generator from the product's photo.
    Generated,
    /// Built here from the product's size, colour and type.
    Placeholder,
}

impl Source {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shop => "shop",
            Self::Generated => "generated",
            Self::Placeholder => "placeholder",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "placeholder" => Self::Placeholder,
            "generated" => Self::Generated,
            _ => Self::Shop,
        }
    }
}

/// Where a model's link may lead: the public internet only. A link to this
/// machine, its private networks, the cloud's metadata address or the
/// tailnet would let anyone who can call `attach_model` read what only the
/// server reaches.
#[derive(Debug, Clone, Default)]
pub struct Links(Arc<RwLock<Vec<IpAddr>>>);

impl Links {
    /// Lets links reach `ip` although it is not public. Only the tests call
    /// this, for the file server they start on loopback: no configuration
    /// or variable sets it.
    ///
    /// # Panics
    ///
    /// If another thread panicked while holding the list.
    pub fn allow_for_tests(&self, ip: IpAddr) {
        self.0.write().expect("links").push(ip);
    }

    fn admits(&self, ip: IpAddr) -> bool {
        public(ip) || self.0.read().is_ok_and(|allowed| allowed.contains(&ip))
    }
}

/// Whether `ip` is an address on the public internet: not unspecified,
/// loopback, private, link-local, shared (100.64/10, the tailnet's),
/// multicast, broadcast, documentation, benchmarking or reserved, and not an
/// IPv6 form of one of those (mapped, compatible, NAT64) nor an IPv6
/// unique-local, site-local or link-local one.
#[must_use]
pub fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            !(v4.is_unspecified()
                || v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_broadcast()
                || v4.is_documentation()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (b == 18 || b == 19))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public(IpAddr::V4(v4));
            }
            let s = v6.segments();
            // NAT64 and the old IPv4-compatible form (`::a.b.c.d`) carry an
            // IPv4 address in their last 32 bits.
            if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0]
                || (s[..6] == [0; 6] && !v6.is_unspecified() && !v6.is_loopback())
            {
                let [.., hi, lo] = s;
                let v4 = std::net::Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo));
                return public(IpAddr::V4(v4));
            }
            !(v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] & 0xffc0) == 0xfec0
                || (s[0] == 0x2001 && s[1] == 0x0db8))
        }
    }
}

/// A model file, read through the editor's import: its natural width, depth
/// and height in cm.
///
/// # Errors
///
/// When it is over [`MODEL_LIMIT`], not a binary glTF, or the import cannot
/// read a mesh out of it; the error says which.
pub fn check_glb(bytes: &[u8]) -> Result<[f64; 3], String> {
    if bytes.len() > MODEL_LIMIT {
        return Err(too_big(bytes.len()));
    }
    if !bytes.starts_with(b"glTF") {
        return Err("not a GLB: a binary glTF file starts with `glTF`".to_owned());
    }
    // The import reads through the editor's file system: the bytes are put
    // there, under a name of their own, for as long as it reads them.
    let dir = std::path::PathBuf::from(format!("/memory/product-models/{}", secret::id()));
    newera_core::vfs::mount(&dir, [("model.glb".to_owned(), bytes.to_vec())]);
    let loaded = newera_catalog::load_model(&dir.join("model.glb"));
    newera_core::vfs::unmount(&dir);
    loaded
        .map(|model| model.size)
        .map_err(|e| format!("not a model the editor can read: {e}"))
}

fn too_big(size: usize) -> String {
    format!(
        "the model is {} MB, over the {} MB a product's model may be",
        size.div_ceil(1024 * 1024),
        MODEL_LIMIT / 1024 / 1024
    )
}

/// The model at `url`, an `http` or `https` link to the public internet,
/// up to [`MODEL_LIMIT`] ([`fetch`]).
///
/// # Errors
///
/// When the HTTP client cannot be built; a link that fails is a refusal.
pub async fn download(links: &Links, url: &str) -> anyhow::Result<Result<Vec<u8>, String>> {
    fetch(links, url, MODEL_LIMIT, "model", too_big).await
}

/// The bytes at `url`, an `http` or `https` link to the public internet,
/// up to `limit` (`too_big` says why a larger one is not taken); `what` is
/// what the file is, in the refusals. The host's addresses are looked up
/// first, every one must be public (or let in by `links`), and the request
/// goes to the address checked — a name that resolves elsewhere a moment
/// later is not asked again. Redirects are not followed. `Ok(Err(why))` for
/// a link that gave no file: the agent is told why.
///
/// # Errors
///
/// When the HTTP client cannot be built; a link that fails is a refusal.
pub async fn fetch(
    links: &Links,
    url: &str,
    limit: usize,
    what: &str,
    too_big: fn(usize) -> String,
) -> anyhow::Result<Result<Vec<u8>, String>> {
    let Ok(parsed) = url::Url::parse(url) else {
        return Ok(Err(format!("{url} is not a link")));
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return Ok(Err(format!(
            "{url}: a {what} is fetched over http or https"
        )));
    }
    let port = parsed.port_or_known_default().unwrap_or(80);
    let (domain, addresses): (Option<String>, Vec<SocketAddr>) = match parsed.host() {
        Some(url::Host::Ipv4(ip)) => (None, vec![SocketAddr::new(IpAddr::V4(ip), port)]),
        Some(url::Host::Ipv6(ip)) => (None, vec![SocketAddr::new(IpAddr::V6(ip), port)]),
        Some(url::Host::Domain(name)) => match tokio::net::lookup_host((name, port)).await {
            Ok(found) => (Some(name.to_owned()), found.collect()),
            Err(err) => return Ok(Err(format!("{url}: could not find {name}: {err}"))),
        },
        None => return Ok(Err(format!("{url} names no host"))),
    };
    let Some(&first) = addresses.first() else {
        return Ok(Err(format!("{url}: its host has no address")));
    };
    if let Some(off) = addresses.iter().find(|a| !links.admits(a.ip())) {
        return Ok(Err(format!(
            "{url}: {} is not on the public internet, and a {what} is fetched only from there",
            off.ip()
        )));
    }
    let mut client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(DOWNLOAD_WAIT)
        .user_agent(concat!("3d-new-era-ai/", env!("CARGO_PKG_VERSION")));
    if let Some(name) = &domain {
        client = client.resolve(name, first);
    }
    let client = client.build()?;
    let mut response = match client.get(parsed).send().await {
        Ok(response) => response,
        Err(err) => return Ok(Err(format!("could not fetch {url}: {err}"))),
    };
    let status = response.status();
    if status.is_redirection() {
        let to = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|l| l.to_str().ok())
            .unwrap_or("elsewhere");
        return Ok(Err(format!(
            "{url} redirects to {to}: give the link it redirects to"
        )));
    }
    if !status.is_success() {
        return Ok(Err(format!("{url} answered {status}")));
    }
    if let Some(length) = response.content_length()
        && usize::try_from(length).unwrap_or(usize::MAX) > limit
    {
        return Ok(Err(too_big(usize::try_from(length).unwrap_or(usize::MAX))));
    }
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                bytes.extend_from_slice(&chunk);
                if bytes.len() > limit {
                    return Ok(Err(too_big(bytes.len())));
                }
            }
            Ok(None) => return Ok(Ok(bytes)),
            Err(err) => return Ok(Err(format!("the download of {url} broke off: {err}"))),
        }
    }
}

/// Keeps `bytes` as the product's model, in place of the one it had, once
/// the editor's import reads it. This is how every model but a placeholder
/// gets in. Answers the bytes kept; `Ok(Err(why))` for a file that is no
/// model.
///
/// # Errors
///
/// When the database fails.
pub async fn keep<'e>(
    db: impl sqlx::PgExecutor<'e>,
    product: &str,
    bytes: Vec<u8>,
    source: Source,
    generator: Option<&str>,
) -> anyhow::Result<Result<Vec<u8>, String>> {
    let (checked, bytes) = tokio::task::spawn_blocking(move || (check_glb(&bytes), bytes)).await?;
    if let Err(why) = checked {
        return Ok(Err(why));
    }
    sqlx::query(
        "insert into product_assets
             (id, product_id, kind, name, content_type, data, size, source, generator)
         values ($1, $2, 'model', 'model.glb', 'model/gltf-binary', $3, $4, $5, $6)
         on conflict (product_id) where kind = 'model' do update set
             data = excluded.data, size = excluded.size, source = excluded.source,
             generator = excluded.generator, basis = null, job_id = null, created_at = now()",
    )
    .bind(secret::id())
    .bind(product)
    .bind(&bytes)
    .bind(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
    .bind(source.as_str())
    .bind(generator.map(str::trim).filter(|g| !g.is_empty()))
    .execute(db)
    .await?;
    Ok(Ok(bytes))
}

/// What a placeholder is built from: the catalog piece it is drawn as, its
/// size and its colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// The built-in catalog piece, or `None` for a plain box.
    pub catalog: Option<&'static str>,
    /// Width, depth, height, cm.
    pub size: [f64; 3],
    pub colour: Option<[u8; 3]>,
}

impl Look {
    /// What a stored placeholder records it was built from: its shape (the
    /// catalog id, or `box`) first, then its size and colour.
    #[must_use]
    pub fn basis(&self) -> String {
        let colour = self
            .colour
            .map_or_else(String::new, |[r, g, b]| format!(" #{r:02x}{g:02x}{b:02x}"));
        format!(
            "{} {}x{}x{}{colour}",
            self.shape(),
            self.size[0],
            self.size[1],
            self.size[2]
        )
    }

    /// The catalog id it is drawn as, or `box`.
    #[must_use]
    pub fn shape(&self) -> &'static str {
        self.catalog.unwrap_or("box")
    }
}

/// A product's width, depth and height in cm, each when known.
#[must_use]
pub fn dimensions(fields: &Fields) -> [Option<f64>; 3] {
    ["width_cm", "depth_cm", "height_cm"].map(|name| {
        fields
            .get(name)
            .and_then(|f| f.value.as_f64())
            .filter(|v| *v > 0.0)
    })
}

/// The catalog piece a placeholder is drawn as: `asked`, when it names one;
/// else the one the product's type names. `None` is a box.
#[must_use]
pub fn shape_for(fields: &Fields, asked: Option<&str>) -> Option<&'static str> {
    asked
        .and_then(newera_catalog::find)
        .or_else(|| {
            fields
                .get("product_type")
                .and_then(|f| f.value.as_str())
                .and_then(catalog_for)
        })
        .map(|item| item.id)
}

/// The look of a product's placeholder in `shape` (`None`: a box), at its
/// size, in its colour. `None` without all three dimensions.
#[must_use]
pub fn look(fields: &Fields, shape: Option<&'static str>) -> Option<Look> {
    let [Some(w), Some(d), Some(h)] = dimensions(fields) else {
        return None;
    };
    let colour = fields
        .get("colour_hex")
        .and_then(|f| f.value.as_str())
        .and_then(newera_shop::look::hex);
    Some(Look {
        catalog: shape,
        size: [w, d, h],
        colour,
    })
}

/// Shop words for a kind of furniture — the Dutch IKEA, bol and Leen Bakker
/// use, and English — and the catalog piece each is drawn as. A word of
/// the product's type is looked up whole, the first that is here wins.
const TYPE_WORDS: &[(&str, &str)] = &[
    ("boekenkast", "bookcase"),
    ("boekenrek", "bookcase"),
    ("wandkast", "bookcase"),
    ("vakkenkast", "bookcase"),
    ("bookcase", "bookcase"),
    ("bookshelf", "bookcase"),
    ("shelving", "bookcase"),
    ("hoekbank", "sofa-l"),
    ("bank", "sofa-3"),
    ("bankstel", "sofa-3"),
    ("zitsbank", "sofa-3"),
    ("slaapbank", "sofa-3"),
    ("sofa", "sofa-3"),
    ("couch", "sofa-3"),
    ("fauteuil", "armchair"),
    ("armchair", "armchair"),
    ("bureaustoel", "office-chair"),
    ("stoel", "chair"),
    ("eetkamerstoel", "chair"),
    ("chair", "chair"),
    ("kruk", "stool"),
    ("barkruk", "stool"),
    ("stool", "stool"),
    ("salontafel", "coffee-table"),
    ("bijzettafel", "side-table"),
    ("eettafel", "dining-table-4"),
    ("tafel", "dining-table-4"),
    ("table", "dining-table-4"),
    ("tweepersoonsbed", "bed-double"),
    ("eenpersoonsbed", "bed-single"),
    ("bedframe", "bed-double"),
    ("bed", "bed-double"),
    ("nachtkastje", "nightstand"),
    ("nightstand", "nightstand"),
    ("kledingkast", "wardrobe"),
    ("garderobekast", "wardrobe"),
    ("wardrobe", "wardrobe"),
    ("ladekast", "dresser"),
    ("commode", "dresser"),
    ("dresser", "dresser"),
    ("dressoir", "sideboard"),
    ("sideboard", "sideboard"),
    ("tv-meubel", "tv-stand"),
    ("bureau", "desk"),
    ("desk", "desk"),
    ("vloerkleed", "rug"),
    ("karpet", "rug"),
    ("rug", "rug"),
];

/// The catalog piece a product type names: by [`TYPE_WORDS`], else the
/// first catalog piece whose id, name or keywords hold one of its words
/// whole.
fn catalog_for(kind: &str) -> Option<&'static newera_catalog::CatalogItem> {
    let folded = newera_catalog::fold(kind);
    let wanted: Vec<&str> = folded
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .flat_map(|w| std::iter::once(w).chain(w.split('-')))
        .filter(|w| w.len() > 2)
        .collect();
    if let Some(id) = wanted
        .iter()
        .find_map(|w| TYPE_WORDS.iter().find(|(word, _)| word == w))
        .map(|(_, id)| *id)
    {
        return newera_catalog::find(id);
    }
    newera_catalog::search(kind).into_iter().find(|item| {
        let words = newera_catalog::fold(&format!("{} {} {}", item.id, item.name, item.keywords));
        words
            .split(|c: char| !c.is_alphanumeric())
            .any(|w| wanted.contains(&w))
    })
}

/// The placeholder for `look`, as a GLB: the catalog piece (or a box) at its
/// size and colour, exported the way the editor exports a scene.
#[must_use]
pub fn placeholder_glb(look: &Look) -> Vec<u8> {
    let mut home = newera_core::Home::default();
    let mut piece = newera_core::Furniture {
        catalog: look.shape().to_owned(),
        width: look.size[0],
        depth: look.size[1],
        height: look.size[2],
        color: look.colour,
        visible: true,
        ..newera_core::Furniture::default()
    };
    piece.id = home.new_furniture_id();
    home.furniture.push(piece);
    newera_render::glb_home(&home)
}

/// A product's model, ready to place.
#[derive(Debug, Clone)]
pub struct Model {
    pub bytes: Vec<u8>,
    pub source: Source,
    /// For a placeholder: the shape it is drawn as.
    pub shape: Option<String>,
}

/// The model `product` is placed with: the one kept, or — when it has none
/// — a placeholder built now in the shape `asked` (a catalog id the call
/// gave), or its type names, and kept. A placeholder kept already is the
/// one placed, whatever its size or colour are now: the file under the
/// pieces placed with it never changes, and `asked` must be its shape.
/// `Ok(Err(why))` for a placement that cannot be: no model and no size to
/// build one at, or another shape asked.
///
/// # Errors
///
/// When the database fails.
pub async fn for_placement(
    db: &PgPool,
    product: &str,
    fields: &Fields,
    asked: Option<&str>,
) -> anyhow::Result<Result<Model, String>> {
    let kept: Option<(Vec<u8>, String, Option<String>)> = sqlx::query_as(
        "select data, source, basis from product_assets
         where product_id = $1 and kind = 'model'",
    )
    .bind(product)
    .fetch_optional(db)
    .await?;
    let Some((bytes, source, basis)) = kept else {
        let Some(look) = look(fields, shape_for(fields, asked)) else {
            return Ok(Err(format!(
                "{product} has no 3D model and no width, depth and height to draw one at: edit_products action=attach_model gives it a model, or action=set fields width_cm, depth_cm, height_cm"
            )));
        };
        let basis = look.basis();
        let bytes = tokio::task::spawn_blocking(move || placeholder_glb(&look)).await?;
        // One kept meanwhile — a model attached, or another call's
        // placeholder — is the one placed.
        sqlx::query(
            "insert into product_assets
                 (id, product_id, kind, name, content_type, data, size, source, basis)
             values ($1, $2, 'model', 'model.glb', 'model/gltf-binary', $3, $4, 'placeholder', $5)
             on conflict (product_id) where kind = 'model' do nothing",
        )
        .bind(secret::id())
        .bind(product)
        .bind(&bytes)
        .bind(i32::try_from(bytes.len()).unwrap_or(i32::MAX))
        .bind(&basis)
        .execute(db)
        .await?;
        return Box::pin(for_placement(db, product, fields, asked)).await;
    };
    let source = Source::parse(&source);
    let shape = basis
        .as_deref()
        .and_then(|b| b.split_whitespace().next())
        .map(str::to_owned);
    if source == Source::Placeholder
        && let (Some(asked), Some(shape)) = (asked, shape.as_deref())
        && asked != shape
    {
        return Ok(Err(format!(
            "{product} is drawn as {shape} already: cat={asked} would change the placeholder under the pieces placed with it; leave cat out"
        )));
    }
    Ok(Ok(Model {
        bytes,
        source,
        shape: (source == Source::Placeholder).then_some(shape).flatten(),
    }))
}

#[cfg(test)]
mod tests {
    use newera_shop::product::{Evidence, Field, Fields};
    use serde_json::{Value, json};

    fn fields(of: &[(&str, Value)]) -> Fields {
        of.iter()
            .map(|(name, value)| {
                (
                    (*name).to_owned(),
                    Field {
                        value: value.clone(),
                        evidence: Evidence::Adapter,
                        from: "test".to_owned(),
                    },
                )
            })
            .collect()
    }

    fn sized(extra: &[(&str, Value)]) -> Fields {
        let mut all = vec![
            ("width_cm", json!(80.0)),
            ("depth_cm", json!(28.0)),
            ("height_cm", json!(202.0)),
        ];
        all.extend_from_slice(extra);
        fields(&all)
    }

    #[test]
    fn a_placeholder_is_the_catalog_piece_its_type_names_at_the_shops_size() {
        let sofa_fields = sized(&[
            ("product_type", json!("3-zitsbank / Sofa")),
            ("colour_hex", json!("#eeebe1")),
        ]);
        let sofa = super::look(&sofa_fields, super::shape_for(&sofa_fields, None)).unwrap();
        assert_eq!(
            sofa,
            super::Look {
                catalog: Some("sofa-3"),
                size: [80.0, 28.0, 202.0],
                colour: Some([0xee, 0xeb, 0xe1]),
            }
        );
        assert_eq!(sofa.basis(), "sofa-3 80x28x202 #eeebe1");
        let unknown = sized(&[("product_type", json!("Spiegel"))]);
        let unknown = super::look(&unknown, super::shape_for(&unknown, None)).unwrap();
        assert_eq!(
            (unknown.catalog, unknown.basis()),
            (None, "box 80x28x202".to_owned())
        );
        assert_eq!(
            super::shape_for(&sized(&[]), Some("wardrobe")),
            Some("wardrobe")
        );
        assert_eq!(super::shape_for(&sized(&[]), Some("nope")), None);
        let mut no_depth = sized(&[]);
        no_depth.remove("depth_cm");
        assert_eq!(super::look(&no_depth, None), None);
    }

    /// The shop's own words for a kind of furniture, Dutch or English, name
    /// the catalog piece: BILLY, as the IKEA adapter reads it, is a
    /// bookcase.
    #[test]
    fn a_shop_type_name_is_drawn_as_its_catalog_piece() {
        let record: serde_json::Value = serde_json::from_str(include_str!(
            "../../newera-shop/tests/fixtures/ikea_billy_00263850.json"
        ))
        .unwrap();
        let billy = newera_shop::extract::from_ikea(&record["body"][0]);
        assert_eq!(super::shape_for(&billy, None), Some("bookcase"));
        for (kind, shape) in [
            ("Boekenkast", Some("bookcase")),
            ("Bank", Some("sofa-3")),
            ("Bankstel", Some("sofa-3")),
            ("3-zitsbank", Some("sofa-3")),
            ("Hoekbank", Some("sofa-l")),
            ("Fauteuil", Some("armchair")),
            ("Stoel", Some("chair")),
            ("Bureaustoel", Some("office-chair")),
            ("Tafel", Some("dining-table-4")),
            ("Eettafel", Some("dining-table-4")),
            ("Salontafel", Some("coffee-table")),
            ("Bed", Some("bed-double")),
            ("Kledingkast", Some("wardrobe")),
            ("Ladekast", Some("dresser")),
            ("Bureau", Some("desk")),
            ("TV-meubel", Some("tv-stand")),
            ("Bookcase", Some("bookcase")),
            ("Wardrobe", Some("wardrobe")),
            ("Spiegel", None),
        ] {
            let fields = fields(&[("product_type", json!(kind))]);
            assert_eq!(super::shape_for(&fields, None), shape, "{kind}");
        }
    }

    #[test]
    fn only_an_address_on_the_public_internet_is_public() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.178.33",
            "169.254.169.254",
            "100.76.21.14",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "192.0.2.1",
            "198.18.0.1",
            "240.0.0.1",
            "::",
            "::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a9fe:a9fe",
            "fe80::1",
            "fd00::1",
            "ff02::1",
            "2001:db8::1",
            "::127.0.0.1",
            "::10.0.0.1",
            "fec0::1",
        ] {
            assert!(!super::public(private.parse().unwrap()), "{private}");
        }
        for public in ["8.8.8.8", "185.70.186.229", "100.128.0.1", "2a00:1450::1"] {
            assert!(super::public(public.parse().unwrap()), "{public}");
        }
        // Every catalog piece the shop words name is there to draw.
        for (word, id) in super::TYPE_WORDS {
            assert!(newera_catalog::find(id).is_some(), "{word} → {id}");
        }
        let links = super::Links::default();
        let loopback = "127.0.0.1".parse().unwrap();
        assert!(!links.admits(loopback));
        links.allow_for_tests(loopback);
        assert!(links.admits(loopback));
    }

    #[test]
    fn a_placeholder_glb_reads_back_at_its_size() {
        for catalog in [None, Some("sofa-3")] {
            let look = super::Look {
                catalog,
                size: [80.0, 28.0, 202.0],
                colour: Some([10, 20, 30]),
            };
            let size = super::check_glb(&super::placeholder_glb(&look)).unwrap();
            for (got, want) in size.iter().zip(look.size) {
                assert!((got - want).abs() < 0.5, "{catalog:?}: {size:?}");
            }
        }
    }

    #[test]
    fn only_a_readable_glb_under_the_limit_is_a_model() {
        assert_eq!(
            super::check_glb(b"solid cube\n"),
            Err("not a GLB: a binary glTF file starts with `glTF`".to_owned())
        );
        let broken = super::check_glb(b"glTF\x02\x00\x00\x00\x0c\x00\x00\x00").unwrap_err();
        assert!(
            broken.starts_with("not a model the editor can read: "),
            "{broken}"
        );
        let mut big = b"glTF".to_vec();
        big.resize(super::MODEL_LIMIT + 1, 0);
        assert_eq!(
            super::check_glb(&big),
            Err("the model is 21 MB, over the 20 MB a product's model may be".to_owned())
        );
    }
}
