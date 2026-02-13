use super::json::lua_value_to_json;
use digest::Digest;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
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
