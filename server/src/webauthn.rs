//! Pure-Rust WebAuthn relying-party implementation.
//! Supports passkeys (discoverable credentials) with P-256/ES256 only.
//! Accepts any attestation format but does not verify attestation statements;
//! only the credential data from authData is extracted and used.

use base64ct::{Base64UrlUnpadded, Encoding};
use p256::ecdsa::{Signature, VerifyingKey, signature::Verifier};
use p256::{EncodedPoint, FieldBytes};
use sha2::{Digest, Sha256};

pub struct RelyingParty {
    rp_id: String,
    rp_origin: String,
    rp_name: String,
}

/// Challenge state stored in the session DB during a registration ceremony.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct RegChallenge {
    pub challenge: String, // base64url-encoded random bytes
    pub user_id: String,
    pub username: String,
}

/// Challenge state stored in the session DB during an authentication ceremony.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct AuthChallenge {
    pub challenge: String, // base64url-encoded random bytes
}

/// A registered passkey credential persisted in the database.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct StoredCredential {
    pub cred_id: String, // hex-encoded credential ID
    pub x: String,       // hex-encoded P-256 x coordinate (32 bytes)
    pub y: String,       // hex-encoded P-256 y coordinate (32 bytes)
    pub sign_count: u32,
}

impl RelyingParty {
    pub fn new(rp_id: &str, rp_origin: &str, rp_name: &str) -> Self {
        Self {
            rp_id: rp_id.to_string(),
            rp_origin: rp_origin.to_string(),
            rp_name: rp_name.to_string(),
        }
    }

    /// Begin a registration ceremony.
    /// Returns (PublicKeyCredentialCreationOptions JSON, challenge state to store in session).
    pub fn start_registration(
        &self,
        user_id: &str,
        username: &str,
    ) -> (serde_json::Value, RegChallenge) {
        let challenge_bytes = random_bytes(32);
        let challenge = Base64UrlUnpadded::encode_string(&challenge_bytes);
        let user_id_b64 = Base64UrlUnpadded::encode_string(user_id.as_bytes());

        let options = serde_json::json!({
            "rp": {"id": &self.rp_id, "name": &self.rp_name},
            "user": {
                "id": user_id_b64,
                "name": username,
                "displayName": username,
            },
            "challenge": &challenge,
            "pubKeyCredParams": [{"type": "public-key", "alg": -7}],
            "timeout": 60000,
            "authenticatorSelection": {
                "residentKey": "required",
                "requireResidentKey": true,
                "userVerification": "preferred",
            },
            "attestation": "none",
        });

        (options, RegChallenge { challenge, user_id: user_id.to_string(), username: username.to_string() })
    }

    /// Finish a registration ceremony. Returns the credential to store.
    pub fn finish_registration(
        &self,
        response: &serde_json::Value,
        state: &RegChallenge,
    ) -> Result<StoredCredential, String> {
        let cdj_bytes = b64url_decode(
            response["response"]["clientDataJSON"].as_str().ok_or("missing clientDataJSON")?,
        )?;
        let cdj: serde_json::Value =
            serde_json::from_slice(&cdj_bytes).map_err(|e| format!("clientDataJSON: {e}"))?;

        if cdj["type"].as_str() != Some("webauthn.create") {
            return Err("wrong clientData type".into());
        }
        let got = cdj["challenge"].as_str().ok_or("missing challenge in clientData")?;
        if !ct_eq(got.as_bytes(), state.challenge.as_bytes()) {
            return Err("challenge mismatch".into());
        }
        if cdj["origin"].as_str() != Some(&self.rp_origin) {
            return Err(format!("origin mismatch: {:?}", cdj["origin"]));
        }

        let attest_bytes = b64url_decode(
            response["response"]["attestationObject"].as_str().ok_or("missing attestationObject")?,
        )?;
        let attest: ciborium::value::Value = ciborium::de::from_reader(attest_bytes.as_slice())
            .map_err(|e| format!("attestationObject CBOR: {e}"))?;
        let auth_data =
            cbor_get_bytes(&attest, "authData").ok_or("missing authData in attestationObject")?;

        parse_auth_data_registration(&self.rp_id, &auth_data)
    }

    /// Begin an authentication ceremony.
    /// Returns (PublicKeyCredentialRequestOptions JSON, challenge state to store in session).
    pub fn start_authentication(
        &self,
        credentials: &[StoredCredential],
    ) -> (serde_json::Value, AuthChallenge) {
        let challenge_bytes = random_bytes(32);
        let challenge = Base64UrlUnpadded::encode_string(&challenge_bytes);

        let allow: Vec<serde_json::Value> = credentials
            .iter()
            .map(|c| {
                let id_b64 = Base64UrlUnpadded::encode_string(&hex_decode(&c.cred_id));
                serde_json::json!({"type": "public-key", "id": id_b64})
            })
            .collect();

        let options = serde_json::json!({
            "challenge": &challenge,
            "timeout": 60000,
            "rpId": &self.rp_id,
            "allowCredentials": allow,
            "userVerification": "preferred",
        });

        (options, AuthChallenge { challenge })
    }

    /// Finish an authentication ceremony.
    /// Returns the matched credential with its sign_count updated to the new value.
    pub fn finish_authentication(
        &self,
        response: &serde_json::Value,
        state: &AuthChallenge,
        credentials: &[StoredCredential],
    ) -> Result<StoredCredential, String> {
        let cdj_bytes = b64url_decode(
            response["response"]["clientDataJSON"].as_str().ok_or("missing clientDataJSON")?,
        )?;
        let cdj: serde_json::Value =
            serde_json::from_slice(&cdj_bytes).map_err(|e| format!("clientDataJSON: {e}"))?;

        if cdj["type"].as_str() != Some("webauthn.get") {
            return Err("wrong clientData type".into());
        }
        let got = cdj["challenge"].as_str().ok_or("missing challenge in clientData")?;
        if !ct_eq(got.as_bytes(), state.challenge.as_bytes()) {
            return Err("challenge mismatch".into());
        }
        if cdj["origin"].as_str() != Some(&self.rp_origin) {
            return Err(format!("origin mismatch: {:?}", cdj["origin"]));
        }

        let auth_data = b64url_decode(
            response["response"]["authenticatorData"].as_str().ok_or("missing authenticatorData")?,
        )?;
        if auth_data.len() < 37 {
            return Err("authenticatorData too short".into());
        }
        let rp_id_hash: [u8; 32] = Sha256::digest(self.rp_id.as_bytes()).into();
        if auth_data[..32] != rp_id_hash {
            return Err("rpIdHash mismatch".into());
        }
        if auth_data[32] & 0x01 == 0 {
            return Err("user not present".into());
        }
        let sign_count = u32::from_be_bytes(auth_data[33..37].try_into().unwrap());

        // Locate the credential that was used.
        let cred_id_bytes =
            b64url_decode(response["id"].as_str().ok_or("missing credential id")?)?;
        let cred_id_hex = crate::db::hex_encode(&cred_id_bytes);
        let cred = credentials
            .iter()
            .find(|c| c.cred_id == cred_id_hex)
            .ok_or("credential not found")?;

        // Verify ES256 signature over authData || SHA-256(clientDataJSON).
        let cdj_hash: [u8; 32] = Sha256::digest(&cdj_bytes).into();
        let mut signed = auth_data.clone();
        signed.extend_from_slice(&cdj_hash);

        let sig_bytes = b64url_decode(
            response["response"]["signature"].as_str().ok_or("missing signature")?,
        )?;
        let x = hex_decode_32(&cred.x)?;
        let y = hex_decode_32(&cred.y)?;
        let point =
            EncodedPoint::from_affine_coordinates(FieldBytes::from_slice(&x), FieldBytes::from_slice(&y), false);
        let vk = VerifyingKey::from_encoded_point(&point)
            .map_err(|e| format!("invalid public key: {e}"))?;
        let sig = Signature::from_der(&sig_bytes)
            .map_err(|e| format!("invalid signature encoding: {e}"))?;
        vk.verify(&signed, &sig).map_err(|_| "signature verification failed".to_string())?;

        // Warn on sign_count regression — indicates possible authenticator cloning.
        // Don't hard-fail: many platform authenticators always return 0.
        if cred.sign_count > 0 && sign_count <= cred.sign_count {
            tracing::warn!(
                cred_id = %cred.cred_id,
                stored = cred.sign_count,
                received = sign_count,
                "sign_count did not increase — possible authenticator clone"
            );
        }

        Ok(StoredCredential {
            cred_id: cred.cred_id.clone(),
            x: cred.x.clone(),
            y: cred.y.clone(),
            sign_count,
        })
    }
}

/// Parse authenticatorData during registration, extract credential ID and P-256 public key.
fn parse_auth_data_registration(rp_id: &str, auth_data: &[u8]) -> Result<StoredCredential, String> {
    // Layout: [32 rpIdHash][1 flags][4 signCount][attested cred data when AT flag set]
    if auth_data.len() < 37 {
        return Err("authData too short".into());
    }
    let rp_id_hash: [u8; 32] = Sha256::digest(rp_id.as_bytes()).into();
    if auth_data[..32] != rp_id_hash {
        return Err("rpIdHash mismatch".into());
    }
    let flags = auth_data[32];
    if flags & 0x01 == 0 {
        return Err("user not present".into());
    }
    if flags & 0x40 == 0 {
        return Err("no attested credential data present".into());
    }
    let sign_count = u32::from_be_bytes(auth_data[33..37].try_into().unwrap());

    // Attested credential data: [16 AAGUID][2 credIdLen][N credId][CBOR coseKey]
    let att = &auth_data[37..];
    if att.len() < 18 {
        return Err("attested credential data too short".into());
    }
    let cred_id_len = u16::from_be_bytes([att[16], att[17]]) as usize;
    if att.len() < 18 + cred_id_len {
        return Err("credentialId truncated".into());
    }
    let cred_id = crate::db::hex_encode(&att[18..18 + cred_id_len]);

    let cose_bytes = &att[18 + cred_id_len..];
    let (x, y) = parse_cose_p256(cose_bytes)?;

    Ok(StoredCredential {
        cred_id,
        x: crate::db::hex_encode(&x),
        y: crate::db::hex_encode(&y),
        sign_count,
    })
}

/// Parse a COSE_Key map for an EC2/P-256 key (alg=-7), returning (x, y) as 32-byte arrays.
fn parse_cose_p256(data: &[u8]) -> Result<([u8; 32], [u8; 32]), String> {
    let value: ciborium::value::Value =
        ciborium::de::from_reader(data).map_err(|e| format!("COSE key CBOR: {e}"))?;
    let map = match value {
        ciborium::value::Value::Map(m) => m,
        _ => return Err("COSE key is not a CBOR map".into()),
    };

    let mut kty: Option<i64> = None;
    let mut crv: Option<i64> = None;
    let mut x_bytes: Option<Vec<u8>> = None;
    let mut y_bytes: Option<Vec<u8>> = None;

    for (k, v) in map {
        let key_i = match k {
            ciborium::value::Value::Integer(i) => i64::try_from(i).ok(),
            _ => None,
        };
        match key_i {
            Some(1) => kty = if let ciborium::value::Value::Integer(i) = v { i64::try_from(i).ok() } else { None },
            Some(-1) => crv = if let ciborium::value::Value::Integer(i) = v { i64::try_from(i).ok() } else { None },
            Some(-2) => {
                if let ciborium::value::Value::Bytes(b) = v {
                    x_bytes = Some(b);
                }
            }
            Some(-3) => {
                if let ciborium::value::Value::Bytes(b) = v {
                    y_bytes = Some(b);
                }
            }
            _ => {}
        }
    }

    if kty != Some(2) {
        return Err(format!("unsupported COSE key type: {kty:?} (expected 2/EC2)"));
    }
    if crv != Some(1) {
        return Err(format!("unsupported COSE curve: {crv:?} (expected 1/P-256)"));
    }

    let x = x_bytes.ok_or("COSE key missing x coordinate")?;
    let y = y_bytes.ok_or("COSE key missing y coordinate")?;
    if x.len() != 32 || y.len() != 32 {
        return Err(format!("unexpected coordinate length: x={}, y={}", x.len(), y.len()));
    }
    let mut xa = [0u8; 32];
    let mut ya = [0u8; 32];
    xa.copy_from_slice(&x);
    ya.copy_from_slice(&y);
    Ok((xa, ya))
}

/// Extract a byte-string value from a CBOR text-keyed map.
fn cbor_get_bytes(value: &ciborium::value::Value, key: &str) -> Option<Vec<u8>> {
    let ciborium::value::Value::Map(m) = value else { return None };
    for (k, v) in m {
        if matches!(k, ciborium::value::Value::Text(t) if t == key) {
            if let ciborium::value::Value::Bytes(b) = v {
                return Some(b.clone());
            }
        }
    }
    None
}

fn b64url_decode(s: &str) -> Result<Vec<u8>, String> {
    // Strip any trailing padding — WebAuthn uses unpadded base64url but be permissive.
    Base64UrlUnpadded::decode_vec(s.trim_end_matches('='))
        .map_err(|e| format!("base64url decode: {e}"))
}

fn hex_decode(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap_or(0))
        .collect()
}

fn hex_decode_32(s: &str) -> Result<[u8; 32], String> {
    let v = hex_decode(s);
    if v.len() != 32 {
        return Err(format!("expected 32 bytes, got {}", v.len()));
    }
    let mut a = [0u8; 32];
    a.copy_from_slice(&v);
    Ok(a)
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    let mut f = std::fs::File::open("/dev/urandom").expect("open /dev/urandom");
    std::io::Read::read_exact(&mut f, &mut buf).expect("read /dev/urandom");
    buf
}

// Constant-time byte slice comparison.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::ecdsa::{SigningKey, Signature, signature::Signer};
    use p256::SecretKey;

    fn make_rp() -> RelyingParty {
        RelyingParty::new("example.com", "https://example.com", "Example")
    }

    /// A minimal software authenticator backed by a fixed P-256 key.
    struct SoftAuthenticator {
        signing_key: SigningKey,
        cred_id: Vec<u8>,
    }

    impl SoftAuthenticator {
        fn new() -> Self {
            // Scalar value 7 — arbitrary non-zero value well below the P-256 group order.
            let mut key_bytes = [0u8; 32];
            key_bytes[31] = 7;
            Self {
                signing_key: SigningKey::from(SecretKey::from_slice(&key_bytes).unwrap()),
                cred_id: (1u8..=16).collect(),
            }
        }

        fn cred_id_b64(&self) -> String {
            Base64UrlUnpadded::encode_string(&self.cred_id)
        }

        fn make_cose_key(&self) -> Vec<u8> {
            let point = self.signing_key.verifying_key().to_encoded_point(false);
            let x = point.x().unwrap().to_vec();
            let y = point.y().unwrap().to_vec();
            let mut cbor = Vec::new();
            ciborium::ser::into_writer(
                &ciborium::value::Value::Map(vec![
                    (cbor_int(1), cbor_int(2)),   // kty: EC2
                    (cbor_int(3), cbor_int(-7)),  // alg: ES256
                    (cbor_int(-1), cbor_int(1)),  // crv: P-256
                    (cbor_int(-2), ciborium::value::Value::Bytes(x)),
                    (cbor_int(-3), ciborium::value::Value::Bytes(y)),
                ]),
                &mut cbor,
            )
            .unwrap();
            cbor
        }

        fn make_auth_data_with_cred(&self, rp_id: &str) -> Vec<u8> {
            let rp_id_hash: [u8; 32] = sha2::Sha256::digest(rp_id.as_bytes()).into();
            let cose_key = self.make_cose_key();
            let mut auth_data = Vec::new();
            auth_data.extend_from_slice(&rp_id_hash);
            auth_data.push(0x41); // UP + AT flags
            auth_data.extend_from_slice(&0u32.to_be_bytes()); // sign_count
            auth_data.extend_from_slice(&[0u8; 16]); // AAGUID
            auth_data.extend_from_slice(&(self.cred_id.len() as u16).to_be_bytes());
            auth_data.extend_from_slice(&self.cred_id);
            auth_data.extend_from_slice(&cose_key);
            auth_data
        }

        fn registration_response(&self, rp_id: &str, origin: &str, challenge: &str) -> serde_json::Value {
            let auth_data = self.make_auth_data_with_cred(rp_id);

            let mut attest_obj = Vec::new();
            ciborium::ser::into_writer(
                &ciborium::value::Value::Map(vec![
                    (cbor_text("fmt"), cbor_text("none")),
                    (cbor_text("attStmt"), ciborium::value::Value::Map(vec![])),
                    (cbor_text("authData"), ciborium::value::Value::Bytes(auth_data)),
                ]),
                &mut attest_obj,
            )
            .unwrap();

            let cdj = serde_json::json!({"type": "webauthn.create", "challenge": challenge, "origin": origin});
            let cdj_bytes = serde_json::to_vec(&cdj).unwrap();

            serde_json::json!({
                "id": self.cred_id_b64(),
                "rawId": self.cred_id_b64(),
                "type": "public-key",
                "response": {
                    "clientDataJSON": Base64UrlUnpadded::encode_string(&cdj_bytes),
                    "attestationObject": Base64UrlUnpadded::encode_string(&attest_obj),
                }
            })
        }

        fn authentication_response(
            &self,
            rp_id: &str,
            origin: &str,
            challenge: &str,
            sign_count: u32,
        ) -> serde_json::Value {
            let rp_id_hash: [u8; 32] = sha2::Sha256::digest(rp_id.as_bytes()).into();
            let mut auth_data = Vec::new();
            auth_data.extend_from_slice(&rp_id_hash);
            auth_data.push(0x05); // UP + UV flags
            auth_data.extend_from_slice(&sign_count.to_be_bytes());

            let cdj = serde_json::json!({"type": "webauthn.get", "challenge": challenge, "origin": origin});
            let cdj_bytes = serde_json::to_vec(&cdj).unwrap();
            let cdj_hash: [u8; 32] = sha2::Sha256::digest(&cdj_bytes).into();

            let mut signed = auth_data.clone();
            signed.extend_from_slice(&cdj_hash);

            let sig: Signature = self.signing_key.sign(&signed);
            let sig_der = sig.to_der();

            serde_json::json!({
                "id": self.cred_id_b64(),
                "rawId": self.cred_id_b64(),
                "type": "public-key",
                "response": {
                    "clientDataJSON": Base64UrlUnpadded::encode_string(&cdj_bytes),
                    "authenticatorData": Base64UrlUnpadded::encode_string(&auth_data),
                    "signature": Base64UrlUnpadded::encode_string(sig_der.as_bytes()),
                    "userHandle": serde_json::Value::Null,
                }
            })
        }
    }

    fn cbor_int(i: i64) -> ciborium::value::Value {
        ciborium::value::Value::Integer(i.into())
    }

    fn cbor_text(s: &str) -> ciborium::value::Value {
        ciborium::value::Value::Text(s.to_string())
    }

    fn do_register(rp: &RelyingParty, authn: &SoftAuthenticator) -> StoredCredential {
        let (opts, reg_state) = rp.start_registration("uid-1", "alice");
        let challenge = opts["challenge"].as_str().unwrap().to_string();
        let response = authn.registration_response("example.com", "https://example.com", &challenge);
        rp.finish_registration(&response, &reg_state).unwrap()
    }

    #[test]
    fn round_trip_registration_then_authentication() {
        let rp = make_rp();
        let authn = SoftAuthenticator::new();

        let cred = do_register(&rp, &authn);
        assert_eq!(cred.cred_id, crate::db::hex_encode(&authn.cred_id));
        assert_eq!(cred.sign_count, 0);

        let (auth_opts, auth_state) = rp.start_authentication(&[cred.clone()]);
        let challenge = auth_opts["challenge"].as_str().unwrap().to_string();
        let response = authn.authentication_response("example.com", "https://example.com", &challenge, 1);
        let updated = rp.finish_authentication(&response, &auth_state, &[cred]).unwrap();

        assert_eq!(updated.sign_count, 1);
    }

    #[test]
    fn wrong_challenge_rejected_at_registration() {
        let rp = make_rp();
        let authn = SoftAuthenticator::new();
        let (_, reg_state) = rp.start_registration("uid-1", "alice");
        let response = authn.registration_response("example.com", "https://example.com", "wrong-challenge");
        assert!(rp.finish_registration(&response, &reg_state).is_err());
    }

    #[test]
    fn wrong_challenge_rejected_at_authentication() {
        let rp = make_rp();
        let authn = SoftAuthenticator::new();
        let cred = do_register(&rp, &authn);
        let (_, auth_state) = rp.start_authentication(&[cred.clone()]);
        let response = authn.authentication_response("example.com", "https://example.com", "wrong-challenge", 1);
        assert!(rp.finish_authentication(&response, &auth_state, &[cred]).is_err());
    }

    #[test]
    fn wrong_origin_rejected() {
        let rp = make_rp();
        let authn = SoftAuthenticator::new();
        let (opts, reg_state) = rp.start_registration("uid-1", "alice");
        let challenge = opts["challenge"].as_str().unwrap().to_string();
        let response = authn.registration_response("example.com", "https://evil.example.com", &challenge);
        assert!(rp.finish_registration(&response, &reg_state).is_err());
    }

    #[test]
    fn bad_signature_rejected() {
        let rp = make_rp();
        let authn = SoftAuthenticator::new();
        let cred = do_register(&rp, &authn);

        // Different key, same cred_id — simulates a forged assertion.
        let mut key_bytes = [0u8; 32];
        key_bytes[31] = 99;
        let imposter = SoftAuthenticator {
            signing_key: SigningKey::from(SecretKey::from_slice(&key_bytes).unwrap()),
            cred_id: authn.cred_id.clone(),
        };

        let (auth_opts, auth_state) = rp.start_authentication(&[cred.clone()]);
        let challenge = auth_opts["challenge"].as_str().unwrap().to_string();
        let response = imposter.authentication_response("example.com", "https://example.com", &challenge, 1);
        assert!(rp.finish_authentication(&response, &auth_state, &[cred]).is_err());
    }

    #[test]
    fn wrong_rp_id_rejected() {
        let rp = make_rp();
        let authn = SoftAuthenticator::new();
        let (opts, reg_state) = rp.start_registration("uid-1", "alice");
        let challenge = opts["challenge"].as_str().unwrap().to_string();
        // Authenticator hashes a different rp_id into authData.
        let response = authn.registration_response("attacker.com", "https://example.com", &challenge);
        assert!(rp.finish_registration(&response, &reg_state).is_err());
    }
}
