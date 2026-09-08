//! Boot configuration, read once from the environment.
//!
//! The load-bearing rule here is that **OIDC is all-four-or-nothing**. A client secret is issued
//! by a *running* kanidm, so on a fresh fleet install there is a window where the other three
//! variables can be composed and that one cannot: a present-but-empty `OIDC_CLIENT_SECRET` must
//! therefore yield `None` — the service stays up and closed — rather than a half-configured client
//! that fails at the first login. (`../keel` deploys nib; a sealed environment there fails by name
//! until the secret is in the vault, so this is the safety net under that bootstrap, not a stage
//! of it.)

use std::path::PathBuf;

/// The four env vars that make up a usable OIDC client. Absent (or any one empty) = not configured.
#[derive(Clone)]
pub struct OidcSettings {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
}

impl OidcSettings {
    /// All four or `None` — the `?` on each `filter` short-circuits the whole `Option`.
    fn from_env() -> Option<Self> {
        let issuer = env_nonempty("OIDC_ISSUER")?;
        let client_id = env_nonempty("OIDC_CLIENT_ID")?;
        let client_secret = env_nonempty("OIDC_CLIENT_SECRET")?;
        let redirect_url = env_nonempty("OIDC_REDIRECT_URL")?;
        Some(Self {
            issuer,
            client_id,
            client_secret,
            redirect_url,
        })
    }
}

pub struct Config {
    pub db_url: String,
    pub dist: PathBuf,
    pub port: u16,
    /// Local-dev bypass: synthesize an identity instead of requiring a login. Never set in prod.
    pub dev_auth: bool,
    /// Token seeded for the dev user under `dev_auth` — lets `just dev` + a local MCP client work
    /// with zero setup. Not seeded at all in production; real users mint their own on first login.
    pub dev_token: String,
    /// Hex, ≥64 bytes decoded — the signing key for the session cookie.
    pub session_key: String,
    pub oidc: Option<OidcSettings>,
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let dev_auth = env_nonempty("NIB_DEV_AUTH").is_some_and(|v| v != "0");
        Ok(Self {
            db_url: env_nonempty("NIB_DB").unwrap_or_else(|| "sqlite:nib.db".to_string()),
            dist: PathBuf::from(
                env_nonempty("NIB_DIST").unwrap_or_else(|| "../frontend/dist".to_string()),
            ),
            port: env_nonempty("NIB_PORT")
                .and_then(|p| p.parse().ok())
                .unwrap_or(4321),
            dev_auth,
            dev_token: env_nonempty("NIB_DEV_TOKEN")
                .unwrap_or_else(|| crate::db::DEV_TOKEN_DEFAULT.to_string()),
            session_key: resolve_session_key(dev_auth)?,
            oidc: OidcSettings::from_env(),
        })
    }
}

/// Fail closed in production, random-ephemeral under dev auth.
///
/// A missing key in prod is not recoverable by guessing: every session cookie the process signs
/// would be invalidated on the next restart, silently logging everyone out in a loop. Better to
/// refuse to boot and let the deploy show the error.
fn resolve_session_key(dev_auth: bool) -> Result<String, String> {
    match env_nonempty("SESSION_KEY") {
        Some(k) => {
            let decoded = hex::decode(&k)
                .map_err(|_| "SESSION_KEY must be hex (128 chars = 64 bytes)".to_string())?;
            if decoded.len() < 64 {
                return Err(format!(
                    "SESSION_KEY too short: {} bytes decoded, need >=64 (128 hex chars). \
                     Generate one with `openssl rand -hex 64`",
                    decoded.len()
                ));
            }
            Ok(k)
        }
        None if dev_auth => {
            tracing::warn!(
                "SESSION_KEY unset; using a random ephemeral key (dev auth only). Sessions drop on restart."
            );
            Ok(random_hex(64))
        }
        None => Err("SESSION_KEY is required when NIB_DEV_AUTH is off. \
                     Generate one with `openssl rand -hex 64`"
            .to_string()),
    }
}

/// `n` random bytes, hex-encoded.
pub fn random_hex(n: usize) -> String {
    use rand::Rng;
    let mut bytes = vec![0u8; n];
    rand::rng().fill(&mut bytes[..]);
    hex::encode(bytes)
}
