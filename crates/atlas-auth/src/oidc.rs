//! OIDC / OAuth 2.0 authentication (T7.3).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OidcError {
    #[error("discovery failed: {0}")]
    Discovery(String),
    #[error("token exchange failed: {0}")]
    TokenExchange(String),
    #[error("token validation failed: {0}")]
    Validation(String),
    #[error("missing required claim: {0}")]
    MissingClaim(String),
}

/// OIDC provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OidcConfig {
    /// Issuer URL (e.g. `https://accounts.google.com`).
    pub issuer: String,
    /// OAuth client ID registered with the IdP.
    pub client_id: String,
    /// OAuth client secret.
    pub client_secret: String,
    /// Redirect URI registered with the IdP.
    pub redirect_uri: String,
    /// Scopes to request (always includes `openid`).
    pub scopes: Vec<String>,
    /// Claim to use as the ATLAS principal name (`sub`, `email`, etc.).
    pub principal_claim: String,
    /// Claim containing group memberships.
    pub groups_claim: Option<String>,
}

impl OidcConfig {
    pub fn new(
        issuer: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri: redirect_uri.into(),
            scopes: vec!["openid".into(), "email".into(), "profile".into()],
            principal_claim: "sub".into(),
            groups_claim: Some("groups".into()),
        }
    }
}

/// Decoded and verified ID token claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenClaims {
    /// Subject (the unique user identifier from the IdP).
    pub sub: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub groups: Vec<String>,
    /// Expiry as Unix timestamp.
    pub exp: u64,
    pub iss: String,
    pub aud: String,
}

/// Seconds of clock skew to tolerate between the ATLAS server and the IdP.
const CLOCK_LEEWAY_SECS: u64 = 30;

impl TokenClaims {
    /// The ATLAS principal derived from this token.
    pub fn atlas_principal(&self, config: &OidcConfig) -> String {
        match config.principal_claim.as_str() {
            "email" => self.email.clone().unwrap_or_else(|| self.sub.clone()),
            "name" => self.name.clone().unwrap_or_else(|| self.sub.clone()),
            _ => self.sub.clone(),
        }
    }

    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now > self.exp.saturating_add(CLOCK_LEEWAY_SECS)
    }
}

// ---- JWKS types --------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JwksResponse {
    keys: Vec<JwkKey>,
}

#[derive(Debug, Deserialize)]
struct JwkKey {
    #[serde(rename = "kid")]
    key_id: Option<String>,
    #[serde(rename = "kty")]
    key_type: String,
    #[serde(rename = "use")]
    key_use: Option<String>,
    alg: Option<String>,
    n: Option<String>,
    e: Option<String>,
}

// ---- OpenID discovery document -----------------------------------------

#[derive(Debug, Deserialize)]
struct OidcDiscovery {
    token_endpoint: String,
    jwks_uri: String,
}

// ---- Token endpoint response -------------------------------------------

#[derive(Debug, Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
    access_token: Option<String>,
}

// ---- Raw claims for serde deserialization --------------------------------

#[derive(Debug, Deserialize)]
struct RawClaims {
    sub: String,
    email: Option<String>,
    name: Option<String>,
    #[serde(default)]
    groups: Vec<String>,
    exp: u64,
    iss: String,
    #[serde(deserialize_with = "aud_string_or_array")]
    aud: String,
}

fn aud_string_or_array<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    use serde::de::Error;
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Array(a) => a
            .first()
            .and_then(|x| x.as_str())
            .map(str::to_owned)
            .ok_or_else(|| Error::custom("empty aud array")),
        _ => Err(Error::custom("aud must be string or array")),
    }
}

// ---- Public API ---------------------------------------------------------

/// Build the authorization URL to redirect the user to.
pub fn authorization_url(config: &OidcConfig, state: &str) -> String {
    let scopes = config.scopes.join("%20");
    format!(
        "{}/authorize?response_type=code&client_id={}&redirect_uri={}&scope={}&state={}",
        config.issuer,
        config.client_id,
        urlencoded(&config.redirect_uri),
        scopes,
        state,
    )
}

/// Exchange an authorization code for verified token claims.
///
/// Steps:
/// 1. Fetch the OpenID Connect discovery document from `{issuer}/.well-known/openid-configuration`.
/// 2. POST to the `token_endpoint` with the authorization code.
/// 3. Fetch the JWKS from `jwks_uri`.
/// 4. Verify the JWT signature and validate standard claims.
pub fn exchange_code(config: &OidcConfig, code: &str) -> Result<TokenClaims, OidcError> {
    if code.is_empty() {
        return Err(OidcError::TokenExchange("empty code".into()));
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| OidcError::Discovery(e.to_string()))?;

    // 1. Discovery document.
    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        config.issuer.trim_end_matches('/')
    );
    let discovery: OidcDiscovery = client
        .get(&discovery_url)
        .send()
        .map_err(|e| OidcError::Discovery(e.to_string()))?
        .json()
        .map_err(|e| OidcError::Discovery(e.to_string()))?;

    // 2. Token exchange.
    let params = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("client_id", config.client_id.as_str()),
        ("client_secret", config.client_secret.as_str()),
    ];
    let token_resp: TokenResponse = client
        .post(&discovery.token_endpoint)
        .form(&params)
        .send()
        .map_err(|e| OidcError::TokenExchange(e.to_string()))?
        .json()
        .map_err(|e| OidcError::TokenExchange(e.to_string()))?;

    let id_token = token_resp
        .id_token
        .or(token_resp.access_token)
        .ok_or_else(|| OidcError::TokenExchange("no id_token in response".into()))?;

    // 3. Fetch JWKS.
    let jwks: JwksResponse = client
        .get(&discovery.jwks_uri)
        .send()
        .map_err(|e| OidcError::Validation(format!("jwks fetch: {e}")))?
        .json()
        .map_err(|e| OidcError::Validation(format!("jwks parse: {e}")))?;

    // 4. Verify JWT.
    verify_jwt(&id_token, &jwks, config)
}

fn verify_jwt(
    token: &str,
    jwks: &JwksResponse,
    config: &OidcConfig,
) -> Result<TokenClaims, OidcError> {
    use jsonwebtoken::{Algorithm, DecodingKey, Header, Validation};

    // Peek at header to find kid and algorithm.
    let header: Header = jsonwebtoken::decode_header(token)
        .map_err(|e| OidcError::Validation(format!("header decode: {e}")))?;

    // Find matching key from JWKS.
    let jwk = jwks
        .keys
        .iter()
        .find(|k| {
            // Prefer key with matching kid; fall back to first RSA sig key.
            let kid_match = header
                .kid
                .as_ref()
                .and_then(|h_kid| k.key_id.as_ref().map(|k_kid| h_kid == k_kid))
                .unwrap_or(false);
            let use_ok = k.key_use.as_deref().map_or(true, |u| u == "sig");
            let type_ok = k.key_type == "RSA";
            kid_match || (type_ok && use_ok)
        })
        .ok_or_else(|| OidcError::Validation("no matching JWK found".into()))?;

    // Build decoding key from RSA n/e components.
    let n = jwk
        .n
        .as_deref()
        .ok_or_else(|| OidcError::Validation("JWK missing 'n'".into()))?;
    let e = jwk
        .e
        .as_deref()
        .ok_or_else(|| OidcError::Validation("JWK missing 'e'".into()))?;
    let decoding_key = DecodingKey::from_rsa_components(n, e)
        .map_err(|e| OidcError::Validation(format!("decoding key: {e}")))?;

    // Choose algorithm (RS256 default if not specified in JWK/header).
    let alg = match jwk.alg.as_deref().or_else(|| {
        // map jsonwebtoken::Algorithm back to string
        Some(match header.alg {
            Algorithm::RS384 => "RS384",
            Algorithm::RS512 => "RS512",
            _ => "RS256",
        })
    }) {
        Some("RS384") => Algorithm::RS384,
        Some("RS512") => Algorithm::RS512,
        _ => Algorithm::RS256,
    };

    let mut validation = Validation::new(alg);
    validation.set_issuer(&[&config.issuer]);
    validation.set_audience(&[&config.client_id]);
    validation.leeway = CLOCK_LEEWAY_SECS;

    let token_data = jsonwebtoken::decode::<RawClaims>(token, &decoding_key, &validation)
        .map_err(|e| OidcError::Validation(format!("JWT verify: {e}")))?;

    let c = token_data.claims;
    Ok(TokenClaims {
        sub: c.sub,
        email: c.email,
        name: c.name,
        groups: c.groups,
        exp: c.exp,
        iss: c.iss,
        aud: c.aud,
    })
}

fn urlencoded(s: &str) -> String {
    s.replace(':', "%3A").replace('/', "%2F")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OidcConfig {
        OidcConfig::new(
            "https://idp.example.com",
            "client-id",
            "secret",
            "https://atlas/callback",
        )
    }

    #[test]
    fn authorization_url_contains_client_id() {
        let url = authorization_url(&cfg(), "random-state");
        assert!(url.contains("client-id"));
        assert!(url.contains("random-state"));
    }

    #[test]
    fn exchange_empty_code_errors() {
        assert!(exchange_code(&cfg(), "").is_err());
    }

    #[test]
    fn is_expired_with_past_timestamp() {
        let claims = TokenClaims {
            sub: "u".into(),
            email: None,
            name: None,
            groups: vec![],
            exp: 1,
            iss: "iss".into(),
            aud: "aud".into(),
        };
        assert!(claims.is_expired());
    }

    #[test]
    fn atlas_principal_uses_email_when_configured() {
        let mut cfg = cfg();
        cfg.principal_claim = "email".into();
        let claims = TokenClaims {
            sub: "sub-id".into(),
            email: Some("user@example.com".into()),
            name: None,
            groups: vec![],
            exp: u64::MAX,
            iss: cfg.issuer.clone(),
            aud: cfg.client_id.clone(),
        };
        assert_eq!(claims.atlas_principal(&cfg), "user@example.com");
    }
}
