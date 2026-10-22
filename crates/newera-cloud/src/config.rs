//! What the service needs to know about where it runs, from the environment.

use anyhow::Context as _;

/// How sign-in links reach people.
#[derive(Clone)]
pub enum Mail {
    /// Through Resend's HTTP API.
    Resend { api_key: String, from: String },
    /// Into the log — for a machine that is only being tried out. Never in
    /// production: whoever reads the log could sign in as anyone.
    Log,
    /// Not set up yet: sign-in by email is off, and everything else — the
    /// relay above all — keeps working.
    Off,
}

/// Where Google trades a sign-in's code for an access token.
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Where Google says whose the access token is.
pub const GOOGLE_USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";

/// Google as a way to sign in, when configured.
#[derive(Clone)]
pub struct Google {
    pub client_id: String,
    pub client_secret: String,
    /// [`GOOGLE_TOKEN_URL`], or a stand-in for it in the tests.
    pub token_url: String,
    /// [`GOOGLE_USERINFO_URL`], or a stand-in for it in the tests.
    pub userinfo_url: String,
}

// Secrets stay out of logs and panics.
impl std::fmt::Debug for Mail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Resend { from, .. } => {
                write!(f, "Resend {{ from: {from:?}, api_key: <hidden> }}")
            }
            Self::Log => f.write_str("Log"),
            Self::Off => f.write_str("Off"),
        }
    }
}

impl std::fmt::Debug for Google {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Google {{ client_id: {:?}, client_secret: <hidden> }}",
            self.client_id
        )
    }
}

/// Buy Me a Coffee, where the paid plan is supported: it sells the
/// membership on a page of its own and pays out through the Stripe account
/// connected to it. This service only points people at that page and listens
/// to what it says afterwards.
#[derive(Clone)]
pub struct BuyMeACoffee {
    /// The page's name, the part after `buymeacoffee.com/`.
    pub page: String,
    /// The webhook's signing secret: only deliveries signed with it change a
    /// plan.
    pub webhook_secret: String,
    /// The membership levels that pay for the plan, by id. Empty means every
    /// level does, which is right for a page that sells only this.
    pub levels: Vec<i64>,
    /// Whether a test event from the dashboard (`live_mode: false`) may
    /// change a plan. Off in production, where only real money should.
    pub test_events: bool,
}

impl BuyMeACoffee {
    /// Reads `BUYMEACOFFEE_*`: both the page and the webhook secret, or
    /// neither (the paid plan off).
    fn from_env(var: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Option<Self>> {
        let required = ["BUYMEACOFFEE_PAGE", "BUYMEACOFFEE_WEBHOOK_SECRET"];
        let values: Vec<Option<String>> = required.iter().map(|name| var(name)).collect();
        if values.iter().all(Option::is_none) {
            tracing::warn!("no BUYMEACOFFEE_* settings: the paid plan is off");
            return Ok(None);
        }
        let missing: Vec<&str> = required
            .iter()
            .zip(&values)
            .filter(|(_, v)| v.is_none())
            .map(|(name, _)| *name)
            .collect();
        anyhow::ensure!(missing.is_empty(), "missing {}", missing.join(", "));
        let mut values = values.into_iter().map(Option::unwrap_or_default);
        let mut next = || values.next().unwrap_or_default();
        // A whole URL where a name was asked for is the easy mistake to make.
        let page = next()
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.")
            .trim_start_matches("buymeacoffee.com/")
            .trim_matches('/')
            .to_owned();
        anyhow::ensure!(!page.is_empty(), "BUYMEACOFFEE_PAGE is empty");
        let webhook_secret = next();
        let levels = var("BUYMEACOFFEE_LEVELS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| l.parse::<i64>().context("BUYMEACOFFEE_LEVELS"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Some(Self {
            page,
            webhook_secret,
            levels,
            test_events: matches!(
                var("BUYMEACOFFEE_TEST_EVENTS").as_deref(),
                Some("1" | "true")
            ),
        }))
    }
}

impl std::fmt::Debug for BuyMeACoffee {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuyMeACoffee")
            .field("page", &self.page)
            .field("webhook_secret", &"<hidden>")
            .field("levels", &self.levels)
            .field("test_events", &self.test_events)
            .finish()
    }
}

/// The 3D generator a furniture product's model is made by from its photo:
/// the service in `deploy/spark-generator/`, on a host of the private
/// network (the tailnet), never the public internet.
#[derive(Clone)]
pub struct Generator {
    /// Where it answers, without a trailing slash, e.g. `http://100.76.21.14:7980`.
    pub url: String,
    /// Sent as `X-Generator-Token`.
    pub token: String,
    /// How often a job in flight is asked after.
    pub poll: std::time::Duration,
    /// The wait before a failed job's first retry; each next one waits twice
    /// as long.
    pub backoff: std::time::Duration,
    /// How long the generator may be running a job without an answer
    /// before it is given again; a job waiting its turn is never.
    pub stall: std::time::Duration,
}

/// How often a job in flight is asked after: a generation takes about two
/// minutes (`deploy/spark-generator/`), so a model arrives within this of
/// being made.
pub const GENERATOR_POLL: std::time::Duration = std::time::Duration::from_secs(10);
/// How long the generator may be running a job without an answer: it fails
/// a job itself after 15 minutes (`deploy/spark-generator/server.py`) and
/// restarts; past this, the job is given again. A chosen bound.
pub const GENERATOR_STALL: std::time::Duration = std::time::Duration::from_mins(30);
/// The wait before a failed job's first retry. A chosen bound: with
/// `generation::ATTEMPTS` it rides out a restart of the generator's host.
pub const GENERATOR_BACKOFF: std::time::Duration = std::time::Duration::from_secs(60);

impl Generator {
    /// Reads `NEWERA_GENERATOR_URL` and `NEWERA_GENERATOR_TOKEN`: both, or
    /// neither (generation off).
    fn from_env(var: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Option<Self>> {
        match (var("NEWERA_GENERATOR_URL"), var("NEWERA_GENERATOR_TOKEN")) {
            (None, None) => {
                tracing::warn!("no NEWERA_GENERATOR_URL: furniture models are not generated");
                Ok(None)
            }
            (Some(url), Some(token)) => {
                let url = url.trim().trim_end_matches('/').to_owned();
                let token = token.trim().to_owned();
                anyhow::ensure!(!token.is_empty(), "NEWERA_GENERATOR_TOKEN is empty");
                anyhow::ensure!(
                    url.starts_with("http://") || url.starts_with("https://"),
                    "NEWERA_GENERATOR_URL is an http(s) address"
                );
                Ok(Some(Self {
                    url,
                    token,
                    poll: GENERATOR_POLL,
                    backoff: GENERATOR_BACKOFF,
                    stall: GENERATOR_STALL,
                }))
            }
            (Some(_), None) => anyhow::bail!("missing NEWERA_GENERATOR_TOKEN"),
            (None, Some(_)) => anyhow::bail!("missing NEWERA_GENERATOR_URL"),
        }
    }
}

impl std::fmt::Debug for Generator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Generator")
            .field("url", &self.url)
            .field("token", &"<hidden>")
            .field("poll", &self.poll)
            .field("backoff", &self.backoff)
            .field("stall", &self.stall)
            .finish()
    }
}

/// Who may hold an account here, from `NEWERA_ALLOWED_EMAILS`: anyone, or
/// only the addresses listed — an installation that is someone's own. The
/// listed addresses are its owners, on the `owner` plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedEmails(Option<Vec<String>>);

impl AllowedEmails {
    /// Reads a comma-separated list; `None` lets anyone in.
    ///
    /// # Errors
    ///
    /// When the list is given but names no address: that would let nobody
    /// in, which is never what was meant.
    pub fn parse(list: Option<&str>) -> anyhow::Result<Self> {
        let Some(list) = list else {
            return Ok(Self(None));
        };
        let emails: Vec<String> = list
            .split(',')
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        anyhow::ensure!(!emails.is_empty(), "NEWERA_ALLOWED_EMAILS names no address");
        Ok(Self(Some(emails)))
    }

    /// Reads the variable as the environment gave it (the seam under
    /// `Config::from_env`).
    ///
    /// # Errors
    ///
    /// When the variable is present but names no address, or is not text.
    pub fn from_var(value: Result<String, std::env::VarError>) -> anyhow::Result<Self> {
        match value {
            Ok(list) => Self::parse(Some(&list)),
            Err(std::env::VarError::NotPresent) => Self::parse(None),
            Err(err) => Err(anyhow::Error::new(err).context("NEWERA_ALLOWED_EMAILS")),
        }
    }

    /// The addresses listed, lower-cased; `None` when anyone is let in.
    pub fn list(&self) -> Option<&[String]> {
        self.0.as_deref()
    }

    /// Whether this address may sign in and hold an account.
    pub fn admits(&self, email: &str) -> bool {
        self.0.is_none() || self.lists(email)
    }

    /// Whether this address is one of those listed.
    pub fn lists(&self, email: &str) -> bool {
        self.0
            .as_ref()
            .is_some_and(|list| list.contains(&email.trim().to_lowercase()))
    }
}

#[derive(Clone)]
pub struct Config {
    /// Where this service is reached, without a trailing slash, e.g.
    /// `https://mcp.3dneweraai.com`. The issuer, and the base of every URL
    /// it hands out.
    pub public_url: String,
    pub database_url: String,
    /// Pages allowed to call the account endpoints with the person's cookie:
    /// the editor at 3dneweraai.com.
    pub site_origins: Vec<String>,
    pub mail: Mail,
    pub google: Option<Google>,
    /// The browser editor, where "open in the editor" goes.
    pub editor_url: String,
    /// The paid plan, supported through Buy Me a Coffee; `None` leaves every
    /// account on the free plan and the account page without a way to pay.
    pub buymeacoffee: Option<BuyMeACoffee>,
    /// Who may sign in; by default, anyone.
    pub allowed_emails: AllowedEmails,
    /// The port to listen on, on every interface of the container; the
    /// Cloudflare tunnel in front is what reaches it.
    pub port: u16,
    /// `AnySite`, where shop products are read, from `ANYSITE_API_KEY` (and
    /// `ANYSITE_BASE_URL`, the CLI's name, for a stand-in); `None` leaves
    /// product import off and everything else working.
    pub anysite: Option<newera_shop::AnySite>,
    /// The 3D generator; `None` leaves imported furniture on its placeholder
    /// until a model is attached.
    pub generator: Option<Generator>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("public_url", &self.public_url)
            .field("database_url", &"<hidden>")
            .field("site_origins", &self.site_origins)
            .field("mail", &self.mail)
            .field("google", &self.google)
            .field("editor_url", &self.editor_url)
            .field("buymeacoffee", &self.buymeacoffee)
            .field("allowed_emails", &self.allowed_emails)
            .field("port", &self.port)
            .field("anysite", &self.anysite)
            .field("generator", &self.generator)
            .finish()
    }
}

impl Config {
    /// Reads the configuration.
    ///
    /// # Errors
    ///
    /// When a required variable is missing or malformed.
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let public_url = var("NEWERA_PUBLIC_URL")
            .unwrap_or_else(|| "https://mcp.3dneweraai.com".to_owned())
            .trim_end_matches('/')
            .to_owned();
        let database_url = var("DATABASE_URL").context("DATABASE_URL is required")?;
        let site_origins = var("NEWERA_SITE_ORIGINS")
            .unwrap_or_else(|| "https://3dneweraai.com,https://www.3dneweraai.com".to_owned())
            .split(',')
            .map(|o| o.trim().trim_end_matches('/').to_owned())
            .filter(|o| !o.is_empty())
            .collect();
        let mail = match (var("RESEND_API_KEY"), var("NEWERA_MAIL")) {
            (Some(api_key), _) => Mail::Resend {
                api_key,
                from: var("NEWERA_MAIL_FROM")
                    .unwrap_or_else(|| "3D New Era AI <entrar@3dneweraai.com>".to_owned()),
            },
            (None, Some(mode)) if mode == "log" => Mail::Log,
            _ => {
                tracing::warn!(
                    "no RESEND_API_KEY: sign-in by email is off (NEWERA_MAIL=log prints links, for trying it out)"
                );
                Mail::Off
            }
        };
        let google = match (var("GOOGLE_CLIENT_ID"), var("GOOGLE_CLIENT_SECRET")) {
            (Some(client_id), Some(client_secret)) => Some(Google {
                client_id,
                client_secret,
                token_url: GOOGLE_TOKEN_URL.to_owned(),
                userinfo_url: GOOGLE_USERINFO_URL.to_owned(),
            }),
            _ => None,
        };
        let editor_url =
            var("NEWERA_EDITOR_URL").unwrap_or_else(|| "https://3dneweraai.com/app/".to_owned());
        let buymeacoffee = BuyMeACoffee::from_env(&var)?;
        let allowed_emails = AllowedEmails::from_var(std::env::var("NEWERA_ALLOWED_EMAILS"))?;
        let anysite = var("ANYSITE_API_KEY").map(|key| {
            let base = var("ANYSITE_BASE_URL")
                .unwrap_or_else(|| newera_shop::anysite::BASE_URL.to_owned());
            newera_shop::AnySite::new(&base, key.trim())
        });
        if anysite.is_none() {
            tracing::warn!("no ANYSITE_API_KEY: product import is off");
        }
        let generator = Generator::from_env(&var)?;
        let port = var("PORT")
            .map(|p| p.parse().context("PORT"))
            .transpose()?
            .unwrap_or(7979);
        Ok(Self {
            public_url,
            database_url,
            site_origins,
            mail,
            google,
            editor_url,
            buymeacoffee,
            allowed_emails,
            port,
            anysite,
            generator,
        })
    }

    /// Cookies go over HTTPS only, unless this runs on a plain-HTTP address
    /// for local testing.
    pub fn secure_cookies(&self) -> bool {
        self.public_url.starts_with("https://")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_trimmed_and_ignores_case() {
        let allowed = AllowedEmails::parse(Some(" Ana@Example.com ,bo@example.com,")).unwrap();
        assert_eq!(
            allowed,
            AllowedEmails(Some(vec![
                "ana@example.com".to_owned(),
                "bo@example.com".to_owned()
            ]))
        );
        assert!(allowed.admits(" ANA@example.COM"));
        assert!(allowed.lists("bo@EXAMPLE.com"));
        assert!(!allowed.admits("cy@example.com"));
        assert!(!allowed.lists("cy@example.com"));
    }

    #[test]
    fn no_list_lets_anyone_in_and_lists_nobody() {
        let anyone = AllowedEmails::parse(None).unwrap();
        assert!(anyone.admits("cy@example.com"));
        assert!(!anyone.lists("cy@example.com"));
    }

    #[test]
    fn a_variable_present_but_naming_no_address_is_refused() {
        use std::env::VarError;
        for blank in ["", "   ", " , ,", ","] {
            assert!(
                AllowedEmails::from_var(Ok(blank.to_owned())).is_err(),
                "{blank:?}"
            );
        }
        assert!(AllowedEmails::from_var(Err(VarError::NotUnicode("x".into()))).is_err());
        assert_eq!(
            AllowedEmails::from_var(Err(VarError::NotPresent)).unwrap(),
            AllowedEmails::default()
        );
        assert!(
            AllowedEmails::from_var(Ok("a@b.co".to_owned()))
                .unwrap()
                .lists("a@b.co")
        );
    }

    #[test]
    fn the_generator_takes_its_address_and_token_together() {
        let env = |given: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                given
                    .iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert!(Generator::from_env(&env(&[])).unwrap().is_none());
        let both = Generator::from_env(&env(&[
            ("NEWERA_GENERATOR_URL", " http://100.76.21.14:7980/ "),
            ("NEWERA_GENERATOR_TOKEN", " t0k "),
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(
            (
                both.url.as_str(),
                both.token.as_str(),
                both.poll,
                both.backoff
            ),
            (
                "http://100.76.21.14:7980",
                "t0k",
                GENERATOR_POLL,
                GENERATOR_BACKOFF
            )
        );
        assert!(!format!("{both:?}").contains("t0k"), "the token is hidden");
        assert_eq!(both.stall, GENERATOR_STALL);
        for half in [
            &[("NEWERA_GENERATOR_URL", "http://h:1")][..],
            &[("NEWERA_GENERATOR_TOKEN", "t")][..],
            &[
                ("NEWERA_GENERATOR_URL", "h:1"),
                ("NEWERA_GENERATOR_TOKEN", "t"),
            ][..],
            &[
                ("NEWERA_GENERATOR_URL", "http://h:1"),
                ("NEWERA_GENERATOR_TOKEN", "  "),
            ][..],
        ] {
            assert!(Generator::from_env(&env(half)).is_err(), "{half:?}");
        }
    }

    #[test]
    fn a_list_naming_no_address_is_refused() {
        assert!(AllowedEmails::parse(Some(" , ")).is_err());
    }
}
