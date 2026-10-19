//! ES256 JWTs by hand: one key per process, published at `/.well-known/jwks.json`.

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
use p256::{
    ecdsa::{Signature, SigningKey, VerifyingKey, signature::Signer, signature::Verifier},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub struct Keys {
    sk: SigningKey,
    pub kid: String,
}

impl Keys {
    pub fn generate() -> Self {
        let sk = SigningKey::random(&mut rand_core::OsRng);
        let point = sk.verifying_key().to_encoded_point(false);
        let kid = B64.encode(&Sha256::digest(point.as_bytes())[..12]);
        Self { sk, kid }
    }

    pub fn jwks(&self) -> Value {
        let point = self.sk.verifying_key().to_encoded_point(false);
        json!({ "keys": [{
            "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": self.kid,
            "x": B64.encode(point.x().expect("uncompressed")),
            "y": B64.encode(point.y().expect("uncompressed")),
        }]})
    }

    pub fn sign(&self, claims: &Value) -> String {
        let header = json!({ "alg": "ES256", "typ": "JWT", "kid": self.kid });
        let input = format!("{}.{}", B64.encode(header.to_string()), B64.encode(claims.to_string()));
        let sig: Signature = self.sk.sign(input.as_bytes());
        format!("{input}.{}", B64.encode(sig.to_bytes()))
    }

    /// Verifies the signature only; the caller checks the claims it cares about.
    pub fn verify(&self, token: &str) -> Option<Value> {
        let (input, sig) = token.rsplit_once('.')?;
        let sig = Signature::from_slice(&B64.decode(sig).ok()?).ok()?;
        VerifyingKey::from(&self.sk).verify(input.as_bytes(), &sig).ok()?;
        let (_, payload) = input.split_once('.')?;
        serde_json::from_slice(&B64.decode(payload).ok()?).ok()
    }
}
