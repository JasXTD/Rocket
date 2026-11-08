//! Offline build-time compiler. Never accepts extensions or runs as the worker.
use std::{fs, path::PathBuf};
use wasmtime::{Error, Module, Result};
fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(
        args.next()
            .ok_or_else(|| Error::msg("Expected fixed guest wasm"))?,
    );
    let output = PathBuf::from(
        args.next()
            .ok_or_else(|| Error::msg("Expected new output directory"))?,
    );
    if args.next().is_some() || output.exists() {
        return Err(Error::msg("Unexpected arguments or existing output"));
    }
    let bytes = fs::read(input)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Error::msg("Module too large"));
    }
    rocket_lua_wasm_probe::validate_guest(&bytes)?;
    let engine = rocket_lua_wasm_probe::engine()?;
    let module = Module::new(&engine, &bytes)?;
    let imports: Vec<_> = module.imports().map(|i| serde_json::json!({"module":i.module(),"name":i.name(),"type":format!("{:?}",i.ty())})).collect();
    let exports: Vec<_> = module
        .exports()
        .map(|i| serde_json::json!({"name":i.name(),"type":format!("{:?}",i.ty())}))
        .collect();
    fs::create_dir(&output)?;
    fs::write(
        output.join("module.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"engine":"wasmtime-50.0.1-pulley64","production_ready":false,"imports":imports,"exports":exports}),
        )?,
    )?;
    fs::write(output.join("guest.cwasm"), module.serialize()?)?;
    println!("Compiled fixed guest to Pulley; module inspection recorded.");
    Ok(())
}
