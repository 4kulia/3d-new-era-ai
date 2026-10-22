//! What the page reader, the extractor and the identity do with the odd
//! inputs real pages and links bring: text that ends inside a tag,
//! characters whose lower case is longer, a price written `1.299,-`, a
//! price without VAT, a link that selects a variant.

use newera_shop::Evidence;
use newera_shop::extract::from_page;
use newera_shop::identify;
use newera_shop::page::Page;
use serde_json::json;

/// A `<` at the end of the text, followed by a multibyte character, is
/// read as text and never cuts the character.
#[test]
fn an_open_tag_at_the_end_never_panics() {
    for html in ["prijs <€", "<p>prijs</p> <€", "<", "a <b", "x <\u{1F600}"] {
        let _ = Page::read(html);
    }
    assert_eq!(Page::read("<p>prijs <€").lines, ["prijs"]);
}

/// A character whose lower case is longer than itself (`İ`, 2 bytes, lowers
/// to 3) before a `label:` does not move where its value starts.
#[test]
fn a_value_is_cut_where_the_original_line_has_it() {
    let page = Page::read("<p>İİ Kleur: wit</p><p>Kleur: İ wit</p>");
    assert_eq!(
        page.value("Kleur"),
        Some(("İ wit", "Kleur: İ wit".to_owned()))
    );
    let page = Page::read("<p>Kleur: Grijs</p>");
    assert_eq!(
        page.value("kleur"),
        Some(("Grijs", "Kleur: Grijs".to_owned()))
    );
    let page = Page::read("<p>İnhoud: 2,5 L</p>");
    assert_eq!(
        page.value("İnhoud"),
        Some(("2,5 L", "İnhoud: 2,5 L".to_owned()))
    );
}

/// An `&` next to multibyte text, where the entity window ends inside a
/// character, is text.
#[test]
fn an_entity_next_to_multibyte_text_never_panics() {
    let page = Page::read("<p>&€€€€€€ x</p>");
    assert_eq!(page.lines, ["&€€€€€€ x"]);
}

/// `1.299,-` is a price like `135,-`.
#[test]
fn a_thousands_price_written_with_a_dash_is_read() {
    for (shown, price) in [
        ("135,-", "135"),
        ("1.299,-", "1299"),
        ("1.299,50", "1299.50"),
    ] {
        let page = json!({
            "cleaned_html": format!("<h1>Kast Alaska</h1><div>{shown}</div>")
        });
        let (_, fields) = from_page(&page, None);
        assert_eq!(fields["price"].value, price, "{shown}");
    }
}

/// A price a page says is without VAT is not recorded as including it.
#[test]
fn a_price_without_btw_is_not_marked_with_it() {
    let read = |lines: &str| {
        let page = json!({
            "cleaned_html": format!("<h1>Kast Alaska</h1><div>135,20</div>{lines}")
        });
        let (_, fields) = from_page(&page, None);
        let vat = &fields["vat_included"];
        (vat.value.clone(), vat.evidence, vat.from.clone())
    };
    let said = |lines: &str| {
        let page = json!({
            "cleaned_html": format!("<h1>Kast Alaska</h1><div>135,20</div>{lines}")
        });
        from_page(&page, None).1.contains_key("vat_included")
    };
    for line in [
        "excl. btw",
        "Prijs excl. BTW",
        "exclusief btw",
        "ex btw",
        "excl. 21% btw",
    ] {
        assert_eq!(
            read(&format!("<div>{line}</div>")),
            (json!(false), Evidence::Text, line.to_owned()),
            "{line}"
        );
    }
    assert_eq!(
        read("<div>incl. btw</div>"),
        (json!(true), Evidence::Text, "incl. btw".to_owned())
    );
    assert!(
        !said(""),
        "a page silent on VAT says nothing of it: the market's rule is the import's"
    );
    assert_eq!(
        read("<div>excl. btw</div><div>incl. btw</div>").0,
        json!(true),
        "a page that says both gives no reason to turn the price into one without VAT"
    );
}

fn id(link: &str) -> (String, String) {
    let i = identify(link).unwrap();
    (i.sku, i.url)
}

/// A variant parameter names another article and is fetched; a tracking
/// one is dropped; the order of the parameters does not matter.
#[test]
fn a_variant_is_another_article_and_tracking_is_not() {
    let plain = id("https://www.shop.nl/p/stoel/10043467");
    let red = id("https://www.shop.nl/p/stoel/10043467?variant=77&utm_source=mail&gclid=x#top");
    assert_eq!(
        red,
        (
            "10043467?variant=77".to_owned(),
            "https://www.shop.nl/p/stoel/10043467?variant=77".to_owned()
        )
    );
    assert_ne!(plain.0, red.0);
    let swapped = id("https://shop.nl/p/stoel/10043467?Maat=L&Kleur=Rood&fbclid=1");
    let other = id("https://shop.nl/p/stoel/10043467?kleur=Rood&maat=L");
    assert_eq!(swapped.0, "10043467?kleur=Rood&maat=L");
    assert_eq!(swapped.0, other.0);
    let tracked = id("https://shop.nl/p/stoel/10043467?utm_medium=x&ref=mail");
    assert_eq!(
        tracked,
        (
            "10043467".to_owned(),
            "https://shop.nl/p/stoel/10043467".to_owned()
        )
    );
    let paged = id("https://shop.nl/p/stoel/10043467?tab=specs&utm_x=1");
    assert_eq!(
        paged,
        (
            "10043467?tab=specs".to_owned(),
            "https://shop.nl/p/stoel/10043467?tab=specs".to_owned()
        ),
        "any parameter but a tracking one may select the article"
    );
    let ikea = id("https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/?variant=1");
    assert_eq!(
        ikea.1,
        "https://www.ikea.com/nl/nl/p/billy-boekenkast-wit-00263850/"
    );
}

/// Shops that keep the article in a parameter of their own: two articles
/// are two products, and tracking changes nothing.
#[test]
fn an_article_in_any_parameter_names_the_product() {
    let a = id("https://shop.nl/index.php?route=product/product&product_id=50");
    let b = id("https://shop.nl/index.php?route=product/product&product_id=51");
    assert_ne!(a.0, b.0);
    assert_eq!(a.0, "index.php?product_id=50&route=product/product");
    let c = id("https://shop.nl/product.php?id=123");
    let d = id(
        "https://shop.nl/product.php?ID=124&utm_source=x&gclid=1&fbclid=2&msclkid=3&source=mail&campaign=z&ref=q",
    );
    assert_ne!(c.0, d.0);
    assert_eq!(d.0, "product.php?id=124");
    assert_eq!(d.1, "https://shop.nl/product.php?ID=124");
    assert_eq!(c.0, id("https://shop.nl/product.php?id=123&utm_medium=m").0);
}

/// Values keep their case (a code may be case-sensitive), keys do not; a
/// bare `?` names nothing; one key twice with two values is two parameters;
/// the same pair twice is one.
#[test]
fn parameters_are_compared_by_value_as_written() {
    let upper = id("https://shop.nl/p/stoel/10043467?SKU=AbC");
    let lower = id("https://shop.nl/p/stoel/10043467?sku=abc");
    assert_eq!(upper.0, "10043467?sku=AbC");
    assert_ne!(upper.0, lower.0);
    let bare = id("https://shop.nl/p/stoel/10043467?");
    assert_eq!(bare.0, "10043467");
    assert_eq!(bare.1, "https://shop.nl/p/stoel/10043467");
    let two = id("https://shop.nl/p/stoel/10043467?maat=l&maat=m");
    assert_eq!(two.0, "10043467?maat=l&maat=m");
    assert_eq!(
        two.0,
        id("https://shop.nl/p/stoel/10043467?maat=m&maat=l").0
    );
    let same = id("https://shop.nl/p/stoel/10043467?maat=l&maat=l");
    assert_eq!(same.0, "10043467?maat=l");
}
