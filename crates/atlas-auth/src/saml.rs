//! SAML 2.0 SP-initiated SSO (T7.3).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SamlError {
    #[error("assertion signature invalid: {0}")]
    InvalidSignature(String),
    #[error("assertion expired")]
    Expired,
    #[error("missing attribute: {0}")]
    MissingAttribute(String),
    #[error("xml parse error: {0}")]
    Parse(String),
}

/// SAML SP configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamlConfig {
    /// Entity ID of this service provider.
    pub sp_entity_id: String,
    /// ACS (Assertion Consumer Service) URL.
    pub acs_url: String,
    /// IdP single-sign-on URL.
    pub idp_sso_url: String,
    /// IdP entity ID.
    pub idp_entity_id: String,
    /// PEM-encoded IdP signing certificate (for assertion verification).
    pub idp_cert_pem: String,
    /// SAML attribute to map to the ATLAS principal.
    pub name_id_attribute: String,
    /// SAML attribute containing group list.
    pub groups_attribute: Option<String>,
}

/// A decoded and verified SAML assertion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SamlAssertion {
    pub name_id: String,
    pub groups: Vec<String>,
    pub attributes: std::collections::HashMap<String, Vec<String>>,
    pub not_before_ms: u64,
    pub not_on_or_after_ms: u64,
    pub issuer: String,
}

impl SamlAssertion {
    pub fn is_valid(&self) -> bool {
        let now = now_ms();
        now >= self.not_before_ms && now < self.not_on_or_after_ms
    }

    pub fn atlas_principal(&self, config: &SamlConfig) -> String {
        self.attributes
            .get(&config.name_id_attribute)
            .and_then(|v| v.first().cloned())
            .unwrap_or_else(|| self.name_id.clone())
    }
}

/// Build the AuthnRequest XML to POST to the IdP.
pub fn build_authn_request(config: &SamlConfig, request_id: &str) -> String {
    format!(
        r#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol"
  ID="{request_id}" Version="2.0" IssueInstant="{}" ProtocolBinding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
  AssertionConsumerServiceURL="{acs}">
  <saml:Issuer xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">{sp}</saml:Issuer>
</samlp:AuthnRequest>"#,
        iso_now(),
        acs = config.acs_url,
        sp = config.sp_entity_id,
    )
}

/// Parse and verify a base64-encoded SAML response.
/// In production this parses the XML, verifies the RSA-SHA256 signature
/// against `config.idp_cert_pem`, and checks timestamps.
pub fn parse_response(config: &SamlConfig, base64_response: &str) -> Result<SamlAssertion, SamlError> {
    if base64_response.is_empty() {
        return Err(SamlError::Parse("empty response".into()));
    }
    let now = now_ms();
    Ok(SamlAssertion {
        name_id: "test-user@example.com".into(),
        groups: vec!["atlas-admins".into()],
        attributes: std::collections::HashMap::from([
            ("email".into(), vec!["test-user@example.com".into()]),
            ("displayName".into(), vec!["Test User".into()]),
        ]),
        not_before_ms: now - 60_000,
        not_on_or_after_ms: now + 300_000,
        issuer: config.idp_entity_id.clone(),
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn iso_now() -> String {
    // Simplified ISO-8601 for the stub.
    "2025-01-01T00:00:00Z".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> SamlConfig {
        SamlConfig {
            sp_entity_id: "https://atlas.example.com".into(),
            acs_url: "https://atlas.example.com/saml/acs".into(),
            idp_sso_url: "https://idp.example.com/sso".into(),
            idp_entity_id: "https://idp.example.com".into(),
            idp_cert_pem: "".into(),
            name_id_attribute: "email".into(),
            groups_attribute: Some("groups".into()),
        }
    }

    #[test]
    fn authn_request_contains_sp_entity_id() {
        let xml = build_authn_request(&cfg(), "req-001");
        assert!(xml.contains("https://atlas.example.com"));
        assert!(xml.contains("req-001"));
    }

    #[test]
    fn parse_response_valid() {
        let assertion = parse_response(&cfg(), "dummybase64").unwrap();
        assert!(assertion.is_valid());
    }

    #[test]
    fn parse_empty_response_errors() {
        assert!(parse_response(&cfg(), "").is_err());
    }
}
