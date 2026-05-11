use flate2::write::GzEncoder;
use flate2::Compression;
use futures_util::StreamExt;
use mlua::{Lua, Table};
use oci_distribution::client::{Client as OciClient, ClientConfig};
use oci_distribution::manifest::{OciDescriptor, OciManifest};
use oci_distribution::secrets::RegistryAuth;
use oci_distribution::Reference;
use sha2::{Digest, Sha256};
use std::io::Write;

const LAYER_MEDIA_TYPE: &str = "application/vnd.oci.image.layer.v1.tar+gzip";

pub fn register_oci(lua: &Lua) -> mlua::Result<()> {
    let oci_table = lua.create_table()?;

    let copy_fn = lua.create_async_function(
        move |lua, (src, dst, opts): (String, String, Option<Table>)| async move {
            let (src_auth, dst_auth) = parse_src_dst_auth(&lua, opts.as_ref())?;
            let src_ref = parse_ref(&src)?;
            let dst_ref = parse_ref(&dst)?;
            copy_image(&src_ref, &src_auth, &dst_ref, &dst_auth).await?;
            Ok(true)
        },
    )?;
    oci_table.set("copy", copy_fn)?;

    let tag_fn = lua.create_async_function(
        move |lua, (src, new_tag, opts): (String, String, Option<Table>)| async move {
            let auth = parse_auth(&lua, opts.as_ref(), "auth")?;
            let src_ref = parse_ref(&src)?;
            let dst_ref = Reference::with_tag(
                src_ref.registry().to_string(),
                src_ref.repository().to_string(),
                new_tag,
            );
            retag_image(&src_ref, &dst_ref, &auth).await?;
            Ok(true)
        },
    )?;
    oci_table.set("tag", tag_fn)?;

    let mutate_fn = lua.create_async_function(
        move |lua, (src, dst, files, opts): (String, String, Table, Option<Table>)| async move {
            let (src_auth, dst_auth) = parse_src_dst_auth(&lua, opts.as_ref())?;
            let src_ref = parse_ref(&src)?;
            let dst_ref = parse_ref(&dst)?;
            let collected = collect_files(&files)?;
            mutate_image(&src_ref, &src_auth, &dst_ref, &dst_auth, collected).await?;
            Ok(true)
        },
    )?;
    oci_table.set("mutate", mutate_fn)?;

    lua.globals().set("oci", oci_table)?;
    Ok(())
}

fn parse_ref(s: &str) -> mlua::Result<Reference> {
    s.parse::<Reference>()
        .map_err(|e| mlua::Error::external(format!("invalid image ref {s:?}: {e}")))
}

fn parse_auth(_lua: &Lua, opts: Option<&Table>, key: &str) -> mlua::Result<RegistryAuth> {
    let Some(opts) = opts else {
        return Ok(RegistryAuth::Anonymous);
    };
    let auth_table: Option<Table> = opts.get(key)?;
    let Some(t) = auth_table else {
        return Ok(RegistryAuth::Anonymous);
    };
    let username: Option<String> = t.get("username")?;
    let password: Option<String> = t.get("password")?;
    match (username, password) {
        (Some(u), Some(p)) => Ok(RegistryAuth::Basic(u, p)),
        _ => Ok(RegistryAuth::Anonymous),
    }
}

fn parse_src_dst_auth(
    lua: &Lua,
    opts: Option<&Table>,
) -> mlua::Result<(RegistryAuth, RegistryAuth)> {
    Ok((
        parse_auth(lua, opts, "src_auth")?,
        parse_auth(lua, opts, "dst_auth")?,
    ))
}

fn collect_files(files: &Table) -> mlua::Result<Vec<(String, Vec<u8>)>> {
    let mut out = Vec::new();
    for pair in files.pairs::<String, mlua::Value>() {
        let (path, value) = pair?;
        let bytes = match value {
            mlua::Value::String(s) => s.as_bytes().to_vec(),
            other => other.to_string()?.into_bytes(),
        };
        out.push((path, bytes));
    }
    Ok(out)
}

fn new_client() -> OciClient {
    OciClient::new(ClientConfig::default())
}

fn ext(e: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::external(e.to_string())
}

/// Copy an image from src to dst, one layer at a time.
///
/// Peak memory is the size of the largest single layer (vs the high-level
/// `Client::pull` which buffers every layer, the config, and the manifest
/// concurrently). The OCI distribution crate exposes streaming pull
/// (`pull_blob_stream`) but no streaming push, so each layer is collected
/// into a Vec and the Vec is dropped immediately after `push_blob` returns.
async fn copy_image(
    src: &Reference,
    src_auth: &RegistryAuth,
    dst: &Reference,
    dst_auth: &RegistryAuth,
) -> mlua::Result<()> {
    let client = new_client();
    let (manifest, _src_digest, config_str) = client
        .pull_manifest_and_config(src, src_auth)
        .await
        .map_err(|e| ext(format!("pull manifest {src}: {e}")))?;

    // Prime auth on the destination registry up-front so we get a fast,
    // clear failure if creds are wrong, before doing any heavy I/O.
    client
        .auth(
            dst,
            dst_auth,
            oci_distribution::RegistryOperation::Push,
        )
        .await
        .map_err(|e| ext(format!("auth dst {dst}: {e}")))?;

    for layer in &manifest.layers {
        stream_copy_blob(&client, src, layer, dst).await?;
    }

    client
        .push_blob(dst, config_str.as_bytes(), &manifest.config.digest)
        .await
        .map_err(|e| ext(format!("push config {dst}: {e}")))?;

    client
        .push_manifest(dst, &OciManifest::Image(manifest))
        .await
        .map_err(|e| ext(format!("push manifest {dst}: {e}")))?;

    Ok(())
}

async fn stream_copy_blob(
    client: &OciClient,
    src: &Reference,
    layer: &OciDescriptor,
    dst: &Reference,
) -> mlua::Result<()> {
    let mut stream = client
        .pull_blob_stream(src, layer)
        .await
        .map_err(|e| ext(format!("pull blob {} from {src}: {e}", layer.digest)))?;

    let cap = if layer.size > 0 {
        layer.size as usize
    } else {
        0
    };
    let mut buf = Vec::with_capacity(cap);
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|e| ext(format!("pull blob {}: {e}", layer.digest)))?;
        buf.extend_from_slice(&bytes);
    }

    client
        .push_blob(dst, &buf, &layer.digest)
        .await
        .map_err(|e| ext(format!("push blob {} to {dst}: {e}", layer.digest)))?;

    drop(buf);
    Ok(())
}

/// Retag an image inside a single registry.
///
/// Pull the manifest only (no layers), then push it under the new tag.
/// This is the cheap, correct OCI retag: it does not re-upload blobs.
/// For cross-registry retagging use `oci.copy`.
async fn retag_image(
    src: &Reference,
    dst: &Reference,
    auth: &RegistryAuth,
) -> mlua::Result<()> {
    if src.registry() != dst.registry() || src.repository() != dst.repository() {
        return Err(ext(format!(
            "oci.tag: src and dst must share registry+repository; got {src} and {dst}. Use oci.copy for cross-registry."
        )));
    }
    let client = new_client();
    let (manifest, _digest, _config) = client
        .pull_manifest_and_config(src, auth)
        .await
        .map_err(|e| ext(format!("pull manifest {src}: {e}")))?;
    client
        .push_manifest(dst, &OciManifest::Image(manifest))
        .await
        .map_err(|e| ext(format!("push manifest {dst}: {e}")))?;
    Ok(())
}

/// Append a tar.gz layer of `files` to `src`, push the result to `dst`.
///
/// This is the assay analogue of `crane append + mutate`: it preserves the
/// existing layers and updates the OCI image config so `rootfs.diff_ids`
/// includes the new layer. Without the config update the resulting image
/// is spec-violating and runtimes (containerd/Docker) refuse to run it.
async fn mutate_image(
    src: &Reference,
    src_auth: &RegistryAuth,
    dst: &Reference,
    dst_auth: &RegistryAuth,
    files: Vec<(String, Vec<u8>)>,
) -> mlua::Result<()> {
    let client = new_client();

    let (mut manifest, _src_digest, config_str) = client
        .pull_manifest_and_config(src, src_auth)
        .await
        .map_err(|e| ext(format!("pull manifest {src}: {e}")))?;

    client
        .auth(
            dst,
            dst_auth,
            oci_distribution::RegistryOperation::Push,
        )
        .await
        .map_err(|e| ext(format!("auth dst {dst}: {e}")))?;

    // Copy existing layers from src to dst.
    for layer in &manifest.layers {
        stream_copy_blob(&client, src, layer, dst).await?;
    }

    // Build the new layer: tar (uncompressed) → diff_id, then gzip → blob digest.
    let tar_bytes = build_tar(&files)?;
    let diff_id = format!("sha256:{}", hex_sha256(&tar_bytes));

    let gz_bytes = gzip(&tar_bytes)?;
    drop(tar_bytes);
    let layer_digest = format!("sha256:{}", hex_sha256(&gz_bytes));
    let layer_size = gz_bytes.len() as i64;

    client
        .push_blob(dst, &gz_bytes, &layer_digest)
        .await
        .map_err(|e| ext(format!("push new layer to {dst}: {e}")))?;
    drop(gz_bytes);

    // Update the image config: append diff_id to rootfs.diff_ids, then push.
    let new_config_bytes = append_diff_id(&config_str, &diff_id)?;
    let new_config_digest = format!("sha256:{}", hex_sha256(&new_config_bytes));

    client
        .push_blob(dst, &new_config_bytes, &new_config_digest)
        .await
        .map_err(|e| ext(format!("push updated config to {dst}: {e}")))?;

    manifest.config.digest = new_config_digest;
    manifest.config.size = new_config_bytes.len() as i64;
    manifest.layers.push(OciDescriptor {
        media_type: LAYER_MEDIA_TYPE.to_string(),
        digest: layer_digest,
        size: layer_size,
        ..Default::default()
    });

    client
        .push_manifest(dst, &OciManifest::Image(manifest))
        .await
        .map_err(|e| ext(format!("push manifest {dst}: {e}")))?;

    Ok(())
}

fn build_tar(files: &[(String, Vec<u8>)]) -> mlua::Result<Vec<u8>> {
    let mut buf = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut buf);
        for (path, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).map_err(ext)?;
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, path, content.as_slice()).map_err(ext)?;
        }
        builder.finish().map_err(ext)?;
    }
    Ok(buf)
}

fn gzip(data: &[u8]) -> mlua::Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = GzEncoder::new(&mut out, Compression::default());
        enc.write_all(data).map_err(ext)?;
        enc.finish().map_err(ext)?;
    }
    Ok(out)
}

fn hex_sha256(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

/// Parse an OCI image config JSON, append `diff_id` to `rootfs.diff_ids`,
/// and re-serialise. The output is plain (non-canonical) JSON — that's fine
/// for an image config blob; only the manifest needs canonical encoding,
/// and oci-distribution's `push_manifest` handles that.
fn append_diff_id(config_str: &str, diff_id: &str) -> mlua::Result<Vec<u8>> {
    let mut config: serde_json::Value = serde_json::from_str(config_str)
        .map_err(|e| ext(format!("parse image config: {e}")))?;

    let rootfs = config
        .get_mut("rootfs")
        .ok_or_else(|| ext("image config missing 'rootfs'"))?;
    let diff_ids = rootfs
        .get_mut("diff_ids")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| ext("image config missing 'rootfs.diff_ids' array"))?;
    diff_ids.push(serde_json::Value::String(diff_id.to_string()));

    serde_json::to_vec(&config).map_err(|e| ext(format!("serialize image config: {e}")))
}
