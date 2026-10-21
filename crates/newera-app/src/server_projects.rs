//! The account's projects on the service the browser editor runs against:
//! what the service answers when they are listed or one is made, and the
//! addresses a project is fetched from and the editor shows it at. The
//! requests are `ai_web`'s; the window's are `dialogs`'.

/// One of the account's projects, as the service lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub(crate) struct ServerProject {
    pub(crate) id: String,
    pub(crate) name: String,
    /// When it last changed, in seconds since the epoch.
    pub(crate) updated_at: i64,
}

/// The projects in the service's answer to `GET /account/projects`, in the
/// order it gives them (the last changed first).
pub(crate) fn parse_list(text: &str) -> Result<Vec<ServerProject>, String> {
    #[derive(serde::Deserialize)]
    struct Listed {
        projects: Vec<ServerProject>,
    }
    serde_json::from_str::<Listed>(text)
        .map(|listed| listed.projects)
        .map_err(|e| crate::i18n::fill("O serviço respondeu algo inesperado: {}", &[&e]))
}

/// What `POST /account/projects` is sent to make a project named `name`.
pub(crate) fn new_body(name: &str) -> String {
    serde_json::json!({"name": name.trim()}).to_string()
}

/// The id and the name of the project the service made, from its answer.
pub(crate) fn parse_made(text: &str) -> Result<(String, String), String> {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|answer| {
            Some((
                answer["id"].as_str()?.to_owned(),
                answer["name"].as_str()?.to_owned(),
            ))
        })
        .ok_or_else(|| crate::i18n::tr("O serviço respondeu algo inesperado").to_owned())
}

/// The file name a download's `Content-Disposition` gives: the one as
/// written (RFC 5987 `filename*=UTF-8''…`) when it is there, else the plain
/// `filename="…"`.
pub(crate) fn file_name(disposition: &str) -> Option<String> {
    let written = disposition.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        if !key.trim().eq_ignore_ascii_case("filename*") {
            return None;
        }
        let (charset, rest) = value.trim().split_once('\'')?;
        let (_, encoded) = rest.split_once('\'')?;
        if charset.eq_ignore_ascii_case("utf-8") {
            decode(encoded)
        } else {
            None
        }
    });
    written.or_else(|| {
        disposition
            .split("filename=\"")
            .nth(1)
            .and_then(|n| n.split('"').next())
            .map(str::to_owned)
    })
}

/// Percent-encoded UTF-8, decoded; `None` when it is not that.
fn decode(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = std::str::from_utf8(bytes.get(at + 1..at + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Why the service refused, as it said in its answer, or its status.
pub(crate) fn refusal(status: u16, text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|answer| answer["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| crate::i18n::fill("O serviço respondeu {}", &[&status]))
}

/// Where the service keeps the project `id`: the address the tab fetches it
/// from and saves it back to, as a `?project=` link names it.
pub(crate) fn project_url(service: &str, id: &str) -> String {
    format!(
        "{}/account/projects/{}",
        service.trim_end_matches('/'),
        encode(id)
    )
}

/// Where the tab says it shows the project at `project`, which makes it the
/// account's active one on the service.
pub(crate) fn active_url(project: &str) -> String {
    let bare = project.split(['?', '#']).next().unwrap_or(project);
    format!("{}/active", bare.trim_end_matches('/'))
}

/// The editor's address query for the project at `project`, kept at
/// `service`: the one `show_plan` and the account page link to, so a reload
/// opens the same project, bound to the service again.
pub(crate) fn editor_query(project: &str, service: &str) -> String {
    format!("?project={}&relay={}", encode(project), encode(service))
}

/// The editor's address query with only the service: the editor bound to
/// no project of it.
pub(crate) fn service_query(service: &str) -> String {
    format!("?relay={}", encode(service))
}

/// `text` as one component of an address, as the browser's
/// `encodeURIComponent` writes it.
fn encode(text: &str) -> String {
    use std::fmt::Write as _;
    text.bytes().fold(String::new(), |mut out, byte| {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ServerProject, active_url, editor_query, file_name, new_body, parse_list, parse_made,
        project_url, refusal,
    };

    #[test]
    fn a_file_name_is_read_as_written_where_the_service_gives_it() {
        assert_eq!(
            file_name(
                r#"attachment; filename="Escrit_rio.newera"; filename*=UTF-8''Escrit%C3%B3rio.newera"#
            ),
            Some("Escritório.newera".to_owned())
        );
        assert_eq!(
            file_name(r#"attachment; filename="Casa.newera""#),
            Some("Casa.newera".to_owned())
        );
        // Not UTF-8, or not decodable: the plain name.
        for odd in [
            r#"attachment; filename="A_o.newera"; filename*=ISO-8859-1''A%E7o.newera"#,
            r#"attachment; filename="A_o.newera"; filename*=UTF-8''A%E7o.newera"#,
            r#"attachment; filename="A_o.newera"; filename*=UTF-8''A%Zo.newera"#,
        ] {
            assert_eq!(file_name(odd), Some("A_o.newera".to_owned()), "{odd}");
        }
        assert_eq!(file_name("attachment"), None);
    }

    #[test]
    fn an_address_with_no_project_names_the_service_alone() {
        assert_eq!(
            super::service_query("https://interioragent.dutch-atlas.com"),
            "?relay=https%3A%2F%2Finterioragent.dutch-atlas.com"
        );
    }

    #[test]
    fn the_project_on_screen_is_made_active_at_its_own_address() {
        assert_eq!(
            active_url("https://svc.example/account/projects/ab12"),
            "https://svc.example/account/projects/ab12/active"
        );
        assert_eq!(
            active_url("https://svc.example/account/projects/ab12?x=1#top"),
            "https://svc.example/account/projects/ab12/active"
        );
    }

    #[test]
    fn a_list_is_read_in_the_order_the_service_gives() {
        let text = r#"{"projects": [
            {"id": "b2", "name": "Casa nova", "size": 2048, "updated_at": 1700000500, "active": false},
            {"id": "a1", "name": "Apê \"centro\"", "size": 7, "updated_at": 1700000200, "active": true}
        ]}"#;
        assert_eq!(
            parse_list(text),
            Ok(vec![
                ServerProject {
                    id: "b2".into(),
                    name: "Casa nova".into(),
                    updated_at: 1_700_000_500,
                },
                ServerProject {
                    id: "a1".into(),
                    name: "Apê \"centro\"".into(),
                    updated_at: 1_700_000_200,
                },
            ])
        );
        assert_eq!(parse_list(r#"{"projects": []}"#), Ok(vec![]));
    }

    #[test]
    fn a_list_that_is_not_one_says_so() {
        for odd in ["", "[]", r#"{"projects": [{"id": "x"}]}"#, "<html>"] {
            let read = parse_list(odd);
            assert!(
                read.as_ref()
                    .is_err_and(|why| why.starts_with("O serviço respondeu algo inesperado: ")),
                "{odd:?}: {read:?}"
            );
        }
    }

    #[test]
    fn a_new_project_is_asked_for_by_its_name() {
        assert_eq!(
            new_body("  Casa \"praia\" "),
            r#"{"name":"Casa \"praia\""}"#
        );
    }

    #[test]
    fn the_made_project_is_read_by_its_id() {
        assert_eq!(
            parse_made(r#"{"id": "c3", "name": "Escritório", "revision": 0}"#),
            Ok(("c3".to_owned(), "Escritório".to_owned()))
        );
        for odd in [r#"{"error": "no"}"#, r#"{"id": "c3"}"#, "oops"] {
            assert_eq!(
                parse_made(odd),
                Err("O serviço respondeu algo inesperado".to_owned()),
                "{odd}"
            );
        }
    }

    #[test]
    fn a_refusal_is_what_the_service_said_or_its_status() {
        assert_eq!(
            refusal(409, r#"{"error": "there is a project named Casa already"}"#),
            "there is a project named Casa already"
        );
        assert_eq!(
            refusal(502, "<html>Bad gateway</html>"),
            "O serviço respondeu 502"
        );
        assert_eq!(refusal(500, ""), "O serviço respondeu 500");
    }

    #[test]
    fn a_project_is_fetched_where_a_link_to_it_points() {
        assert_eq!(
            project_url("https://svc.example", "ab12"),
            "https://svc.example/account/projects/ab12"
        );
        assert_eq!(
            project_url("https://svc.example/", "a b/c"),
            "https://svc.example/account/projects/a%20b%2Fc"
        );
    }

    #[test]
    fn the_address_is_the_one_the_service_links_to() {
        // As `newera-cloud`'s `engine::editor_link` writes it.
        assert_eq!(
            editor_query(
                "https://interioragent.dutch-atlas.com/account/projects/ab12",
                "https://interioragent.dutch-atlas.com"
            ),
            "?project=https%3A%2F%2Finterioragent.dutch-atlas.com%2Faccount%2Fprojects%2Fab12&relay=https%3A%2F%2Finterioragent.dutch-atlas.com"
        );
        assert_eq!(
            editor_query(
                "http://127.0.0.1:7979/account/projects/x",
                "http://127.0.0.1:7979"
            ),
            "?project=http%3A%2F%2F127.0.0.1%3A7979%2Faccount%2Fprojects%2Fx&relay=http%3A%2F%2F127.0.0.1%3A7979"
        );
    }
}
