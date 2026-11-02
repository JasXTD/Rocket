//! Runs only in rocket-lua-host. The VM has no OS, file, package, bytecode or
//! network APIs. HTTP is a request to the native host, never performed here.
use mlua::chunk::ChunkMode;
use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Table, Value as LValue, VmState};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

const SDK: &str = r#"
local function widget(kind, data)
  if type(data) == 'string' then data = { text = data } end
  data = data or {}
  data.type = kind
  return data
end
local result = {}
for _, kind in ipairs({'page','column','row','card','heading','text','badge','button','input','multiline','select','checkbox','table','separator','progress','split','segments','code'}) do
  result[kind] = function(data) return widget(kind, data) end
end
return result
"#;
fn problem() -> mlua::Error {
    mlua::Error::RuntimeError("bounded_data_required".into())
}
fn to_lua(lua: &Lua, v: &Value, depth: usize) -> mlua::Result<LValue> {
    if depth > 24 {
        return Err(problem());
    }
    Ok(match v {
        Value::Null => LValue::Nil,
        Value::Bool(b) => LValue::Boolean(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                LValue::Integer(i)
            } else {
                LValue::Number(n.as_f64().ok_or_else(problem)?)
            }
        }
        Value::String(s) => LValue::String(lua.create_string(s)?),
        Value::Array(a) => {
            let t = lua.create_table_with_capacity(a.len(), 0)?;
            for (i, x) in a.iter().enumerate() {
                t.raw_set(i + 1, to_lua(lua, x, depth + 1)?)?;
            }
            LValue::Table(t)
        }
        Value::Object(m) => {
            let t = lua.create_table_with_capacity(0, m.len())?;
            for (k, x) in m {
                t.raw_set(k.as_str(), to_lua(lua, x, depth + 1)?)?;
            }
            LValue::Table(t)
        }
    })
}
#[derive(Default)]
struct Conversion {
    nodes: usize,
    bytes: usize,
}
impl Conversion {
    fn charge(&mut self, bytes: usize) -> mlua::Result<()> {
        self.bytes += bytes;
        if self.bytes > 1024 * 1024 {
            return Err(problem());
        }
        Ok(())
    }
}
fn from_lua(v: LValue, depth: usize, count: &mut Conversion) -> mlua::Result<Value> {
    count.nodes += 1;
    count.charge(16)?;
    if depth > 24 || count.nodes > 12000 {
        return Err(problem());
    }
    Ok(match v {
        LValue::Nil => Value::Null,
        LValue::Boolean(b) => Value::Bool(b),
        LValue::Integer(i) => json!(i),
        LValue::Number(n) => Value::Number(serde_json::Number::from_f64(n).ok_or_else(problem)?),
        LValue::String(s) => {
            if s.as_bytes().len() > 256 * 1024 {
                return Err(problem());
            }
            count.charge(s.as_bytes().len())?;
            Value::String(s.to_str()?.to_string())
        }
        LValue::Table(t) => {
            let len = t.raw_len();
            if len > 2000 {
                return Err(problem());
            }
            let mut object = serde_json::Map::new();
            let mut list = vec![];
            let mut entries = 0;
            for pair in t.clone().pairs::<LValue, LValue>() {
                let (k, v) = pair?;
                entries += 1;
                if entries > 2000 {
                    return Err(problem());
                }
                if len > 0 {
                    if !matches!(k,LValue::Integer(i)if i>0&&i as usize<=len) {
                        return Err(problem());
                    }
                } else {
                    let LValue::String(k) = k else {
                        return Err(problem());
                    };
                    if k.as_bytes().len() > 256 {
                        return Err(problem());
                    }
                    count.charge(k.as_bytes().len())?;
                    object.insert(k.to_str()?.to_string(), from_lua(v, depth + 1, count)?);
                }
            }
            if len > 0 {
                if entries != len {
                    return Err(problem());
                }
                for i in 1..=len {
                    list.push(from_lua(t.raw_get(i)?, depth + 1, count)?);
                }
                Value::Array(list)
            } else {
                Value::Object(object)
            }
        }
        _ => return Err(problem()),
    })
}
pub fn run(input: Value) -> Value {
    let pending = Rc::new(RefCell::new(None::<Value>));
    let limited = Rc::new(Cell::new(false));
    let phase = Cell::new("compile");
    let execute = || -> mlua::Result<Value> {
        if input["api"] != 1 {
            return Err(problem());
        }
        let source = input["source"]
            .as_str()
            .filter(|s| s.len() <= 64 * 1024 && !s.starts_with('\u{1b}') && !s.contains('\0'))
            .ok_or_else(problem)?;
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(32 * 1024 * 1024)?;
        let chunk = lua.load(source).set_name("main.lua").set_mode(ChunkMode::Text).into_function()?;
        if input["validate"] == true {
            return Ok(json!({"kind":"validated"}));
        }
        let count = Rc::new(Cell::new(0u32));
        let limit = limited.clone();
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(1000),
            move |_, _| {
                let n = count.get() + 1000;
                count.set(n);
                if n > 2_000_000 {
                    limit.set(true);
                    Err(mlua::Error::RuntimeError("instruction_limit".into()))
                } else {
                    Ok(VmState::Continue)
                }
            },
        )?;
        let globals = lua.globals();
        // Use an explicit base-function allowlist: adding a future standard library
        // function must not silently expand integration permissions.
        let allowed = [
            "_VERSION", "assert", "error", "ipairs", "next", "pairs", "pcall", "select",
            "tonumber", "tostring", "type", "rawequal", "rawlen", "math", "string", "table",
            "utf8",
        ];
        let names: Vec<String> = globals
            .clone()
            .pairs::<String, LValue>()
            .map(|p| p.map(|(k, _)| k))
            .collect::<mlua::Result<_>>()?;
        for name in names {
            if !allowed.contains(&name.as_str()) {
                globals.raw_set(name, LValue::Nil)?;
            }
        }
        let strings: Table = globals.get("string")?;
        strings.raw_set("dump", LValue::Nil)?;
        let math: Table = globals.get("math")?;
        for name in ["random", "randomseed"] {
            math.raw_set(name, LValue::Nil)?;
        }
        let ui: Table = lua
            .load(SDK)
            .set_name("rocket-ui-sdk")
            .set_mode(ChunkMode::Text)
            .eval()?;
        globals.set("ui", ui)?;
        let rocket = lua.create_table()?;
        let responses = input["responses"].as_object().cloned().unwrap_or_default();
        let next = pending.clone();
        rocket.set(
            "http",
            lua.create_function(move |lua, request: Table| {
                let request = from_lua(LValue::Table(request), 0, &mut Conversion::default())?;
                let request = crate::Request::parse(&request)
                    .map_err(|_| mlua::Error::RuntimeError("invalid_http_request".into()))?;
                if let Some(response) = responses.get(&request.key()) {
                    return to_lua(lua, response, 0);
                }
                if next.borrow().is_none() {
                    *next.borrow_mut() = Some(request.value());
                }
                Err(mlua::Error::RuntimeError("native_request_pending".into()))
            })?,
        )?;
        rocket.set(
            "url_encode",
            lua.create_function(|_, s: String| {
                if s.len() > 8192 {
                    return Err(problem());
                }
                let mut out = String::new();
                for b in s.bytes() {
                    if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                        out.push(b as char)
                    } else {
                        use std::fmt::Write;
                        let _ = write!(&mut out, "%{b:02X}");
                    }
                }
                Ok(out)
            })?,
        )?;
        let json_api = lua.create_table()?;
        json_api.set(
            "encode",
            lua.create_function(|_, v: LValue| {
                Ok(from_lua(v, 0, &mut Conversion::default())?.to_string())
            })?,
        )?;
        json_api.set(
            "decode",
            lua.create_function(|lua, s: String| {
                if s.len() > 1024 * 1024 {
                    return Err(problem());
                }
                let v: Value = serde_json::from_str(&s).map_err(|_| problem())?;
                to_lua(lua, &v, 0)
            })?,
        )?;
        rocket.set("json", json_api)?;
        globals.set("rocket", rocket)?;
        phase.set("initialization");
        let callback: Function = chunk.call(())?;
        phase.set("callback");
        let result: LValue = callback.call(to_lua(&lua, &input["context"], 0)?)?;
        let value = from_lua(result, 0, &mut Conversion::default())?;
        if value.get("view").is_some() {
            Ok(
                json!({"kind":"view","view":value["view"],"state":value.get("state").cloned().unwrap_or_else(||json!({}))}),
            )
        } else {
            Ok(json!({"kind":"view","view":value,"state":{}}))
        }
    };
    let result = execute();
    if let Some(request) = pending.borrow_mut().take() {
        return json!({"kind":"request","request":request});
    }
    match result {
        Ok(result) => result,
        Err(e) => {
            // Extract only the numeric source location. Raw Lua errors can carry
            // tokens, form values or response bodies and never leave the worker.
            let description = e.to_string();
            let line = description.find("main.lua").and_then(|offset| {
                let suffix = &description[offset..];
                suffix.split(':').nth(1).and_then(|s| s.parse::<u64>().ok())
            }).filter(|n| (1..=65536).contains(n));
            json!({"kind":"error","code":if limited.get(){"instructions"}else{match e{mlua::Error::SyntaxError{..}=>"syntax",mlua::Error::MemoryError(_)=>"memory",_=>"runtime"}},"phase":phase.get(),"line":line})
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run_source(source: &str) -> Value {
        run(
            json!({"api":1,"source":source,"context":{"event":"open","values":{},"settings":{},"state":{}},"responses":{}}),
        )
    }
    #[test]
    fn native_ui_and_unicode_roundtrip() {
        let v = run_source(
            "return function(ctx) return ui.page{title='資料 🚀',children={ui.input{id='summary',label='Summary',value='é'}}} end",
        );
        assert_eq!(v["kind"], "view");
        assert_eq!(v["view"]["title"], "資料 🚀");
        assert!(crate::Node::parse(&v["view"]).is_ok());
    }
    #[test]
    fn privileged_standard_libraries_are_absent() {
        let v = run_source(
            "return function() assert(os==nil and io==nil and debug==nil and package==nil and require==nil and load==nil and dofile==nil and loadfile==nil and collectgarbage==nil and coroutine==nil and string.dump==nil and _G==nil); return ui.text('bounded') end",
        );
        assert_eq!(v["kind"], "view");
    }
    #[test]
    fn endless_loop_is_bounded() {
        let v = run_source("return function() while true do end end");
        assert_eq!(v["code"], "instructions");
    }
    #[test]
    fn shared_lua_strings_cannot_expand_unbounded_native_data() {
        let v = run_source(
            "return function() local s=string.rep('x',200000) local t={} for i=1,1900 do t[i]=s end return t end",
        );
        assert_eq!(v["kind"], "error");
    }
    #[test]
    fn requests_are_descriptions_not_network_calls() {
        let v = run_source(
            "return function() local r=rocket.http{method='POST',path='/issue',body={summary='資料'}}; return ui.text('sent') end",
        );
        assert_eq!(v["kind"], "request");
        assert_eq!(v["request"]["body"]["summary"], "資料");
    }
    #[test]
    fn syntax_and_cycles_fail_without_payload_errors() {
        assert_eq!(run_source("secret invalid lua")["code"], "syntax");
        assert_eq!(
            run_source("return function() local t={} t.self=t return t end")["kind"],
            "error"
        );
    }
    #[test]
    fn validation_compiles_without_initializing_or_running_the_module() {
        for source in ["while true do end", "error('fixture-private-data')", "return function() rocket.http{method='POST',path='/write'} end"] {
            let result = run(json!({"api":1,"source":source,"validate":true}));
            assert_eq!(result["kind"], "validated");
        }
        let result = run(json!({"api":1,"source":"local private = 'fixture-private-data'\nreturn function(\n", "validate":true}));
        assert_eq!(result["code"], "syntax");
        assert_eq!(result["phase"], "compile");
        assert!(result["line"].as_u64().is_some_and(|n| n >= 2));
        assert!(!result.to_string().contains("fixture-private-data"));
        let result = run_source("return function()\n error('fixture-private-data')\nend");
        assert_eq!(result["phase"], "callback");
        assert_eq!(result["line"], 2);
        assert!(!result.to_string().contains("fixture-private-data"));
    }
}
