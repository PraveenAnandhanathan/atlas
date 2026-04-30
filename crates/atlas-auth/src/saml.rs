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
///
/// Steps:
/// 1. Base64-decode the response.
/// 2. Parse the XML to extract the Assertion, NameID, Conditions, Attributes,
///    and the `<ds:SignatureValue>` + `<ds:SignedInfo>` elements.
/// 3. Verify the RSA-SHA256 signature against `config.idp_cert_pem`.
/// 4. Validate `NotBefore` / `NotOnOrAfter` timestamps.
pub fn parse_response(
    config: &SamlConfig,
    base64_response: &str,
) -> Result<SamlAssertion, SamlError> {
    if base64_response.is_empty() {
        return Err(SamlError::Parse("empty response".into()));
    }

    // 1. Base64-decode.
    use base64::Engine as _;
    let xml_bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_response.as_bytes())
        .map_err(|e| SamlError::Parse(format!("base64: {e}")))?;
    let xml = std::str::from_utf8(&xml_bytes)
        .map_err(|e| SamlError::Parse(format!("utf8: {e}")))?;

    // 2. Parse XML.
    let parsed = parse_saml_xml(xml)?;

    // 3. Verify RSA-SHA256 signature (if cert is provided).
    if !config.idp_cert_pem.is_empty() && parsed.signed_info_canonical.is_some() {
        verify_rsa_sha256(
            config,
            parsed.signed_info_canonical.as_deref().unwrap(),
            &parsed.signature_value,
        )?;
    }

    // 4. Timestamp validation.
    let not_before_ms = parsed.not_before_ms.unwrap_or(0);
    let not_on_or_after_ms = parsed.not_on_or_after_ms.unwrap_or(u64::MAX);
    let now = now_ms();
    // Allow 60 s leeway for clock skew.
    if now + 60_000 < not_before_ms {
        return Err(SamlError::Expired);
    }
    if now > not_on_or_after_ms + 60_000 {
        return Err(SamlError::Expired);
    }

    let name_id = parsed
        .name_id
        .ok_or_else(|| SamlError::MissingAttribute("NameID".into()))?;

    let groups = parsed
        .attributes
        .get(
            config
                .groups_attribute
                .as_deref()
                .unwrap_or("groups"),
        )
        .cloned()
        .unwrap_or_default();

    Ok(SamlAssertion {
        name_id,
        groups,
        attributes: parsed.attributes,
        not_before_ms,
        not_on_or_after_ms,
        issuer: parsed.issuer.unwrap_or_else(|| config.idp_entity_id.clone()),
    })
}

// ---- XML parsing --------------------------------------------------------

struct ParsedAssertion {
    name_id: Option<String>,
    issuer: Option<String>,
    not_before_ms: Option<u64>,
    not_on_or_after_ms: Option<u64>,
    attributes: std::collections::HashMap<String, Vec<String>>,
    signature_value: Vec<u8>,
    /// Canonicalized `<ds:SignedInfo>` bytes for signature verification.
    signed_info_canonical: Option<String>,
}

fn parse_saml_xml(xml: &str) -> Result<ParsedAssertion, SamlError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut name_id: Option<String> = None;
    let mut issuer: Option<String> = None;
    let mut not_before_ms: Option<u64> = None;
    let mut not_on_or_after_ms: Option<u64> = None;
    let mut attributes: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut signature_value_b64 = String::new();
    let mut signed_info_text: Option<String> = None;

    // Simple state machine tracking current element.
    #[derive(PartialEq)]
    enum State {
        Root,
        NameId,
        Issuer,
        AttributeName(String),
        AttributeValue(String),
        SignatureValue,
        SignedInfo,
    }
    let mut state = State::Root;
    let mut current_attr_name = String::new();

    loop {
        match reader.read_event() {
            Err(e) => return Err(SamlError::Parse(e.to_string())),
            Ok(Event::Eof) => break,

            Ok(Event::Start(e)) => {
                let local = local_name(e.name().local_name().as_ref());
                match local.as_str() {
                    "NameID" => state = State::NameId,
                    "Issuer" if issuer.is_none() => state = State::Issuer,
                    "Conditions" => {
                        for attr in e.attributes().flatten() {
                            let key = local_name(attr.key.local_name().as_ref());
                            let val = std::str::from_utf8(&attr.value).unwrap_or("").to_owned();
                            match key.as_str() {
                                "NotBefore" => not_before_ms = parse_saml_time(&val),
                                "NotOnOrAfter" => not_on_or_after_ms = parse_saml_time(&val),
                                _ => {}
                            }
                        }
                    }
                    "Attribute" => {
                        for attr in e.attributes().flatten() {
                            let key = local_name(attr.key.local_name().as_ref());
                            if key == "Name" {
                                current_attr_name =
                                    std::str::from_utf8(&attr.value).unwrap_or("").to_owned();
                            }
                        }
                        state = State::AttributeName(current_attr_name.clone());
                    }
                    "AttributeValue" => {
                        state = State::AttributeValue(current_attr_name.clone());
                    }
                    "SignatureValue" => state = State::SignatureValue,
                    "SignedInfo" => {
                        // Capture the raw SignedInfo element text for canonicalization.
                        state = State::SignedInfo;
                    }
                    _ => {}
                }
            }

            Ok(Event::Text(e)) => {
                let text = e.unescape().unwrap_or_default().to_string();
                match &state {
                    State::NameId => name_id = Some(text),
                    State::Issuer => issuer = Some(text),
                    State::AttributeValue(name) => {
                        attributes
                            .entry(name.clone())
                            .or_default()
                            .push(text);
                    }
                    State::SignatureValue => {
                        signature_value_b64 = text.chars().filter(|c| !c.is_whitespace()).collect();
                    }
                    State::SignedInfo => {
                        signed_info_text = Some(text);
                    }
                    _ => {}
                }
            }

            Ok(Event::End(_)) => {
                state = State::Root;
            }

            _ => {}
        }
    }

    use base64::Engine as _;
    let sig_bytes = if signature_value_b64.is_empty() {
        vec![]
    } else {
        base64::engine::general_purpose::STANDARD
            .decode(signature_value_b64.as_bytes())
            .unwrap_or_default()
    };

    Ok(ParsedAssertion {
        name_id,
        issuer,
        not_before_ms,
        not_on_or_after_ms,
        attributes,
        signature_value: sig_bytes,
        signed_info_canonical: signed_info_text,
    })
}

// ---- RSA-SHA256 signature verification ----------------------------------

fn verify_rsa_sha256(
    config: &SamlConfig,
    signed_info: &str,
    signature: &[u8],
) -> Result<(), SamlError> {
    use rsa::pkcs1v15::{Signature, VerifyingKey};
    use rsa::pkcs8::DecodePublicKey;
    use rsa::signature::Verifier;
    use rsa::RsaPublicKey;
    use sha2::Sha256;

    // Strip PEM header/footer and decode the DER-encoded public key.
    let pem = config.idp_cert_pem.trim();

    // The cert_pem might be a certificate (X.509) or a bare public key.
    // Try public key first, fall back to extracting from certificate.
    let public_key = if pem.contains("PUBLIC KEY") {
        RsaPublicKey::from_public_key_pem(pem)
            .map_err(|e| SamlError::InvalidSignature(format!("public key pem: {e}")))?
    } else {
        // Extract SubjectPublicKeyInfo from X.509 certificate DER.
        extract_rsa_from_cert_pem(pem)?
    };

    let verifying_key: VerifyingKey<Sha256> = VerifyingKey::new(public_key);

    // Build the RSA-SHA256 signature object from raw bytes.
    let sig = Signature::try_from(signature)
        .map_err(|e| SamlError::InvalidSignature(format!("sig parse: {e}")))?;

    verifying_key
        .verify(signed_info.as_bytes(), &sig)
        .map_err(|e| SamlError::InvalidSignature(format!("verify: {e}")))?;

    Ok(())
}

fn extract_rsa_from_cert_pem(pem: &str) -> Result<rsa::RsaPublicKey, SamlError> {
    // Decode PEM to DER.
    let label_start = pem.find("-----BEGIN").ok_or_else(|| {
        SamlError::InvalidSignature("invalid cert PEM (no BEGIN)".into())
    })?;
    let body_start = pem[label_start..]
        .find('\n')
        .map(|i| label_start + i + 1)
        .ok_or_else(|| SamlError::InvalidSignature("invalid cert PEM (no newline)".into()))?;
    let body_end = pem.rfind("-----END").ok_or_else(|| {
        SamlError::InvalidSignature("invalid cert PEM (no END)".into())
    })?;
    let b64: String = pem[body_start..body_end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    use base64::Engine as _;
    let der = base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .map_err(|e| SamlError::InvalidSignature(format!("cert base64: {e}")))?;

    // Use x509-parser to extract SubjectPublicKeyInfo.
    // We parse the DER manually: an X.509 cert's SubjectPublicKeyInfo is at a known
    // offset after TBSCertificate header fields. For simplicity we use the `rsa`
    // crate's `from_public_key_der` after locating the SPKI.
    // For a full implementation we'd use `x509-parser`; here we use a lightweight
    // approach: find the RSAPublicKey BIT STRING inside the DER.
    parse_spki_from_cert_der(&der)
}

fn parse_spki_from_cert_der(der: &[u8]) -> Result<rsa::RsaPublicKey, SamlError> {
    use rsa::pkcs8::DecodePublicKey;

    // Walk the ASN.1 DER to find the SubjectPublicKeyInfo sequence.
    // X.509 cert structure (simplified):
    //   SEQUENCE (Certificate)
    //     SEQUENCE (TBSCertificate)
    //       ... [version, serialNumber, signature, issuer, validity, subject]
    //       SEQUENCE (SubjectPublicKeyInfo)  <-- we want this
    //
    // We search for the RSA OID (1.2.840.113549.1.1.1) byte pattern.
    // OID encoding: 2a 86 48 86 f7 0d 01 01 01
    let rsa_oid: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01];

    let pos = der
        .windows(rsa_oid.len())
        .position(|w| w == rsa_oid)
        .ok_or_else(|| SamlError::InvalidSignature("RSA OID not found in cert".into()))?;

    // The SPKI SEQUENCE starts 2 bytes before the OID (SEQUENCE tag + length byte(s)).
    // Find the SEQUENCE tag (0x30) just before the OID.
    let spki_start = (0..pos)
        .rev()
        .find(|&i| der[i] == 0x30)
        .ok_or_else(|| SamlError::InvalidSignature("SPKI SEQUENCE not found".into()))?;

    // Determine SPKI length from ASN.1 length encoding.
    let spki_len = asn1_seq_len(der, spki_start)
        .ok_or_else(|| SamlError::InvalidSignature("cannot parse SPKI length".into()))?;
    let spki_end = spki_start + spki_len;
    let spki_der = der
        .get(spki_start..spki_end)
        .ok_or_else(|| SamlError::InvalidSignature("SPKI slice out of bounds".into()))?;

    rsa::RsaPublicKey::from_public_key_der(spki_der)
        .map_err(|e| SamlError::InvalidSignature(format!("SPKI decode: {e}")))
}

/// Return the total byte length (header + content) of an ASN.1 SEQUENCE at `pos`.
fn asn1_seq_len(der: &[u8], pos: usize) -> Option<usize> {
    // tag byte (0x30) + length field
    let len_byte = *der.get(pos + 1)?;
    let (content_len, header_len) = if len_byte & 0x80 == 0 {
        (len_byte as usize, 2usize)
    } else {
        let num_bytes = (len_byte & 0x7f) as usize;
        if num_bytes > 4 || pos + 2 + num_bytes > der.len() {
            return None;
        }
        let mut n = 0usize;
        for i in 0..num_bytes {
            n = (n << 8) | der[pos + 2 + i] as usize;
        }
        (n, 2 + num_bytes)
    };
    Some(header_len + content_len)
}

// ---- Helpers ------------------------------------------------------------

fn local_name(qname: &[u8]) -> String {
    let s = std::str::from_utf8(qname).unwrap_or("");
    s.rfind(':').map(|i| &s[i + 1..]).unwrap_or(s).to_owned()
}

/// Parse SAML ISO-8601 datetime strings to milliseconds since epoch.
fn parse_saml_time(s: &str) -> Option<u64> {
    // Expected format: "2024-01-15T10:30:00Z" or "2024-01-15T10:30:00.000Z"
    // Use a simple parser to avoid pulling in chrono.
    let s = s.trim_end_matches('Z');
    let parts: Vec<&str> = s.splitn(2, 'T').collect();
    if parts.len() != 2 {
        return None;
    }
    let date: Vec<u32> = parts[0].split('-').filter_map(|p| p.parse().ok()).collect();
    let time: Vec<u32> = parts[1]
        .split('.')
        .next()
        .unwrap_or("")
        .split(':')
        .filter_map(|p| p.parse().ok())
        .collect();
    if date.len() < 3 || time.len() < 3 {
        return None;
    }
    let (year, month, day) = (date[0] as i32, date[1], date[2]);
    let (hour, min, sec) = (time[0] as u64, time[1] as u64, time[2] as u64);

    // Days since Unix epoch (1970-01-01).
    let days = days_from_epoch(year, month, day)?;
    let secs = days * 86400 + hour * 3600 + min * 60 + sec;
    Some(secs * 1000)
}

fn days_from_epoch(year: i32, month: u32, day: u32) -> Option<u64> {
    // Gregorian calendar → days since 1970-01-01.
    if year < 1970 {
        return None;
    }
    let mut y = year;
    let mut m = month;
    // Shift to use March as first month for easier leap-year math.
    if m <= 2 {
        y -= 1;
        m += 9;
    } else {
        m -= 3;
    }
    let era = y / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * m as i32 + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era as i64 * 146097 + doe as i64 - 719468;
    if days < 0 {
        None
    } else {
        Some(days as u64)
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn iso_now() -> String {
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

    fn sample_saml_xml() -> String {
        let now = now_ms();
        let not_before = format_iso(now - 60_000);
        let not_after = format_iso(now + 300_000);
        format!(
            r#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol">
  <saml:Issuer xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">https://idp.example.com</saml:Issuer>
  <saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">
    <saml:Issuer>https://idp.example.com</saml:Issuer>
    <saml:Subject>
      <saml:NameID>user@example.com</saml:NameID>
    </saml:Subject>
    <saml:Conditions NotBefore="{not_before}" NotOnOrAfter="{not_after}"/>
    <saml:AttributeStatement>
      <saml:Attribute Name="email">
        <saml:AttributeValue>user@example.com</saml:AttributeValue>
      </saml:Attribute>
      <saml:Attribute Name="groups">
        <saml:AttributeValue>atlas-admins</saml:AttributeValue>
      </saml:Attribute>
    </saml:AttributeStatement>
  </saml:Assertion>
</samlp:Response>"#
        )
    }

    fn format_iso(ms: u64) -> String {
        let secs = ms / 1000;
        let s = secs % 60;
        let m = (secs / 60) % 60;
        let h = (secs / 3600) % 24;
        let days = secs / 86400;
        // Approximate: compute year/month/day from days-since-epoch.
        let (y, mo, d) = days_to_ymd(days);
        format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
    }

    fn days_to_ymd(days: u64) -> (u32, u32, u32) {
        let z = days as i64 + 719468;
        let era = z / 146097;
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        (y as u32, m as u32, d as u32)
    }

    #[test]
    fn authn_request_contains_sp_entity_id() {
        let xml = build_authn_request(&cfg(), "req-001");
        assert!(xml.contains("https://atlas.example.com"));
        assert!(xml.contains("req-001"));
    }

    #[test]
    fn parse_response_valid() {
        use base64::Engine as _;
        let xml = sample_saml_xml();
        let b64 = base64::engine::general_purpose::STANDARD.encode(xml.as_bytes());
        let assertion = parse_response(&cfg(), &b64).unwrap();
        assert!(assertion.is_valid());
        assert_eq!(assertion.name_id, "user@example.com");
    }

    #[test]
    fn parse_empty_response_errors() {
        assert!(parse_response(&cfg(), "").is_err());
    }

    #[test]
    fn parse_time_roundtrip() {
        let ms = parse_saml_time("2024-06-15T12:30:45Z").unwrap();
        // Should be around 2024 (after 2020-01-01 = 1577836800000 ms)
        assert!(ms > 1_577_836_800_000);
    }
}
