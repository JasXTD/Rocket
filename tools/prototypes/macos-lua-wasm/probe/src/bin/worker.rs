//! Test-only one-shot process. Only the trusted build artifact is deserialized.
use rocket_lua_wasm_probe::{INPUT_LIMIT, engine, invoke_ready};
use std::io::{Read, Write};
use wasmtime::{Error, Module, Result};
static GUEST: &[u8] = include_bytes!(env!("ROCKET_WASM_GUEST"));
fn run() -> Result<Vec<u8>> {
    let mut input = Vec::new();
    std::io::stdin()
        .take((INPUT_LIMIT + 1) as u64)
        .read_to_end(&mut input)?;
    if input.len() > INPUT_LIMIT {
        return Err(Error::msg("input_limit"));
    }
    let e = engine()?;
    // SAFETY: bytes are embedded at build time from this exact locked compiler
    // and engine configuration. No path, Wasm or AOT bytes come from extension
    // input. Production must separately bind/sign its helper and guest package.
    let m = unsafe { Module::deserialize(&e, GUEST) }?;
    invoke_ready(&e, &m, &input, || eprintln!("{{\"event\":\"running\"}}"))
}
fn main() {
    let (output, status) = match run() {
        Ok(bytes) => (bytes, 0),
        Err(error) => {
            let code = match error.downcast_ref::<wasmtime::Trap>() {
                Some(wasmtime::Trap::OutOfFuel) => "wasm_fuel",
                Some(wasmtime::Trap::Interrupt) => "wasm_interrupted",
                Some(wasmtime::Trap::StackOverflow) => "wasm_stack",
                Some(wasmtime::Trap::UnreachableCodeReached) => "wasm_unreachable",
                Some(wasmtime::Trap::UnhandledTag) => "wasm_unhandled_exception",
                Some(_) => "wasm_trap",
                None => "wasm_boundary",
            };
            (
                format!("{{\"kind\":\"error\",\"code\":\"{code}\"}}").into_bytes(),
                2,
            )
        }
    };
    // Never log source text, VM error payloads or user data to stderr.
    if std::io::stdout().write_all(&output).is_err() {
        std::process::exit(3);
    }
    std::process::exit(status);
}
