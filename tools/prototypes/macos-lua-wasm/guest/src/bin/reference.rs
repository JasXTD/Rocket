//! Native comparison for checked-in synthetic fixtures ONLY; not a sandbox.
use std::io::{Read, Write};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = Vec::new();
    std::io::stdin()
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut input)?;
    if input.len() > 8 * 1024 * 1024 {
        return Err("input_limit".into());
    }
    let value = serde_json::from_slice(&input)?;
    let result = rocket_lua_wasm_guest_prototype::runtime::run(value);
    std::io::stdout().write_all(&serde_json::to_vec(&result)?)?;
    Ok(())
}
