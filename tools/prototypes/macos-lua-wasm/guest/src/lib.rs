//! Compatibility facade: compile Rocket's actual runtime and schema without its
//! native catalog, credential, Git or worker-launching code. The facade changes
//! only the local error representation; the production source files stay intact.
extern crate self as rocket_git;
pub type Result<T> = std::result::Result<T, String>;
pub(crate) fn invalid(message: &str) -> String {
    message.to_owned()
}
pub(crate) fn object(
    value: &serde_json::Value,
) -> Result<&serde_json::Map<String, serde_json::Value>> {
    value
        .as_object()
        .ok_or_else(|| invalid("integration_schema: Expected an object."))
}

#[path = "../../../../../crates/rocket-integrations/src/model.rs"]
mod model;
pub use model::{Node, Request};
#[path = "../../../../../crates/rocket-integrations/src/runtime.rs"]
pub mod runtime;

#[cfg(target_arch = "wasm32")]
mod abi {
    use std::cell::RefCell;
    const INPUT_LIMIT: usize = 8 * 1024 * 1024;
    const OUTPUT_LIMIT: usize = 1024 * 1024;
    thread_local! {
        static INPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
        static OUTPUT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    // Returned offsets refer only to this guest's linear memory. The host must
    // bounds-check each copy; no native pointers or ownership cross the boundary.
    #[unsafe(no_mangle)]
    pub extern "C" fn rocket_input(length: u32) -> u32 {
        if length as usize > INPUT_LIMIT {
            return 0;
        }
        INPUT.with_borrow_mut(|input| {
            input.resize(length as usize, 0);
            input.as_mut_ptr() as u32
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn rocket_run() -> u64 {
        let result = INPUT
            .with_borrow(|bytes| serde_json::from_slice(bytes).ok())
            .map(super::runtime::run)
            .unwrap_or_else(|| serde_json::json!({"kind":"error","code":"input"}));
        OUTPUT.with_borrow_mut(|output| {
            *output = serde_json::to_vec(&result)
                .ok()
                .filter(|bytes| bytes.len() <= OUTPUT_LIMIT)
                .unwrap_or_else(|| b"{\"kind\":\"error\",\"code\":\"result\"}".to_vec());
            ((output.len() as u64) << 32) | u64::from(output.as_ptr() as u32)
        })
    }
}
