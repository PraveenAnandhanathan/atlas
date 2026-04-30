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
    pub fn new(issuer: impl Into<String>, client_id: impl Into<String>, client_secret: impl Into<String>, redirect_uri: impl Into<String>) -> Self {
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
            "name"  => self.name.clone().unwrap_or_else(|| self.sub.clone()),
            _       => self.sub.clone(),
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

/// Exchange an authorization code for token claims.
/// In production this sends an HTTP POST to the token endpoint and
/// verifies the JWT signature against the IdP's JWKS.
pub fn exchange_code(config: &OidcConfig, code: &str) -> Result<TokenClaims, OidcError> {
    if code.is_empty() {
        return Err(OidcError::TokenExchange("empty code".into()));
    }
    // Stub: return a synthetic token for testing.
    Ok(TokenClaims {
        sub: format!("stub-{code}"),
        email: Some(format!("{code}@example.com")),
        name: Some("Test User".into()),
        groups: vec!["atlas-users".into()],
        exp: u64::MAX,
        iss: config.issuer.clone(),
        aud: config.client_id.clone(),
    })
}

fn urlencoded(s: &str) -> String {
    s.replace(':', "%3A").replace('/', "%2F")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> OidcConfig {
        OidcConfig::new("https://idp.example.com", "client-id", "secret", "https://atlas/callback")
    }

    #[test]
    fn authorization_url_contains_client_id() {
        let url = authorization_url(&cfg(), "random-state");
        assert!(url.contains("client-id"));
        assert!(url.contains("random-state"));
    }

    #[test]
    fn exchange_code_returns_claims() {
        let claims = exchange_code(&cfg(), "abc123").unwrap();
        assert_eq!(claims.sub, "stub-abc123");
        assert!(!claims.is_expired());
    }

    #[test]
    fn exchange_empty_code_errors() {
        assert!(exchange_code(&cfg(), "").is_err());
    }

    #[test]
    fn atlas_principal_uses_sub_by_default() {
        let cfg = cfg();
        let claims = exchange_code(&cfg, "xyz").unwrap();
        let principal = claims.atlas_principal(&cfg);
        assert_eq!(principal, "stub-xyz");
    }
}
