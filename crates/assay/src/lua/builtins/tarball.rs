use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use mlua::{Lua, Table};
use std::io::Write;

pub fn register_tar(lua: &Lua) -> mlua::Result<()> {
    let tar_table = lua.create_table()?;

    let create_fn = lua.create_function(move |_lua, (output, files, opts): (String, Table, Option<Table>)| {
        let gzip = opts.as_ref()
            .and_then(|t| t.get::<bool>("gzip").ok())
            .unwrap_or(true);

        let output_data: Vec<u8> = if gzip {
            let enc = GzEncoder::new(Vec::new(), Compression::default());
            let mut tar_builder = tar::Builder::new(enc);
            add_files(&mut tar_builder, &files)?;
            let enc = tar_builder.into_inner().map_err(|e| mlua::Error::external(e.to_string()))?;
            enc.finish().map_err(|e| mlua::Error::external(e.to_string()))?
        } else {
            let mut tar_builder = tar::Builder::new(Vec::new());
            add_files(&mut tar_builder, &files)?;
            tar_builder.into_inner().map_err(|e| mlua::Error::external(e.to_string()))?
        };

        std::fs::write(&output, &output_data).map_err(|e| mlua::Error::external(e.to_string()))?;
        Ok(true)
    })?;
    tar_table.set("create", create_fn)?;

    let extract_fn = lua.create_function(move |_lua, (archive, dest): (String, String)| {
        let file = std::fs::read(&archive).map_err(|e| mlua::Error::external(e.to_string()))?;

        if archive.ends_with(".gz") || archive.ends_with(".tgz") {
            let decoder = GzDecoder::new(&file[..]);
            let mut archive = tar::Archive::new(decoder);
            archive.unpack(&dest).map_err(|e| mlua::Error::external(e.to_string()))?;
        } else {
            let mut archive = tar::Archive::new(&file[..]);
            archive.unpack(&dest).map_err(|e| mlua::Error::external(e.to_string()))?;
        }

        Ok(true)
    })?;
    tar_table.set("extract", extract_fn)?;

    let list_fn = lua.create_function(move |_lua, archive: String| {
        let file = std::fs::read(&archive).map_err(|e| mlua::Error::external(e.to_string()))?;
        let mut entries = Vec::new();

        if archive.ends_with(".gz") || archive.ends_with(".tgz") {
            let decoder = GzDecoder::new(&file[..]);
            let mut archive = tar::Archive::new(decoder);
            for entry in archive.entries().map_err(|e| mlua::Error::external(e.to_string()))? {
                let entry = entry.map_err(|e| mlua::Error::external(e.to_string()))?;
                let path = entry.path().map_err(|e| mlua::Error::external(e.to_string()))?;
                entries.push(path.to_string_lossy().to_string());
            }
        } else {
            let mut archive = tar::Archive::new(&file[..]);
            for entry in archive.entries().map_err(|e| mlua::Error::external(e.to_string()))? {
                let entry = entry.map_err(|e| mlua::Error::external(e.to_string()))?;
                let path = entry.path().map_err(|e| mlua::Error::external(e.to_string()))?;
                entries.push(path.to_string_lossy().to_string());
            }
        }

        Ok(entries)
    })?;
    tar_table.set("list", list_fn)?;

    lua.globals().set("tar", tar_table)?;
    Ok(())
}

fn add_files<W: Write>(builder: &mut tar::Builder<W>, files: &Table) -> mlua::Result<()> {
    for pair in files.pairs::<String, mlua::Value>() {
        let (path, value) = pair?;
        let content = match value {
            mlua::Value::String(s) => s.to_str()?.to_string(),
            _ => value.to_string()?,
        };
        let mut header = tar::Header::new_gnu();
        header.set_path(&path).map_err(|e| mlua::Error::external(e.to_string()))?;
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, &path, content.as_bytes())
            .map_err(|e| mlua::Error::external(e.to_string()))?;
    }
    Ok(())
}
