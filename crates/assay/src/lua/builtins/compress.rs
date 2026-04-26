//! Decompression builtins: gunzip, unxz, unzstd.
//!
//! Each function takes a Lua string (treated as raw bytes) and returns the
//! decompressed bytes as a Lua string. Lua strings in mlua are byte buffers,
//! so binary data round-trips cleanly.

use std::io::Read;

pub fn register_compress(lua: &mlua::Lua) -> mlua::Result<()> {
    let t = lua.create_table()?;
    t.set("gunzip", lua.create_function(gunzip)?)?;
    t.set("unxz", lua.create_function(unxz)?)?;
    t.set("unzstd", lua.create_function(unzstd)?)?;
    lua.globals().set("compress", t)?;
    Ok(())
}

fn gunzip(lua: &mlua::Lua, input: mlua::String) -> mlua::Result<mlua::String> {
    let bytes = input.as_bytes();
    let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| mlua::Error::runtime(format!("compress.gunzip: {e}")))?;
    lua.create_string(&out)
}

fn unxz(lua: &mlua::Lua, input: mlua::String) -> mlua::Result<mlua::String> {
    let bytes = input.as_bytes();
    let mut decoder = xz2::read::XzDecoder::new(&bytes[..]);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| mlua::Error::runtime(format!("compress.unxz: {e}")))?;
    lua.create_string(&out)
}

fn unzstd(lua: &mlua::Lua, input: mlua::String) -> mlua::Result<mlua::String> {
    let bytes = input.as_bytes();
    let out = zstd::stream::decode_all(&bytes[..])
        .map_err(|e| mlua::Error::runtime(format!("compress.unzstd: {e}")))?;
    lua.create_string(&out)
}
