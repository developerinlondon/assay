use super::json::{json_value_to_lua, lua_value_to_json};
use data_encoding::BASE64URL_NOPAD;
use digest::Digest;
use jsonwebtoken::jwk::{JwkSet, KeyAlgorithm};
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header};
use mlua::{Lua, Value};
use rand::RngExt;
use zeroize::Zeroizing;

pub fn register_crypto(lua: &Lua) -> mlua::Result<()> {
    let crypto_table = lua.create_table()?;

    let jwt_sign_fn = lua.create_function(|_, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let claims_table = match args_iter.next() {
            Some(Value::Table(t)) => t,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_sign: first argument must be a claims table",
                ));
            }
        };

        let pem_key: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_sign: second argument must be a PEM key string",
                ));
            }
        };

        let algorithm = match args_iter.next() {
            Some(Value::String(s)) => match s.to_str()?.to_uppercase().as_str() {
                "RS256" => Algorithm::RS256,
                "RS384" => Algorithm::RS384,
                "RS512" => Algorithm::RS512,
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "crypto.jwt_sign: unsupported algorithm: {other}"
                    )));
                }
            },
            Some(Value::Nil) | None => Algorithm::RS256,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_sign: third argument must be an algorithm string or nil",
                ));
            }
        };

        // 4th optional argument: options table with header fields (kid, typ, etc.)
        let opts = match args_iter.next() {
            Some(Value::Table(t)) => Some(t),
            Some(Value::Nil) | None => None,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_sign: fourth argument must be an options table or nil",
                ));
            }
        };

        let claims_json = lua_value_to_json(&Value::Table(claims_table))?;
        let pem_bytes = Zeroizing::new(pem_key.into_bytes());
        let key = EncodingKey::from_rsa_pem(&pem_bytes)
            .map_err(|e| mlua::Error::runtime(format!("crypto.jwt_sign: invalid PEM key: {e}")))?;

        let mut header = Header::new(algorithm);
        if let Some(ref opts_table) = opts
            && let Ok(kid) = opts_table.get::<String>("kid")
        {
            header.kid = Some(kid);
        }
        let token = jsonwebtoken::encode(&header, &claims_json, &key)
            .map_err(|e| mlua::Error::runtime(format!("crypto.jwt_sign: encoding failed: {e}")))?;

        Ok(token)
    })?;
    crypto_table.set("jwt_sign", jwt_sign_fn)?;

    // crypto.jwt_decode(token) -> { header, claims }
    //
    // Decodes a JWT WITHOUT verifying its signature. Returns a table with
    // `header` and `claims` sub-tables — both parsed from the base64url
    // segments of the token. Useful when the JWT travels through a trusted
    // channel (e.g. your own session cookie set over TLS) and you just
    // need to read the claims. For untrusted JWTs, use a verifier instead.
    let jwt_decode_fn = lua.create_function(|lua, token: String| {
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err(mlua::Error::runtime(
                "crypto.jwt_decode: token must have three '.'-separated segments (header.payload.signature)",
            ));
        }

        let decode_segment = |segment: &str, label: &str| -> mlua::Result<serde_json::Value> {
            // JWTs use unpadded base64url encoding per RFC 7515.
            let bytes = BASE64URL_NOPAD.decode(segment.as_bytes()).map_err(|e| {
                mlua::Error::runtime(format!(
                    "crypto.jwt_decode: {label}: invalid base64url: {e}"
                ))
            })?;
            serde_json::from_slice(&bytes).map_err(|e| {
                mlua::Error::runtime(format!("crypto.jwt_decode: {label}: invalid JSON: {e}"))
            })
        };

        let header = decode_segment(parts[0], "header")?;
        let claims = decode_segment(parts[1], "payload")?;

        let result = lua.create_table()?;
        result.set("header", json_value_to_lua(lua, &header)?)?;
        result.set("claims", json_value_to_lua(lua, &claims)?)?;
        Ok(result)
    })?;
    crypto_table.set("jwt_decode", jwt_decode_fn)?;

    // crypto.jwt_verify(token, key, opts?) -> { header, claims }
    //
    // Verify a JWT's signature and validate its claims. Mirrors
    // `crypto.jwt_sign` for the receive side. Use this for tokens
    // arriving from untrusted sources; for tokens travelling through a
    // trusted channel (e.g. your own session cookie over TLS) where
    // you only need to read claims, prefer `crypto.jwt_decode`.
    //
    // `key` accepts either:
    //   - a PEM-encoded RSA public key string (algorithm taken from
    //     `opts.algorithm`, default RS256)
    //   - a JWKS table `{ keys = { ... } }` — the verifier dispatches on
    //     the JWT header's `kid`, picks the matching JWK, and uses the
    //     JWK's `alg` (or the JWT header's `alg` if the key omits it)
    //
    // `opts` (table, optional):
    //   - `algorithm`: "RS256" | "RS384" | "RS512" — only used for the PEM path
    //   - `audience`: string or array of strings — validates `aud`
    //   - `issuer`:   string or array of strings — validates `iss`
    //   - `leeway`: integer seconds of clock skew tolerance (default 0)
    //   - `validate_exp`: boolean (default true)
    //   - `validate_nbf`: boolean (default false)
    //   - `required_claims`: array of strings (default ["exp"]; pass {} to skip)
    //
    // Returns `{ header = {...}, claims = {...} }`. Raises on signature
    // mismatch, expired token, audience/issuer mismatch, malformed token,
    // missing JWK, or bad arguments.
    let jwt_verify_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let token: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_verify: first argument must be a JWT string",
                ));
            }
        };

        let key_arg = match args_iter.next() {
            Some(v) => v,
            None => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_verify: second argument must be a PEM string or JWKS table",
                ));
            }
        };

        let opts: Option<mlua::Table> = match args_iter.next() {
            Some(Value::Table(t)) => Some(t),
            Some(Value::Nil) | None => None,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_verify: third argument must be an options table or nil",
                ));
            }
        };

        // Dispatch on key type (PEM string vs JWKS table).
        let (decoding_key, algorithm) = match key_arg {
            Value::String(pem) => {
                let pem_str = pem.to_str()?.to_string();
                let alg_str = opts
                    .as_ref()
                    .and_then(|t| t.get::<Option<String>>("algorithm").ok().flatten());
                let alg = match alg_str.as_deref() {
                    Some(s) => match s.to_uppercase().as_str() {
                        "RS256" => Algorithm::RS256,
                        "RS384" => Algorithm::RS384,
                        "RS512" => Algorithm::RS512,
                        other => {
                            return Err(mlua::Error::runtime(format!(
                                "crypto.jwt_verify: unsupported algorithm: {other}"
                            )));
                        }
                    },
                    None => Algorithm::RS256,
                };
                let pem_bytes = Zeroizing::new(pem_str.into_bytes());
                let key = DecodingKey::from_rsa_pem(&pem_bytes).map_err(|e| {
                    mlua::Error::runtime(format!("crypto.jwt_verify: invalid PEM key: {e}"))
                })?;
                (key, alg)
            }
            Value::Table(jwks_table) => {
                let jwks_json = lua_value_to_json(&Value::Table(jwks_table))?;
                let jwks: JwkSet = serde_json::from_value(jwks_json).map_err(|e| {
                    mlua::Error::runtime(format!("crypto.jwt_verify: invalid JWKS table: {e}"))
                })?;

                let header = decode_header(&token).map_err(|e| {
                    mlua::Error::runtime(format!(
                        "crypto.jwt_verify: malformed token header: {e}"
                    ))
                })?;
                let kid = header.kid.as_deref().ok_or_else(|| {
                    mlua::Error::runtime(
                        "crypto.jwt_verify: token header missing 'kid' (required for JWKS dispatch)",
                    )
                })?;
                let jwk = jwks.find(kid).ok_or_else(|| {
                    mlua::Error::runtime(format!(
                        "crypto.jwt_verify: no key in JWKS matches kid '{kid}'"
                    ))
                })?;

                let alg = jwk
                    .common
                    .key_algorithm
                    .and_then(key_algorithm_to_algorithm)
                    .unwrap_or(header.alg);
                let key = DecodingKey::from_jwk(jwk).map_err(|e| {
                    mlua::Error::runtime(format!("crypto.jwt_verify: cannot decode JWK: {e}"))
                })?;
                (key, alg)
            }
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.jwt_verify: second argument must be a PEM string or JWKS table",
                ));
            }
        };

        let mut validation = Validation::new(algorithm);

        if let Some(opts_table) = opts.as_ref() {
            if let Some(audience) = lua_string_or_array(opts_table, "audience", "crypto.jwt_verify")? {
                validation.set_audience(&audience);
            }
            if let Some(issuer) = lua_string_or_array(opts_table, "issuer", "crypto.jwt_verify")? {
                validation.set_issuer(&issuer);
            }
            if let Ok(Some(leeway)) = opts_table.get::<Option<u64>>("leeway") {
                validation.leeway = leeway;
            }
            if let Ok(Some(validate_exp)) = opts_table.get::<Option<bool>>("validate_exp") {
                validation.validate_exp = validate_exp;
            }
            if let Ok(Some(validate_nbf)) = opts_table.get::<Option<bool>>("validate_nbf") {
                validation.validate_nbf = validate_nbf;
            }
            if let Ok(Some(required_table)) = opts_table.get::<Option<mlua::Table>>("required_claims") {
                let mut required: Vec<String> = Vec::new();
                for item in required_table.sequence_values::<mlua::String>() {
                    required.push(item?.to_str()?.to_string());
                }
                let required_refs: Vec<&str> = required.iter().map(String::as_str).collect();
                validation.set_required_spec_claims(&required_refs);
            }
        }

        let token_data: jsonwebtoken::TokenData<serde_json::Value> =
            decode(&token, &decoding_key, &validation)
                .map_err(|e| mlua::Error::runtime(format!("crypto.jwt_verify: {e}")))?;

        let header_json = serde_json::to_value(&token_data.header).map_err(|e| {
            mlua::Error::runtime(format!("crypto.jwt_verify: serialize header: {e}"))
        })?;
        let result = lua.create_table()?;
        result.set("header", json_value_to_lua(lua, &header_json)?)?;
        result.set("claims", json_value_to_lua(lua, &token_data.claims)?)?;
        Ok(result)
    })?;
    crypto_table.set("jwt_verify", jwt_verify_fn)?;

    let hash_fn = lua.create_function(|_, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let input: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.hash: first argument must be a string",
                ));
            }
        };

        let algorithm: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_lowercase(),
            Some(Value::Nil) | None => "sha256".to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.hash: second argument must be an algorithm string or nil",
                ));
            }
        };

        let hex = match algorithm.as_str() {
            "sha224" => format!("{:x}", sha2::Sha224::digest(input.as_bytes())),
            "sha256" => format!("{:x}", sha2::Sha256::digest(input.as_bytes())),
            "sha384" => format!("{:x}", sha2::Sha384::digest(input.as_bytes())),
            "sha512" => format!("{:x}", sha2::Sha512::digest(input.as_bytes())),
            "sha3-224" => format!("{:x}", sha3::Sha3_224::digest(input.as_bytes())),
            "sha3-256" => format!("{:x}", sha3::Sha3_256::digest(input.as_bytes())),
            "sha3-384" => format!("{:x}", sha3::Sha3_384::digest(input.as_bytes())),
            "sha3-512" => format!("{:x}", sha3::Sha3_512::digest(input.as_bytes())),
            other => {
                return Err(mlua::Error::runtime(format!(
                    "crypto.hash: unsupported algorithm: {other} (supported: sha224, sha256, sha384, sha512, sha3-224, sha3-256, sha3-384, sha3-512)"
                )));
            }
        };

        Ok(hex)
    })?;
    crypto_table.set("hash", hash_fn)?;

    let random_fn = lua.create_function(|_, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let length: usize = match args_iter.next() {
            Some(Value::Integer(n)) if n > 0 => n as usize,
            Some(Value::Integer(n)) => {
                return Err(mlua::Error::runtime(format!(
                    "crypto.random: length must be positive, got {n}"
                )));
            }
            Some(Value::Nil) | None => 32,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.random: first argument must be a positive integer or nil",
                ));
            }
        };

        let charset: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let mut rng = rand::rng();
        let result: String = (0..length)
            .map(|_| charset[rng.random_range(..charset.len())] as char)
            .collect();

        Ok(result)
    })?;
    crypto_table.set("random", random_fn)?;

    let hmac_fn = lua.create_function(|lua, args: mlua::MultiValue| {
        let mut args_iter = args.into_iter();

        let key_str: mlua::String = match args_iter.next() {
            Some(Value::String(s)) => s,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.hmac: first argument must be a string (key)",
                ));
            }
        };

        let data_str: mlua::String = match args_iter.next() {
            Some(Value::String(s)) => s,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.hmac: second argument must be a string (data)",
                ));
            }
        };

        let algorithm: String = match args_iter.next() {
            Some(Value::String(s)) => s.to_str()?.to_lowercase(),
            Some(Value::Nil) | None => "sha256".to_string(),
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.hmac: third argument must be an algorithm string or nil",
                ));
            }
        };

        let raw: bool = match args_iter.next() {
            Some(Value::Boolean(b)) => b,
            Some(Value::Nil) | None => false,
            _ => {
                return Err(mlua::Error::runtime(
                    "crypto.hmac: fourth argument must be a boolean or nil",
                ));
            }
        };

        let key_ref = key_str.as_bytes();
        let data_ref = data_str.as_bytes();
        let bytes = compute_hmac_bytes(&key_ref, &data_ref, &algorithm)
            .map_err(|e| {
                mlua::Error::runtime(format!(
                    "crypto.hmac: {e} (supported: sha224, sha256, sha384, sha512, sha3-224, sha3-256, sha3-384, sha3-512)"
                ))
            })?;

        if raw {
            Ok(Value::String(lua.create_string(&bytes)?))
        } else {
            let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
            Ok(Value::String(lua.create_string(hex.as_bytes())?))
        }
    })?;
    crypto_table.set("hmac", hmac_fn)?;

    lua.globals().set("crypto", crypto_table)?;
    Ok(())
}

// Read a Lua opts-table field that accepts either a string or an array of
// strings. Returns Ok(None) if the field is absent or nil. Used by jwt_verify
// for `audience` and `issuer`.
fn lua_string_or_array(
    opts: &mlua::Table,
    field: &str,
    fn_name: &str,
) -> mlua::Result<Option<Vec<String>>> {
    let raw: Value = opts.get(field)?;
    match raw {
        Value::Nil => Ok(None),
        Value::String(s) => Ok(Some(vec![s.to_str()?.to_string()])),
        Value::Table(t) => {
            let mut out = Vec::new();
            for item in t.sequence_values::<mlua::String>() {
                out.push(item?.to_str()?.to_string());
            }
            Ok(Some(out))
        }
        _ => Err(mlua::Error::runtime(format!(
            "{fn_name}: '{field}' must be a string or array of strings"
        ))),
    }
}

// Map a JWK's declared key algorithm to the matching JWT algorithm enum.
// Returns None for algorithms not representable in jsonwebtoken's `Algorithm`
// (none in the current crate version, but kept Option-shaped for forward
// compatibility) so the caller can fall back to the JWT header's `alg`.
fn key_algorithm_to_algorithm(key_alg: KeyAlgorithm) -> Option<Algorithm> {
    Some(match key_alg {
        KeyAlgorithm::HS256 => Algorithm::HS256,
        KeyAlgorithm::HS384 => Algorithm::HS384,
        KeyAlgorithm::HS512 => Algorithm::HS512,
        KeyAlgorithm::RS256 => Algorithm::RS256,
        KeyAlgorithm::RS384 => Algorithm::RS384,
        KeyAlgorithm::RS512 => Algorithm::RS512,
        KeyAlgorithm::PS256 => Algorithm::PS256,
        KeyAlgorithm::PS384 => Algorithm::PS384,
        KeyAlgorithm::PS512 => Algorithm::PS512,
        KeyAlgorithm::ES256 => Algorithm::ES256,
        KeyAlgorithm::ES384 => Algorithm::ES384,
        KeyAlgorithm::EdDSA => Algorithm::EdDSA,
        _ => return None,
    })
}

fn compute_hmac_bytes(key: &[u8], data: &[u8], algorithm: &str) -> Result<Vec<u8>, String> {
    let block_size = match algorithm {
        "sha224" | "sha256" => 64,
        "sha384" | "sha512" => 128,
        "sha3-224" => 144,
        "sha3-256" => 136,
        "sha3-384" => 104,
        "sha3-512" => 72,
        other => return Err(format!("unsupported algorithm: {other}")),
    };

    let hash = |input: &[u8]| -> Vec<u8> {
        match algorithm {
            "sha224" => sha2::Sha224::digest(input).to_vec(),
            "sha256" => sha2::Sha256::digest(input).to_vec(),
            "sha384" => sha2::Sha384::digest(input).to_vec(),
            "sha512" => sha2::Sha512::digest(input).to_vec(),
            "sha3-224" => sha3::Sha3_224::digest(input).to_vec(),
            "sha3-256" => sha3::Sha3_256::digest(input).to_vec(),
            "sha3-384" => sha3::Sha3_384::digest(input).to_vec(),
            "sha3-512" => sha3::Sha3_512::digest(input).to_vec(),
            _ => unreachable!(),
        }
    };

    // Step 1: Derive key — hash if longer than block size, pad with zeros if shorter
    let key_prime = if key.len() > block_size {
        hash(key)
    } else {
        key.to_vec()
    };
    let mut key_padded = key_prime;
    key_padded.resize(block_size, 0);

    // Step 2: Inner hash — H((K' XOR ipad) || data)
    let mut inner = vec![0x36u8; block_size];
    for (i, b) in key_padded.iter().enumerate() {
        inner[i] ^= b;
    }
    inner.extend_from_slice(data);
    let inner_hash = hash(&inner);

    // Step 3: Outer hash — H((K' XOR opad) || inner_hash)
    let mut outer = vec![0x5cu8; block_size];
    for (i, b) in key_padded.iter().enumerate() {
        outer[i] ^= b;
    }
    outer.extend_from_slice(&inner_hash);
    Ok(hash(&outer))
}
