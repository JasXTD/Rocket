//! Test-only Pulley boundary. No production caller or general WASI context.
#[cfg(unix)]
pub mod guard;
use wasmtime::{
    Config, Engine, Error, Instance, Linker, Module, Result, Store, StoreLimits, StoreLimitsBuilder,
};

pub const MEMORY_LIMIT: usize = 128 * 1024 * 1024;
pub const INPUT_LIMIT: usize = 8 * 1024 * 1024;
pub const OUTPUT_LIMIT: usize = 1024 * 1024;
pub const FUEL: u64 = 500_000_000;

pub fn validate_guest(bytes: &[u8]) -> Result<()> {
    use wasmtime::wasmparser::{ExternalKind, Parser, Payload};
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Error::msg("guest_size"));
    }
    let mut memories = 0;
    let mut tables = 0;
    let mut exports = Vec::new();
    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(|_| Error::msg("guest_format"))? {
            Payload::ImportSection(reader) if reader.count() != 0 => {
                return Err(Error::msg("guest_imports"));
            }
            Payload::MemorySection(reader) => {
                for memory in reader {
                    let memory = memory.map_err(|_| Error::msg("guest_memory"))?;
                    memories += 1;
                    if memory.shared
                        || memory.memory64
                        || memory.initial > 2048
                        || memory.maximum != Some(2048)
                        || memory.page_size_log2.is_some()
                    {
                        return Err(Error::msg("guest_memory"));
                    }
                }
            }
            Payload::TableSection(reader) => {
                for table in reader {
                    let table = table.map_err(|_| Error::msg("guest_table"))?;
                    tables += 1;
                    if table.ty.table64
                        || table.ty.shared
                        || table.ty.initial > 4096
                        || table.ty.maximum.is_some_and(|n| n > 4096)
                    {
                        return Err(Error::msg("guest_table"));
                    }
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.map_err(|_| Error::msg("guest_exports"))?;
                    exports.push((export.name.to_owned(), export.kind));
                }
            }
            Payload::StartSection { .. } => return Err(Error::msg("guest_start")),
            _ => {}
        }
    }
    exports.sort_by(|a, b| a.0.cmp(&b.0));
    // Rebuilt WASIp1 std uses the SDK reactor entry point. It only initializes
    // guest state and runs under the same empty linker, fuel and memory limits.
    if exports.first() == Some(&("_initialize".into(), ExternalKind::Func)) {
        exports.remove(0);
    }
    if memories != 1
        || tables > 1
        || exports
            != [
                ("memory".into(), ExternalKind::Memory),
                ("rocket_input".into(), ExternalKind::Func),
                ("rocket_run".into(), ExternalKind::Func),
            ]
    {
        return Err(Error::msg("guest_contract"));
    }
    Ok(())
}

pub fn engine() -> Result<Engine> {
    let mut c = Config::new();
    c.target("pulley64")?
        .consume_fuel(true)
        .epoch_interruption(true)
        .wasm_exceptions(true)
        .wasm_features(wasmtime::WasmFeatures::LEGACY_EXCEPTIONS, false)
        .wasm_multi_memory(false)
        .wasm_memory64(false)
        .wasm_gc(false)
        .wasm_function_references(false)
        .wasm_stack_switching(false)
        .wasm_backtrace_max_frames(None)
        .signals_based_traps(false)
        .memory_reservation(MEMORY_LIMIT as u64)
        .memory_may_move(false)
        .memory_reservation_for_growth(0)
        .memory_guard_size(0)
        .guard_before_linear_memory(false)
        .memory_init_cow(false)
        .gc_heap_initial_size(1024 * 1024)
        .gc_heap_reservation(4 * 1024 * 1024)
        .gc_heap_may_move(false)
        .gc_heap_reservation_for_growth(0)
        .gc_heap_guard_size(0)
        .max_wasm_stack(512 * 1024);
    Engine::new(&c)
}

pub struct State {
    pub limits: StoreLimits,
}
pub fn store(engine: &Engine) -> Result<Store<State>> {
    let limits = StoreLimitsBuilder::new()
        .memory_size(MEMORY_LIMIT)
        .table_elements(4096)
        .instances(1)
        .tables(1)
        .memories(1)
        .build();
    let mut store = Store::new(engine, State { limits });
    store.limiter(|state| &mut state.limits);
    store.set_fuel(FUEL)?;
    store.set_epoch_deadline(1);
    Ok(store)
}

pub fn checked_range(
    offset: u32,
    length: u32,
    memory_size: usize,
    cap: usize,
) -> Result<std::ops::Range<usize>> {
    let start = offset as usize;
    let length = length as usize;
    let end = start
        .checked_add(length)
        .ok_or_else(|| Error::msg("guest_range"))?;
    if length > cap || end > memory_size {
        return Err(Error::msg("guest_range"));
    }
    Ok(start..end)
}

pub fn instantiate(engine: &Engine, module: &Module, store: &mut Store<State>) -> Result<Instance> {
    // Begin with no imports. Any required compatibility functions must be
    // inspected and implemented without ambient authority before this can run.
    Linker::new(engine).instantiate(store, module)
}

pub fn invoke(engine: &Engine, module: &Module, bytes: &[u8]) -> Result<Vec<u8>> {
    invoke_ready(engine, module, bytes, || {})
}

pub fn invoke_ready(
    engine: &Engine,
    module: &Module,
    bytes: &[u8],
    ready: impl FnOnce(),
) -> Result<Vec<u8>> {
    if bytes.len() > INPUT_LIMIT {
        return Err(Error::msg("input_limit"));
    }
    let mut store = store(engine)?;
    let instance = instantiate(engine, module, &mut store)?;
    if instance.get_export(&mut store, "_initialize").is_some() {
        instance
            .get_typed_func::<(), ()>(&mut store, "_initialize")?
            .call(&mut store, ())?;
    }
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| Error::msg("guest_memory"))?;
    let input = instance.get_typed_func::<u32, u32>(&mut store, "rocket_input")?;
    let run = instance.get_typed_func::<(), u64>(&mut store, "rocket_run")?;
    let offset = input.call(&mut store, bytes.len() as u32)?;
    if offset == 0 {
        return Err(Error::msg("guest_input"));
    }
    let range = checked_range(
        offset,
        bytes.len() as u32,
        memory.data_size(&store),
        INPUT_LIMIT,
    )?;
    memory.write(&mut store, range.start, bytes)?;
    ready();
    let packed = run.call(&mut store, ())?;
    let range = checked_range(
        packed as u32,
        (packed >> 32) as u32,
        memory.data_size(&store),
        OUTPUT_LIMIT,
    )?;
    Ok(memory.data(&store)[range].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guest_ranges_reject_wraparound_and_oversized_results() {
        assert!(checked_range(u32::MAX, 8, 128, 32).is_err());
        assert!(checked_range(20, 32, 32, 32).is_err());
        assert!(checked_range(0, 33, 128, 32).is_err());
        assert_eq!(checked_range(16, 16, 32, 32).unwrap(), 16..32);
    }
    #[test]
    fn fixed_guest_contract_refuses_imports_before_compilation() -> Result<()> {
        // Use Wasmtime's approved parser without executing the fixture.
        let bytes =
            wat::parse_str("(module (import \"wasi_snapshot_preview1\" \"path_open\" (func)))")
                .map_err(|_| Error::msg("test_wat"))?;
        assert_eq!(
            validate_guest(&bytes).unwrap_err().to_string(),
            "guest_imports"
        );
        assert!(validate_guest(b"not a module").is_err());
        Ok(())
    }
    #[test]
    fn unauthorized_import_cannot_instantiate() -> Result<()> {
        let e = engine()?;
        let m = Module::new(&e, "(module (import \"env\" \"open_secret\" (func)))")?;
        assert!(instantiate(&e, &m, &mut store(&e)?).is_err());
        Ok(())
    }
    #[test]
    fn reactor_initializes_before_input_and_keeps_the_export_gate() -> Result<()> {
        let e = engine()?;
        let source = r#"(module
            (memory (export "memory") 1 2048)
            (global $ready (mut i32) (i32.const 0))
            (func (export "_initialize") i32.const 1 global.set $ready)
            (func (export "rocket_input") (param i32) (result i32) global.get $ready)
            (func (export "rocket_run") (result i64) i64.const 0))"#;
        let bytes = wat::parse_str(source).map_err(|_| Error::msg("test_wat"))?;
        validate_guest(&bytes)?;
        assert!(invoke(&e, &Module::new(&e, &bytes)?, b"{}").is_ok());
        let other = wat::parse_str(source.replace("_initialize", "unexpected"))
            .map_err(|_| Error::msg("test_wat"))?;
        assert!(validate_guest(&other).is_err());
        let trap = wat::parse_str(source.replace("i32.const 1 global.set $ready", "unreachable"))
            .map_err(|_| Error::msg("test_wat"))?;
        assert!(invoke(&e, &Module::new(&e, &trap)?, b"{}").is_err());
        Ok(())
    }
    #[test]
    fn extra_shared_and_64bit_memories_are_refused() -> Result<()> {
        let e = engine()?;
        for wat in [
            "(module (memory 1) (memory 1))",
            "(module (memory 1 2 shared))",
            "(module (memory i64 1))",
        ] {
            assert!(Module::new(&e, wat).is_err(), "{wat}");
        }
        Ok(())
    }
    #[test]
    fn memory_growth_hits_real_store_limit() -> Result<()> {
        let e = engine()?;
        let m = Module::new(
            &e,
            "(module (memory 1) (func (export \"grow\") (param i32) (result i32) local.get 0 memory.grow))",
        )?;
        let mut s = store(&e)?;
        let i = instantiate(&e, &m, &mut s)?;
        let grow = i.get_typed_func::<i32, i32>(&mut s, "grow")?;
        assert_eq!(grow.call(&mut s, 1)?, 1);
        assert_eq!(grow.call(&mut s, 2048)?, -1);
        Ok(())
    }
    #[test]
    fn table_growth_and_out_of_bounds_access_are_blocked() -> Result<()> {
        let e = engine()?;
        let m = Module::new(
            &e,
            "(module (table 1 funcref) (memory 1) (func (export \"grow\") (result i32) ref.null func i32.const 4096 table.grow) (func (export \"read\") (result i32) i32.const 65536 i32.load))",
        )?;
        let mut s = store(&e)?;
        let i = instantiate(&e, &m, &mut s)?;
        assert_eq!(
            i.get_typed_func::<(), i32>(&mut s, "grow")?
                .call(&mut s, ())?,
            -1
        );
        let error = i
            .get_typed_func::<(), i32>(&mut s, "read")?
            .call(&mut s, ())
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(&wasmtime::Trap::MemoryOutOfBounds)
        );
        Ok(())
    }
    #[test]
    fn fuel_interrupts_wasm_infinite_loop() -> Result<()> {
        let e = engine()?;
        let m = Module::new(&e, "(module (func (export \"run\") (loop br 0)))")?;
        let mut s = store(&e)?;
        s.set_fuel(10_000)?;
        let i = instantiate(&e, &m, &mut s)?;
        let error = i
            .get_typed_func::<(), ()>(&mut s, "run")?
            .call(&mut s, ())
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(&wasmtime::Trap::OutOfFuel)
        );
        Ok(())
    }
    #[test]
    fn epoch_interrupts_without_waiting_for_fuel() -> Result<()> {
        let e = engine()?;
        let m = Module::new(&e, "(module (func (export \"run\") (loop br 0)))")?;
        let mut s = store(&e)?;
        let i = instantiate(&e, &m, &mut s)?;
        e.increment_epoch();
        let error = i
            .get_typed_func::<(), ()>(&mut s, "run")?
            .call(&mut s, ())
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(&wasmtime::Trap::Interrupt)
        );
        Ok(())
    }

    #[test]
    fn resident_linear_pages_stay_bounded_and_fresh_store_recovers() -> Result<()> {
        let e = engine()?;
        let m = Module::new(
            &e,
            r#"(module (memory (export "memory") 1 2048)
            (func (export "fill") (result i32)
                i32.const 2047 memory.grow drop
                i32.const 0 i32.const 90 i32.const 134217728 memory.fill
                i32.const 1 memory.grow))"#,
        )?;
        let mut s = store(&e)?;
        let i = instantiate(&e, &m, &mut s)?;
        assert_eq!(
            i.get_typed_func::<(), i32>(&mut s, "fill")?
                .call(&mut s, ())?,
            -1
        );
        let memory = i.get_memory(&mut s, "memory").unwrap();
        assert_eq!(memory.data_size(&s), MEMORY_LIMIT);
        assert!(memory.data(&s).iter().step_by(4096).all(|b| *b == 90));
        drop(s);
        let mut fresh = store(&e)?;
        let i = instantiate(&e, &m, &mut fresh)?;
        assert_eq!(
            i.get_memory(&mut fresh, "memory").unwrap().data(&fresh)[0],
            0
        );
        Ok(())
    }

    #[test]
    fn retained_exceptions_exhaust_bounded_heap_then_fresh_store_works() -> Result<()> {
        let e = engine()?;
        // Large retained payloads reach the 4 MiB exception heap before fuel or
        // the 4096-element table. Never request an unbounded native allocation.
        let parameters = "i64 ".repeat(512);
        let payload = "i64.const 1 ".repeat(512);
        let source = format!(
            r#"(module
            (tag $e (param {parameters})) (table 4096 exnref)
            (global (export "count") (mut i32) (i32.const 0))
            (func (export "retain")
                (loop $again
                    global.get 0
                    (block $caught (result exnref)
                        (try_table (catch_all_ref $caught) {payload} throw $e)
                        unreachable)
                    table.set
                    global.get 0 i32.const 1 i32.add global.set 0
                    br $again)))"#
        );
        let m = Module::new(&e, source)?;
        for _ in 0..2 {
            let mut s = store(&e)?;
            let i = instantiate(&e, &m, &mut s)?;
            let error = i
                .get_typed_func::<(), ()>(&mut s, "retain")?
                .call(&mut s, ())
                .unwrap_err();
            assert!(error.is::<wasmtime::GcHeapOutOfMemory<()>>(), "{error:?}");
            let count = i
                .get_global(&mut s, "count")
                .unwrap()
                .get(&mut s)
                .i32()
                .unwrap();
            assert!(count > 0 && count < 4096, "{count}");
        }
        Ok(())
    }

    #[test]
    fn repeated_catch_releases_exceptions_and_fuel_still_interrupts() -> Result<()> {
        let e = engine()?;
        let m = Module::new(
            &e,
            r#"(module (tag $e)
            (global (export "count") (mut i32) (i32.const 0))
            (func (export "run")
                (loop $again
                    (block $caught (try_table (catch $e $caught) throw $e))
                    global.get 0 i32.const 1 i32.add global.set 0
                    br $again)))"#,
        )?;
        let mut s = store(&e)?;
        s.set_fuel(2_000_000)?;
        let i = instantiate(&e, &m, &mut s)?;
        let error = i
            .get_typed_func::<(), ()>(&mut s, "run")?
            .call(&mut s, ())
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(&wasmtime::Trap::OutOfFuel)
        );
        assert!(
            i.get_global(&mut s, "count")
                .unwrap()
                .get(&mut s)
                .i32()
                .unwrap()
                > 10_000
        );
        Ok(())
    }
}
