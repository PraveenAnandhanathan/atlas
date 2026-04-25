//! Read-time PII redaction engine (T4.6).
//!
//! Detects and masks emails, US SSNs, common API key formats, and
//! arbitrary caller-supplied regex patterns.

use crate::Result;
use regex::Regex;

/// Configuration for which PII categories to redact.
#[derive(Debug, Clone, Default)]
pub struct RedactConfig {
    pub redact_email: bool,
    pub redact_ssn: bool,
    pub redact_api_keys: bool,
    /// Additional (pattern, replacement) pairs.
    pub custom_patterns: Vec<(String, String)>,
}

impl RedactConfig {
    /// Enable all built-in detectors.
    pub fn all() -> Self {
        Self {
            redact_email: true,
            redact_ssn: true,
            redact_api_keys: true,
            custom_patterns: vec![],
        }
    }
}

/// Compiled, ready-to-apply redaction engine.
pub struct RedactEngine {
    patterns: Vec<(Regex, String)>,
}

impl RedactEngine {
    pub fn new(config: &RedactConfig) -> Result<Self> {
        let mut patterns: Vec<(Regex, String)> = Vec::new();

        if config.redact_email {
            patterns.push((
                Regex::new(r"[a-zA-Z0-9._%+\-]+@[a-zA-Z0-9.\-]+\.[a-zA-Z]{2,}")?,
                "[REDACTED_EMAIL]".into(),
            ));
        }
        if config.redact_ssn {
            patterns.push((
                Regex::new(r"\b\d{3}-\d{2}-\d{4}\b")?,
                "[REDACTED_SSN]".into(),
            ));
        }
        if config.redact_api_keys {
            patterns.push((
                Regex::new(
                    r"(?i)\b(sk-[A-Za-z0-9]{20,}|ghp_[A-Za-z0-9]{36,}|xoxb-[A-Za-z0-9\-]+)",
                )?,
                "[REDACTED_API_KEY]".into(),
            ));
            // Bearer token
            patterns.push((
                Regex::new(r"(?i)Bearer\s+[A-Za-z0-9\-._~+/]+=*")?,
                "[REDACTED_BEARER]".into(),
            ));
        }
        for (pat, repl) in &config.custom_patterns {
            patterns.push((Regex::new(pat)?, repl.clone()));
        }
        Ok(Self { patterns })
    }

    /// Return a copy of `text` with all PII replaced by redaction markers.
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (re, repl) in &self.patterns {
            out = re.replace_all(&out, repl.as_str()).into_owned();
        }
        out
    }

    /// Returns true if `text` contains at least one PII match.
    pub fn has_pii(&self, text: &str) -> bool {
        self.patterns.iter().any(|(re, _)| re.is_match(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_email() {
        let e = RedactEngine::new(&RedactConfig {
            redact_email: true,
            ..Default::default()
        })
        .unwrap();
        let out = e.redact("Contact us at user@example.com for help.");
        assert!(!out.contains("user@example.com"));
        assert!(out.contains("[REDACTED_EMAIL]"));
    }

    #[test]
    fn redact_ssn() {
        let e = RedactEngine::new(&RedactConfig {
            redact_ssn: true,
            ..Default::default()
        })
        .unwrap();
        let out = e.redact("SSN: 123-45-6789");
        assert!(!out.contains("123-45-6789"));
        assert!(out.contains("[REDACTED_SSN]"));
    }

    #[test]
    fn redact_api_key() {
        let e = RedactEngine::new(&RedactConfig {
            redact_api_keys: true,
            ..Default::default()
        })
        .unwrap();
        let out = e.redact("key=sk-abcdefghijklmnopqrstuv");
        assert!(!out.contains("sk-abcdefghijklmnopqrstuv"));
        assert!(out.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn has_pii_detection() {
        let e = RedactEngine::new(&RedactConfig::all()).unwrap();
        assert!(e.has_pii("admin@corp.io sent the report"));
        assert!(!e.has_pii("no sensitive data here"));
    }

    #[test]
    fn custom_pattern() {
        let cfg = RedactConfig {
            custom_patterns: vec![("PROJECT-\\d+".into(), "[TICKET]".into())],
            ..Default::default()
        };
        let e = RedactEngine::new(&cfg).unwrap();
        let out = e.redact("See PROJECT-1234 for details.");
        assert!(out.contains("[TICKET]"));
    }
}
