//! Minimal AWS Signature Version 4 verifier (no external HMAC crate).
//!
//! Scope is intentionally narrow:
//!
//! - Only the `AWS4-HMAC-SHA256` algorithm.
//! - Only header-based signing (`Authorization` header).
//! - The canonical request is reconstructed from the supplied
//!   [`SignedRequest`] inputs; we do not parse HTTP wire frames here.
//!
//! This is enough to authenticate any standard S3 SDK client against
//! the ATLAS S3 gateway (T5.6).  Streaming/chunked SigV4 is out of
//! scope for v1.

use sha2::{Digest, Sha256};

const ALGO: &str = "AWS4-HMAC-SHA256";

/// Inputs needed to verify a SigV4 request.
pub struct SignedRequest<'a> {
    pub method: &'a str,
    pub canonical_uri: &'a str,
    pub canonical_query: &'a str,
    /// Signed headers, *lowercased keys*, sorted by key. Each entry is
    /// `(name, value)` with the canonical value (trimmed, single-space).
    pub signed_headers: &'a [(&'a str, &'a str)],
    pub payload_hash_hex: &'a str,
    /// Long-form date `YYYYMMDDTHHMMSSZ`.
    pub amz_date: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
    pub provided_signature: &'a str,
}

pub fn verify(req: &SignedRequest<'_>) -> bool {
    let computed = sign(req);
    constant_time_eq(computed.as_bytes(), req.provided_signature.as_bytes())
}

/// Compute the SigV4 hex signature for `req`.
pub fn sign(req: &SignedRequest<'_>) -> String {
    let date_stamp = &req.amz_date[..8];
    let scope = format!("{}/{}/{}/aws4_request", date_stamp, req.region, req.service);

    // Canonical request.
    let mut signed_names: Vec<&str> = req.signed_headers.iter().map(|(k, _)| *k).collect();
    signed_names.sort();
    let signed_headers_str = signed_names.join(";");
    let canonical_headers: String = req
        .signed_headers
        .iter()
        .map(|(k, v)| format!("{k}:{v}\n"))
        .collect();
    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        req.method,
        req.canonical_uri,
        req.canonical_query,
        canonical_headers,
        signed_headers_str,
        req.payload_hash_hex,
    );
    let cr_hash = sha256_hex(canonical_request.as_bytes());

    // String to sign.
    let string_to_sign = format!("{ALGO}\n{}\n{scope}\n{cr_hash}", req.amz_date);

    // Derive signing key.
    let k_date = hmac_sha256(format!("AWS4{}", req.secret_key).as_bytes(), date_stamp.as_bytes());
    let k_region = hmac_sha256(&k_date, req.region.as_bytes());
    let k_service = hmac_sha256(&k_region, req.service.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"aws4_request");
    let sig = hmac_sha256(&k_signing, string_to_sign.as_bytes());
    hex::encode(sig)
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    hex::encode(h.finalize())
}

/// HMAC-SHA256 implemented over [`Sha256`] — avoids pulling the `hmac`
/// crate into the workspace.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        let mut h = Sha256::new();
        h.update(key);
        let mut v = h.finalize().to_vec();
        v.resize(BLOCK, 0);
        v
    } else {
        let mut v = key.to_vec();
        v.resize(BLOCK, 0);
        v
    };
    let mut o_pad = vec![0x5c; BLOCK];
    let mut i_pad = vec![0x36; BLOCK];
    for i in 0..BLOCK {
        o_pad[i] ^= k[i];
        i_pad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(&i_pad);
    inner.update(msg);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&o_pad);
    outer.update(&inner_hash);
    k.iter_mut().for_each(|b| *b = 0); // wipe
    outer.finalize().to_vec()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip: sign then verify must accept; flipping one bit must reject.
    #[test]
    fn sign_verify_roundtrip() {
        let req = SignedRequest {
            method: "GET",
            canonical_uri: "/foo",
            canonical_query: "",
            signed_headers: &[("host", "atlas.local"), ("x-amz-date", "20260101T000000Z")],
            payload_hash_hex: &sha256_hex(b""),
            amz_date: "20260101T000000Z",
            region: "us-east-1",
            service: "s3",
            access_key: "AKIA",
            secret_key: "secret",
            provided_signature: "",
        };
        let sig = sign(&req);
        let mut accepting = SignedRequest { provided_signature: &sig, ..req };
        assert!(verify(&accepting));
        let bad = format!("{}0", &sig[..sig.len() - 1]);
        accepting.provided_signature = &bad;
        assert!(!verify(&accepting));
    }

    /// AWS test vector for the empty-string SHA-256 hash.
    #[test]
    fn empty_sha256() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
