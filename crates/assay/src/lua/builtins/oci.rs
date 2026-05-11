use mlua::{Lua, Table};
use oci_distribution::client::{Client as OciClient, ClientConfig, ImageLayer};
use oci_distribution::secrets::RegistryAuth;
use oci_distribution::Reference;

pub fn register_oci(lua: &Lua) -> mlua::Result<()> {
    let oci_table = lua.create_table()?;

    let copy_fn = lua.create_async_function(move |_lua, (src, dst, opts): (String, String, Option<Table>)| {
        async move {
            let opts = opts.unwrap_or_else(|| _lua.create_table().unwrap());
            let src_auth = parse_auth(&opts, "src_auth")?;
            let dst_auth = parse_auth(&opts, "dst_auth")?;

            let config = ClientConfig::default();
            let client = OciClient::new(config);

            let src_ref: Reference = src.parse().map_err(|e: oci_distribution::ParseError| mlua::Error::external(e.to_string()))?;
            let dst_ref: Reference = dst.parse().map_err(|e: oci_distribution::ParseError| mlua::Error::external(e.to_string()))?;

            let image = client
                .pull(&src_ref, &src_auth, vec!["*/*"])
                .await
                .map_err(|e| mlua::Error::external(format!("pull failed: {e}")))?;

            client
                .push(&dst_ref, &image.layers, image.config, &dst_auth, image.manifest)
                .await
                .map_err(|e| mlua::Error::external(format!("push failed: {e}")))?;

            Ok(true)
        }
    })?;
    oci_table.set("copy", copy_fn)?;

    let tag_fn = lua.create_async_function(move |_lua, (src, new_tag): (String, String)| {
        async move {
            let client = OciClient::new(ClientConfig::default());
            let src_ref: Reference = src.parse().map_err(|e: oci_distribution::ParseError| mlua::Error::external(e.to_string()))?;
            let new_ref = Reference::with_tag(src_ref.registry().to_string(), src_ref.repository().to_string(), new_tag);
            let auth = RegistryAuth::Anonymous;

            let image = client
                .pull(&src_ref, &auth, vec!["*/*"])
                .await
                .map_err(|e| mlua::Error::external(format!("pull failed: {e}")))?;

            client
                .push(&new_ref, &image.layers, image.config, &auth, image.manifest)
                .await
                .map_err(|e| mlua::Error::external(format!("tag push failed: {e}")))?;

            Ok(true)
        }
    })?;
    oci_table.set("tag", tag_fn)?;

    let mutate_fn = lua.create_async_function(move |_lua, (src, dst, files, opts): (String, String, Table, Option<Table>)| {
        async move {
            let opts = opts.unwrap_or_else(|| _lua.create_table().unwrap());
            let src_auth = parse_auth(&opts, "src_auth")?;
            let dst_auth = parse_auth(&opts, "dst_auth")?;

            let client = OciClient::new(ClientConfig::default());
            let src_ref: Reference = src.parse().map_err(|e: oci_distribution::ParseError| mlua::Error::external(e.to_string()))?;
            let dst_ref: Reference = dst.parse().map_err(|e: oci_distribution::ParseError| mlua::Error::external(e.to_string()))?;

            let mut image = client
                .pull(&src_ref, &src_auth, vec!["*/*"])
                .await
                .map_err(|e| mlua::Error::external(format!("pull failed: {e}")))?;

            // Create a new layer with the provided files as a tar.gz
            let mut tarbuf = Vec::new();
            {
                let enc = flate2::write::GzEncoder::new(&mut tarbuf, flate2::Compression::default());
                let mut tar_builder = tar::Builder::new(enc);
                for pair in files.pairs::<String, mlua::String>() {
                    let (path, content) = pair?;
                    let content_str = content.to_str()?;
                    let mut header = tar::Header::new_gnu();
                    header.set_path(&path).map_err(|e| mlua::Error::external(e.to_string()))?;
                    header.set_size(content_str.len() as u64);
                    header.set_mode(0o644);
                    header.set_cksum();
                    tar_builder
                        .append_data(&mut header, &path, content_str.as_bytes())
                        .map_err(|e| mlua::Error::external(e.to_string()))?;
                }
                let enc = tar_builder.into_inner().map_err(|e| mlua::Error::external(e.to_string()))?;
                enc.finish().map_err(|e| mlua::Error::external(e.to_string()))?;
            }

            let new_layer = ImageLayer::new(tarbuf, "application/vnd.oci.image.layer.v1.tar+gzip".to_string(), None);
            image.layers.push(new_layer);

            client
                .push(&dst_ref, &image.layers, image.config, &dst_auth, image.manifest)
                .await
                .map_err(|e| mlua::Error::external(format!("mutate push failed: {e}")))?;

            Ok(true)
        }
    })?;
    oci_table.set("mutate", mutate_fn)?;

    lua.globals().set("oci", oci_table)?;
    Ok(())
}

fn parse_auth(opts: &Table, key: &str) -> mlua::Result<RegistryAuth> {
    let auth_table: Option<Table> = opts.get(key)?;
    match auth_table {
        Some(t) => {
            let username: Option<String> = t.get("username")?;
            let password: Option<String> = t.get("password")?;
            match (username, password) {
                (Some(u), Some(p)) => Ok(RegistryAuth::Basic(u, p)),
                _ => Ok(RegistryAuth::Anonymous),
            }
        }
        None => Ok(RegistryAuth::Anonymous),
    }
}
