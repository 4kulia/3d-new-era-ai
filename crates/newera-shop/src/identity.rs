//! What a product link names: a shop, a market and an article in it.
//!
//! A product is `(shop, market, sku)`, not a URL: two links to one article
//! (another slug, a tracking query, `http` for `https`) are one product, so
//! importing it again finds the stored one without a fetch. The identity is
//! read from the link alone, before anything is fetched.
//!
//! A link's query is not thrown away: a shop may name the article, or its
//! variant, in it (`?product_id=50`, `?variant=…`, `?colour=…`). Every
//! parameter in [`TRACKING`] (and any `utm_*`) says where the visitor came
//! from and is dropped; all the others are part of the article — sorted, keys in
//! lower case and values as written, after the path's article in the sku — and of the address that
//! is fetched.

use url::Url;

/// One article in one shop's market.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The shop's host without `www.`: `ikea.com`, `praxis.nl`.
    pub shop: String,
    /// The market, lower case: `nl`.
    pub market: String,
    /// The article: the shop's item number where the link carries one,
    /// otherwise the link's path, then `?name=value&…` for the
    /// non-tracking query parameters the link carries, by name.
    pub sku: String,
    /// The link that is fetched: without its fragment and tracking
    /// parameters (IKEA's, without any query).
    pub url: String,
}

/// The market a link with no market of its own is in: the operator buys in
/// the Netherlands.
pub const DEFAULT_MARKET: &str = "nl";

/// The query parameters that only say where a visitor came from, lower
/// case; `utm_*` are tracking too.
pub const TRACKING: &[&str] = &[
    "gclid",
    "gclsrc",
    "gad_source",
    "gad_campaignid",
    "dclid",
    "fbclid",
    "msclkid",
    "yclid",
    "twclid",
    "ttclid",
    "igshid",
    "srsltid",
    "mc_cid",
    "mc_eid",
    "_ga",
    "_gl",
    "ref",
    "source",
    "campaign",
    "medium",
    "pk_campaign",
    "pk_source",
    "pk_medium",
    "mkt_tok",
    "_hsenc",
    "_hsmi",
    "affid",
    "aff",
    "referrer",
    "referer",
    "cmpid",
    "wt_mc",
];

fn is_tracking(name: &str) -> bool {
    name.starts_with("utm_") || TRACKING.contains(&name)
}

/// Where a shop's data comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `AnySite`'s IKEA adapter, `ikea/products`.
    Ikea,
    /// The page itself: `webparser/parse`, then `webparser/render`.
    Page,
}

impl Identity {
    pub fn source(&self) -> Source {
        if self.shop == "ikea.com" {
            Source::Ikea
        } else {
            Source::Page
        }
    }
}

/// The identity a product link names. `Err` says why the text is no
/// product link.
pub fn identify(link: &str) -> Result<Identity, String> {
    let mut url = Url::parse(link.trim()).map_err(|_| {
        format!("{link} is not a link: a product page's address, like https://www.praxis.nl/…")
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{link} is not a web page"));
    }
    let host = url
        .host_str()
        .ok_or_else(|| format!("{link} has no shop in it"))?
        .to_lowercase();
    let shop = host.strip_prefix("www.").unwrap_or(&host).to_owned();
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.set_fragment(None);
    let _ = url.set_scheme("https");
    let segments: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(str::to_lowercase).collect())
        .unwrap_or_default();
    if segments.is_empty() {
        return Err(format!("{link} is a shop's home page, not a product"));
    }
    let (market, mut sku) = if shop == "ikea.com" {
        ikea(&segments).ok_or_else(|| {
            format!(
                "{link} is no IKEA product page: those end in the item number, like https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/"
            )
        })?
    } else {
        (market_of(&shop), article_of(&segments))
    };
    if shop == "ikea.com" {
        url.set_query(None);
    } else {
        let kept: Vec<&(String, String)> = pairs
            .iter()
            .filter(|(k, _)| !is_tracking(&k.to_lowercase()))
            .collect();
        if kept.is_empty() {
            url.set_query(None);
        } else {
            url.query_pairs_mut()
                .clear()
                .extend_pairs(kept.iter().copied());
        }
        // Which parameter names the article is the shop's business
        // (`?product_id=50`, `?variant=7`): every one that is not tracking
        // does. A needless one costs a fetch, a missing one the wrong product.
        let mut named: Vec<(String, String)> = kept
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.clone()))
            .collect();
        named.sort();
        named.dedup();
        if !named.is_empty() {
            let query: Vec<String> = named.iter().map(|(k, v)| format!("{k}={v}")).collect();
            sku = format!("{sku}?{}", query.join("&"));
        }
    }
    Ok(Identity {
        shop,
        market,
        sku,
        url: url.to_string(),
    })
}

/// `/nl/nl/p/billy-boekenkast-wit-00263850/` → (`nl`, `00263850`); a
/// combination is `s` and its digits.
fn ikea(segments: &[String]) -> Option<(String, String)> {
    let market = segments.first().filter(|m| m.len() == 2)?.clone();
    let at = segments.iter().position(|s| s == "p")?;
    let slug = segments.get(at + 1)?;
    let last = slug.rsplit('-').next()?;
    let digits = last.strip_prefix('s').unwrap_or(last);
    (digits.len() == 8 && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| (market, last.to_owned()))
}

/// The market a shop's domain sells in: its country code, or the default.
fn market_of(shop: &str) -> String {
    match shop.rsplit('.').next() {
        Some(tld) if tld.len() == 2 && tld.chars().all(|c| c.is_ascii_lowercase()) => {
            tld.to_owned()
        }
        _ => DEFAULT_MARKET.to_owned(),
    }
}

/// The last path segment that is all digits — the article number Praxis
/// (`…/10043467`) and Hornbach (`/p/…/10656129/`) end their links with —
/// or the whole path where there is none (Leen Bakker's links are a slug).
fn article_of(segments: &[String]) -> String {
    segments
        .iter()
        .rev()
        .find(|s| s.len() >= 4 && s.chars().all(|c| c.is_ascii_digit()))
        .cloned()
        .unwrap_or_else(|| segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(link: &str) -> (String, String, String, String) {
        let i = identify(link).unwrap();
        (i.shop, i.market, i.sku, i.url)
    }

    /// The links of the recorded products, and the variants of one link
    /// that must name the same product.
    #[test]
    fn a_link_names_shop_market_and_article() {
        let cases = [
            (
                "https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/",
                (
                    "ikea.com",
                    "nl",
                    "00263850",
                    "https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/",
                ),
            ),
            (
                "https://www.ikea.com/de/de/p/billy-buecherregal-weiss-s19445469/?utm=x",
                (
                    "ikea.com",
                    "de",
                    "s19445469",
                    "https://www.ikea.com/de/de/p/billy-buecherregal-weiss-s19445469/",
                ),
            ),
            (
                "https://www.praxis.nl/tegels-vloeren/laminaat/laminaat-planken/decomode-laminaat-medium-dortmund-4-zijdige-v-groef-8mm-1-394m2/10043467",
                (
                    "praxis.nl",
                    "nl",
                    "10043467",
                    "https://www.praxis.nl/tegels-vloeren/laminaat/laminaat-planken/decomode-laminaat-medium-dortmund-4-zijdige-v-groef-8mm-1-394m2/10043467",
                ),
            ),
            (
                "http://hornbach.nl/p/wandtegel-elegance-wit-mat-30-x-60-cm-gerectificeerd/10656129/#reviews",
                (
                    "hornbach.nl",
                    "nl",
                    "10656129",
                    "https://hornbach.nl/p/wandtegel-elegance-wit-mat-30-x-60-cm-gerectificeerd/10656129/",
                ),
            ),
            (
                "https://www.leenbakker.nl/kasten/dressoirs/kast-alaska-wit-101x107x35cm?colour=1",
                (
                    "leenbakker.nl",
                    "nl",
                    "kasten/dressoirs/kast-alaska-wit-101x107x35cm?colour=1",
                    "https://www.leenbakker.nl/kasten/dressoirs/kast-alaska-wit-101x107x35cm?colour=1",
                ),
            ),
            (
                "https://www.example.com/products/Chair",
                (
                    "example.com",
                    "nl",
                    "products/chair",
                    "https://www.example.com/products/Chair",
                ),
            ),
        ];
        for (link, (shop, market, sku, url)) in cases {
            assert_eq!(
                id(link),
                (
                    shop.to_owned(),
                    market.to_owned(),
                    sku.to_owned(),
                    url.to_owned()
                ),
                "{link}"
            );
        }
    }

    /// Another slug or a tracking query is the same article.
    #[test]
    fn one_article_under_two_links_is_one_product() {
        let a = identify("https://www.praxis.nl/x/old-name/10043467").unwrap();
        let b = identify("https://praxis.nl/y/new-name/10043467?ref=mail").unwrap();
        assert_eq!((a.shop, a.market, a.sku), (b.shop, b.market, b.sku));
    }

    #[test]
    fn what_is_no_product_link_says_why() {
        for (link, says) in [
            ("billy", "is not a link"),
            ("ftp://praxis.nl/a", "is not a web page"),
            ("https://www.praxis.nl/", "home page"),
            (
                "https://www.ikea.com/nl/nl/cat/boekenkasten-st002/",
                "no IKEA product page",
            ),
        ] {
            let why = identify(link).unwrap_err();
            assert!(why.contains(says), "{link}: {why}");
        }
    }

    #[test]
    fn ikea_comes_from_its_adapter_and_the_rest_from_the_page() {
        assert_eq!(
            identify("https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/")
                .unwrap()
                .source(),
            Source::Ikea
        );
        assert_eq!(
            identify("https://www.praxis.nl/a/10043467")
                .unwrap()
                .source(),
            Source::Page
        );
    }
}
