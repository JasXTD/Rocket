use crate::{invalid, object};
use rocket_git::Result;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

const TONES: [&str; 5] = ["neutral", "success", "warning", "error", "accent"];

pub fn id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
pub fn text(s: &str, limit: usize) -> bool {
    s.len() <= limit
        && !s
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
}
pub fn array(v: &Value) -> Result<&[Value]> {
    if let Some(a) = v.as_array() {
        Ok(a)
    } else if v.as_object().is_some_and(|o| o.is_empty()) {
        Ok(&[])
    } else {
        Err(invalid("integration_schema: Expected a list."))
    }
}
fn string(v: &Value, key: &str, max: usize) -> Result<String> {
    let s = v[key]
        .as_str()
        .ok_or_else(|| invalid("integration_schema: A required text field is missing."))?;
    if !text(s, max) {
        return Err(invalid(
            "integration_schema: Text is too long or contains control characters.",
        ));
    }
    Ok(s.into())
}
fn keys(v: &Value, allowed: &[&str]) -> Result<()> {
    if object(v)?.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid(
            "integration_schema: Unknown field. Check the API 1 widget or request documentation.",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Setting {
    pub id: String,
    pub label: String,
    pub default: String,
}
impl Setting {
    pub(crate) fn parse(v: &Value) -> Result<Self> {
        keys(v, &["id", "label", "default"])?;
        let setting = Self {
            id: string(v, "id", 64)?,
            label: string(v, "label", 100)?,
            default: v["default"].as_str().unwrap_or("").into(),
        };
        if !id(&setting.id) || !text(&setting.default, 2048) {
            return Err(invalid(
                "integration_manifest: Invalid setting identifier/default.",
            ));
        }
        Ok(setting)
    }
}

/// What a command acts on. Rocket decides where a command shows from this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Commit,
    Branch,
}
impl Target {
    pub fn key(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Branch => "branch",
        }
    }
}
/// A command a package declares. Its label and target come from the manifest alone, so a
/// review has seen every row a menu can show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub target: Option<Target>,
}
impl Command {
    pub(crate) fn parse(v: &Value) -> Result<Self> {
        keys(v, &["id", "label", "target"])?;
        let command_id = v["id"]
            .as_str()
            .filter(|s| id(s))
            .ok_or_else(|| invalid("integration_manifest: A command needs an ASCII ID."))?;
        let label = v["label"]
            .as_str()
            .filter(|s| !s.trim().is_empty() && s.len() <= 64 && !s.chars().any(char::is_control))
            .ok_or_else(|| {
                invalid("integration_manifest: A command needs a one-line label of at most 64 bytes.")
            })?;
        let target = match v.get("target") {
            None => None,
            Some(target) => Some(match target.as_str() {
                Some("commit") => Target::Commit,
                Some("branch") => Target::Branch,
                _ => {
                    return Err(invalid(
                        "integration_manifest: A command's target must be commit or branch.",
                    ));
                }
            }),
        };
        Ok(Self {
            id: command_id.to_owned(),
            label: label.to_owned(),
            target,
        })
    }
}

/// A table cell: plain text, or a pill when it has a tone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub tone: Option<String>,
    pub mono: bool,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: String,
    pub id: Option<String>,
    pub label: String,
    pub value: Value,
    pub children: Vec<Node>,
    pub options: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    /// One per row when a click on a row runs the callback.
    pub keys: Vec<String>,
    /// The key of the row to show as chosen.
    pub selected: Option<String>,
    pub primary: bool,
    pub disabled: bool,
    pub tone: String,
    /// The hint of an empty input.
    pub placeholder: String,
    /// The event Enter runs in an input.
    pub submit: Option<String>,
    /// Secondary text, or a column that keeps its children close.
    pub small: bool,
    /// The pane of a split that a narrow window opens on: 0 or 1.
    pub focus: usize,
}
impl Node {
    /// A callback can change a control's type or available choices while keeping
    /// its ID. Only values valid for the new view may survive that replacement.
    pub fn accepts_value(&self, key: &str, value: &Value) -> bool {
        if self.id.as_deref() == Some(key) {
            return match self.kind.as_str() {
                "input" | "multiline" => value.as_str().is_some_and(|s| s.chars().count() <= 4096 && text(s, 16384)),
                "checkbox" => value.is_boolean(),
                "select" | "segments" => value.as_str().is_some_and(|s| self.options.iter().any(|o| o == s)),
                _ => false,
            };
        }
        self.children.iter().any(|node| node.accepts_value(key, value))
    }
    /// Segments show what the script said, so `ctx.values` follows it; nothing chosen is no
    /// value. This runs once per result: a click stays in `values` until its callback has run.
    pub fn sync_segments(&self, values: &mut HashMap<String, Value>) {
        if self.kind == "segments"
            && let Some(id) = &self.id
        {
            match self.value.as_str().filter(|v| self.options.iter().any(|o| o == v)) {
                Some(value) => {
                    values.insert(id.clone(), json!(value));
                }
                None => {
                    values.remove(id);
                }
            }
        }
        for child in &self.children {
            child.sync_segments(values);
        }
    }
    /// Canonical API data for protected draft storage; always parsed again on load.
    pub fn snapshot(&self) -> Value {
        let mut v = json!({"type":self.kind,"label":self.label,"value":self.value,
            "primary":self.primary,"disabled":self.disabled,"tone":self.tone});
        if let Some(id) = &self.id {
            v["id"] = json!(id);
        }
        if matches!(self.kind.as_str(), "page" | "column" | "row" | "card" | "split") {
            v["children"] = json!(self.children.iter().map(Self::snapshot).collect::<Vec<_>>());
        }
        if matches!(self.kind.as_str(), "select" | "segments") {
            v["options"] = json!(self.options);
        }
        if self.kind == "table" {
            v["columns"] = json!(self.options);
            let cell = |c: &Cell| match (&c.tone, c.mono) {
                (None, false) => json!(c.text),
                (None, true) => json!({"text":c.text,"mono":true}),
                (Some(tone), mono) => json!({"text":c.text,"tone":tone,"mono":mono}),
            };
            v["rows"] = json!(self.rows.iter().map(|row| row.iter().map(cell).collect::<Vec<_>>()).collect::<Vec<_>>());
            if self.clickable() {
                v["keys"] = json!(self.keys);
                if let Some(selected) = &self.selected {
                    v["selected"] = json!(selected);
                }
            }
        }
        if self.kind == "input" {
            v["placeholder"] = json!(self.placeholder);
            if let Some(submit) = &self.submit {
                v["submit"] = json!(submit);
            }
        }
        if matches!(self.kind.as_str(), "text" | "column") {
            v["small"] = json!(self.small);
        }
        if self.kind == "split" {
            v["focus"] = json!(self.focus + 1);
        }
        v
    }
    /// A click on a row runs the callback.
    pub fn clickable(&self) -> bool {
        self.kind == "table" && !self.keys.is_empty()
    }
    /// Whether this view has two panes anywhere, so it can use the page's full width.
    pub fn has_split(&self) -> bool {
        self.kind == "split" || self.children.iter().any(Self::has_split)
    }
    pub fn parse(v: &Value) -> Result<Self> {
        if serde_json::to_vec(v)
            .map_err(|_| invalid("integration_ui: Cannot encode the view."))?
            .len()
            > 512 * 1024
        {
            return Err(invalid("integration_ui_limit: The view exceeds 512 KiB."));
        }
        let mut count = 0;
        let mut ids = HashSet::new();
        Self::parse_node(v, 0, &mut count, &mut ids)
    }
    fn parse_node(
        v: &Value,
        depth: usize,
        count: &mut usize,
        ids: &mut HashSet<String>,
    ) -> Result<Self> {
        *count += 1;
        if depth > 12 || *count > 256 {
            return Err(invalid(
                "integration_ui_limit: Use at most 256 widgets and 12 layout levels.",
            ));
        }
        keys(
            v,
            &[
                "type", "id", "text", "label", "title", "value", "children", "options", "columns",
                "rows", "primary", "disabled", "tone", "keys", "selected", "placeholder", "submit",
                "small", "focus",
            ],
        )?;
        let kind = string(v, "type", 24)?;
        if ![
            "page",
            "column",
            "row",
            "card",
            "heading",
            "text",
            "badge",
            "button",
            "input",
            "multiline",
            "select",
            "checkbox",
            "table",
            "separator",
            "progress",
            "split",
            "segments",
            "code",
        ]
        .contains(&kind.as_str())
        {
            return Err(invalid("integration_ui: Unsupported widget type."));
        }
        // An id on a table without keys has always been ignored, so it stays allowed.
        let interactive = matches!(
            kind.as_str(),
            "button" | "input" | "multiline" | "select" | "checkbox" | "segments"
        ) || (kind == "table" && v.get("keys").is_some());
        let control_id = v["id"].as_str().map(str::to_owned);
        if interactive
            && control_id
                .as_ref()
                .is_none_or(|s| !id(s) || !ids.insert(s.clone()))
        {
            return Err(invalid(
                "integration_ui: Interactive widget IDs must be unique ASCII identifiers.",
            ));
        }
        let label = v["label"]
            .as_str()
            .or_else(|| v["text"].as_str())
            .or_else(|| v["title"].as_str())
            .unwrap_or("")
            .to_owned();
        if !text(&label, 16384) {
            return Err(invalid(
                "integration_ui: Invalid or oversized widget label.",
            ));
        }
        let value = v["value"].clone();
        if !value.is_null()
            && !value.is_boolean()
            && !value.is_number()
            && !value.as_str().is_some_and(|s| text(s, 16384))
        {
            return Err(invalid("integration_ui: Invalid widget value."));
        }
        let tone = v["tone"].as_str().unwrap_or("neutral").to_owned();
        if !TONES.contains(&tone.as_str()) {
            return Err(invalid("integration_ui: Unknown color tone."));
        }
        let mut children = vec![];
        if let Some(c) = v.get("children") {
            if !matches!(kind.as_str(), "page" | "column" | "row" | "card" | "split") {
                return Err(invalid(
                    "integration_ui: Only layout widgets may contain children.",
                ));
            }
            for c in array(c)? {
                children.push(Self::parse_node(c, depth + 1, count, ids)?);
            }
        }
        if kind == "split" && children.len() != 2 {
            return Err(invalid("integration_ui: A split needs exactly two children."));
        }
        let mut options = vec![];
        if let Some(a) = v.get(if kind == "table" {
            "columns"
        } else {
            "options"
        }) {
            for option in array(a)? {
                let s = option
                    .as_str()
                    .filter(|s| text(s, 256))
                    .ok_or_else(|| invalid("integration_ui: Invalid choice or column label."))?;
                options.push(s.into());
            }
            if options.len() > 64 || (kind == "table" && options.len() > 8) {
                return Err(invalid(
                    "integration_ui_limit: Too many choices or table columns.",
                ));
            }
        }
        if kind == "select" && options.is_empty() {
            return Err(invalid(
                "integration_ui: A select needs at least one option.",
            ));
        }
        if kind == "segments" && (options.is_empty() || options.len() > 8) {
            return Err(invalid("integration_ui: Segments need one to eight options."));
        }
        let mut rows = vec![];
        if kind == "table" {
            if options.is_empty() {
                return Err(invalid("integration_ui: A table needs column labels."));
            }
            for row in array(&v["rows"])? {
                let row = array(row)?;
                if row.len() != options.len() {
                    return Err(invalid(
                        "integration_ui: A table row does not match its columns.",
                    ));
                }
                let mut cells = vec![];
                for cell in row {
                    cells.push(parse_cell(cell)?);
                }
                rows.push(cells);
                if rows.len() > 200 {
                    return Err(invalid(
                        "integration_ui_limit: Use pagination for more than 200 table rows.",
                    ));
                }
            }
        }
        let mut row_keys = vec![];
        if kind == "table" && let Some(list) = v.get("keys") {
            for key in array(list)? {
                let key = key
                    .as_str()
                    .filter(|s| !s.is_empty() && text(s, 256))
                    .ok_or_else(|| invalid("integration_ui: Invalid table row key."))?;
                row_keys.push(key.to_owned());
            }
            if row_keys.len() != rows.len() || row_keys.iter().collect::<HashSet<_>>().len() != rows.len() {
                return Err(invalid("integration_ui: A table needs one unique key per row."));
            }
        }
        let selected = v["selected"].as_str().filter(|s| text(s, 256)).map(str::to_owned);
        let placeholder = v["placeholder"].as_str().unwrap_or("").to_owned();
        if !text(&placeholder, 256) {
            return Err(invalid("integration_ui: Invalid input hint."));
        }
        let submit = v["submit"].as_str().map(str::to_owned);
        if submit.as_deref().is_some_and(|s| !id(s)) {
            return Err(invalid("integration_ui: An input's submit must be an ASCII event ID."));
        }
        let focus = match v.get("focus").and_then(Value::as_u64) {
            None | Some(1) => 0,
            Some(2) => 1,
            Some(_) => return Err(invalid("integration_ui: A split's focus is 1 or 2.")),
        };
        Ok(Self {
            kind,
            id: control_id,
            label,
            value,
            children,
            options,
            rows,
            keys: row_keys,
            selected,
            primary: v["primary"].as_bool().unwrap_or(false),
            disabled: v["disabled"].as_bool().unwrap_or(false),
            tone,
            placeholder,
            submit,
            small: v["small"].as_bool().unwrap_or(false),
            focus,
        })
    }
    pub fn control_ids(&self, out: &mut Vec<String>) {
        if let Some(id) = &self.id {
            out.push(id.clone());
        }
        for c in &self.children {
            c.control_ids(out);
        }
    }
}

/// A cell is its text, or an object that adds a tone, a monospace face or both.
fn parse_cell(cell: &Value) -> Result<Cell> {
    let bad = || invalid("integration_ui: Invalid table cell.");
    if let Some(s) = cell.as_str() {
        return if text(s, 2048) {
            Ok(Cell { text: s.into(), tone: None, mono: false })
        } else {
            Err(bad())
        };
    }
    keys(cell, &["text", "tone", "mono"])?;
    let s = cell["text"].as_str().filter(|s| text(s, 2048)).ok_or_else(bad)?;
    let tone = cell["tone"].as_str().map(str::to_owned);
    if tone.as_deref().is_some_and(|t| !TONES.contains(&t)) {
        return Err(invalid("integration_ui: Unknown color tone."));
    }
    Ok(Cell { text: s.into(), tone, mono: cell["mono"].as_bool().unwrap_or(false) })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Option<Value>,
    pub check: Option<Check>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Check {
    pub path: String,
    pub pointer: String,
    pub expected: Value,
}
impl Request {
    pub fn parse(v: &Value) -> Result<Self> {
        keys(v, &["method", "path", "body", "check"])?;
        let method = string(v, "method", 8)?;
        let path = string(v, "path", 8192)?;
        if !["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&method.as_str())
            || !path.starts_with('/')
            || path.starts_with("//")
            || !path.is_ascii()
            || path
                .bytes()
                .any(|b| b <= 32 || b == 127 || matches!(b, b'"' | b'\\' | b'#'))
        {
            return Err(invalid(
                "integration_request: Use a supported method and an encoded relative path on the approved server.",
            ));
        }
        let body = v.get("body").filter(|v| !v.is_null()).cloned();
        if method == "GET" && body.is_some() {
            return Err(invalid("integration_request: GET cannot carry a body."));
        }
        if body
            .as_ref()
            .is_some_and(|v| v.to_string().len() > 256 * 1024)
        {
            return Err(invalid(
                "integration_request_limit: The request body exceeds 256 KiB.",
            ));
        }
        let check = v.get("check").filter(|c| !c.is_null()).map(|c| {
            keys(c, &["path", "pointer", "expected"])?;
            let read = Self::parse(&json!({"method":"GET", "path":c["path"]}))?;
            let pointer = string(c, "pointer", 256)?;
            let expected = c.get("expected").cloned().unwrap_or(Value::Null);
            if method == "GET" || !pointer.starts_with('/') || pointer.chars().any(char::is_control)
                || !(expected.is_string() || expected.is_number() || expected.is_boolean())
                || expected.to_string().len() > 4096
            {
                return Err(invalid("integration_check: A reviewed write check needs a bounded JSON pointer and expected scalar."));
            }
            Ok(Check { path: read.path, pointer, expected })
        }).transpose()?;
        Ok(Self { method, path, body, check })
    }
    pub fn value(&self) -> Value {
        let mut value = json!({"method":self.method,"path":self.path,"body":self.body});
        if let Some(check) = &self.check {
            value["check"] = json!({"path":check.path,"pointer":check.pointer,"expected":check.expected});
        }
        value
    }
    pub fn key(&self) -> String {
        self.value().to_string()
    }
    pub fn writes(&self) -> bool {
        self.method != "GET"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_duplicate_ids_and_unknown_controls() {
        let button = json!({"type":"button","id":"save","label":"Save"});
        assert!(Node::parse(&json!({"type":"column","children":[button.clone(),button]})).is_err());
        assert!(Node::parse(&json!({"type":"webview","html":"x"})).is_err());
    }
    #[test]
    fn a_table_with_keys_runs_the_callback_and_keeps_its_rows_through_a_draft() {
        let view = json!({"type":"table","id":"issues","columns":["Key","Status"],"keys":["A-1","A-2"],"selected":"A-2",
            "rows":[[{"text":"A-1","mono":true},{"text":"Done","tone":"success"}],["A-2","Open"]]});
        let table = Node::parse(&view).unwrap();
        assert!(table.clickable());
        assert_eq!(table.selected.as_deref(), Some("A-2"));
        assert_eq!(table.rows[0][0], Cell { text: "A-1".into(), tone: None, mono: true });
        assert_eq!(table.rows[0][1].tone.as_deref(), Some("success"));
        let again = Node::parse(&table.snapshot()).unwrap();
        assert_eq!(again.rows, table.rows);
        assert_eq!(again.keys, table.keys);
        assert_eq!(again.selected, table.selected);
    }
    #[test]
    fn a_table_needs_one_key_per_row_and_an_id_to_click() {
        for bad in [
            json!({"type":"table","id":"t","columns":["A"],"keys":["1"],"rows":[["x"],["y"]]}),
            json!({"type":"table","id":"t","columns":["A"],"keys":["1","1"],"rows":[["x"],["y"]]}),
            json!({"type":"table","columns":["A"],"keys":["1"],"rows":[["x"]]}),
            json!({"type":"table","id":"t","columns":["A"],"keys":[""],"rows":[["x"]]}),
            json!({"type":"table","columns":["A"],"rows":[[{"text":"x","tone":"loud"}]]}),
            json!({"type":"table","columns":["A"],"rows":[[{"text":"x","size":9}]]}),
        ] {
            assert!(Node::parse(&bad).is_err(), "{bad}");
        }
        // An id on a table without keys has always been ignored.
        let plain = Node::parse(&json!({"type":"table","id":"t","columns":["A"],"rows":[["x"]]})).unwrap();
        assert!(!plain.clickable());
        assert_eq!(plain.rows[0][0].text, "x");
    }
    #[test]
    fn a_split_has_two_panes_and_says_which_a_narrow_window_opens_on() {
        let pane = json!({"type":"card","title":"One"});
        let view = json!({"type":"page","children":[{"type":"split","focus":2,"children":[pane.clone(),pane.clone()]}]});
        let page = Node::parse(&view).unwrap();
        assert!(page.has_split());
        assert_eq!(page.children[0].focus, 1);
        assert_eq!(Node::parse(&page.snapshot()).unwrap().children[0].focus, 1);
        assert!(Node::parse(&json!({"type":"split","children":[pane.clone()]})).is_err());
        assert!(Node::parse(&json!({"type":"split","focus":3,"children":[pane.clone(),pane]})).is_err());
        assert!(!Node::parse(&json!({"type":"column"})).unwrap().has_split());
    }
    #[test]
    fn segments_keep_only_their_own_choices() {
        let segments = Node::parse(&json!({"type":"segments","id":"scope","options":["Mine","All"],"value":""})).unwrap();
        assert!(segments.accepts_value("scope", &json!("All")));
        assert!(!segments.accepts_value("scope", &json!("None")));
        assert_eq!(Node::parse(&segments.snapshot()).unwrap().options, segments.options);
        let mut values = HashMap::from([("scope".to_owned(), json!("Mine"))]);
        let page = Node::parse(&json!({"type":"page","children":[{"type":"segments","id":"scope","options":["Mine","All"],"value":"All"}]})).unwrap();
        page.sync_segments(&mut values);
        assert_eq!(values["scope"], "All", "the script moved it");
        segments.sync_segments(&mut values);
        assert!(values.is_empty(), "nothing chosen is no value");
        assert!(Node::parse(&json!({"type":"segments","options":["Mine"]})).is_err(), "needs an id");
        assert!(Node::parse(&json!({"type":"segments","id":"s","options":[]})).is_err());
        let many: Vec<_> = (0..9).map(|i| i.to_string()).collect();
        assert!(Node::parse(&json!({"type":"segments","id":"s","options":many})).is_err());
    }
    #[test]
    fn an_input_can_hint_and_submit_and_text_can_be_small() {
        let input = Node::parse(&json!({"type":"input","id":"q","placeholder":"Key or words","submit":"search"})).unwrap();
        assert_eq!(input.placeholder, "Key or words");
        assert_eq!(input.submit.as_deref(), Some("search"));
        let again = Node::parse(&input.snapshot()).unwrap();
        assert_eq!(again.submit, input.submit);
        assert_eq!(again.placeholder, input.placeholder);
        assert!(Node::parse(&json!({"type":"input","id":"q","submit":"not an id"})).is_err());
        let small = Node::parse(&json!({"type":"text","text":"x","small":true,"tone":"warning"})).unwrap();
        assert!(small.small);
        assert_eq!(small.tone, "warning");
        assert!(Node::parse(&small.snapshot()).unwrap().small);
        let close = Node::parse(&json!({"type":"column","small":true})).unwrap();
        assert!(Node::parse(&close.snapshot()).unwrap().small);
        assert_eq!(Node::parse(&json!({"type":"code","text":"return 1"})).unwrap().label, "return 1");
    }
    #[test]
    fn a_command_has_an_id_a_one_line_label_and_at_most_a_commit_or_branch_target() {
        let command = Command::parse(&json!({"id":"find","label":"Find issues","target":"commit"})).unwrap();
        assert_eq!(command.target, Some(Target::Commit));
        assert_eq!(Command::parse(&json!({"id":"new","label":"資料"})).unwrap().target, None);
        for bad in [
            json!({"label":"No id"}),
            json!({"id":"has space","label":"x"}),
            json!({"id":"x"}),
            json!({"id":"x","label":"  "}),
            json!({"id":"x","label":"two\nlines"}),
            json!({"id":"x","label":"y".repeat(65)}),
            json!({"id":"x","label":"y","target":"file"}),
            json!({"id":"x","label":"y","surface":"palette"}),
        ] {
            assert!(Command::parse(&bad).is_err(), "{bad}");
        }
    }
    #[test]
    fn unicode_is_content_not_control_identity() {
        assert!(
            Node::parse(
                &json!({"type":"input","id":"summary","label":"資料 🚀","value":"e\u{301}"})
            )
            .is_ok()
        );
        assert!(Node::parse(&json!({"type":"button","id":"資料"})).is_err());
    }
    #[test]
    fn requests_cannot_escape_origin_or_smuggle_headers() {
        for path in [
            "https://other/",
            "//other/",
            "/x\r\nHeader: x",
            "/x#fragment",
            "/\\other",
        ] {
            assert!(Request::parse(&json!({"method":"GET","path":path})).is_err());
        }
        assert!(
            Request::parse(&json!({"method":"GET","path":"/x","headers":{"Authorization":"x"}}))
                .is_err()
        );
    }
    #[test]
    fn write_preconditions_are_bounded_and_cannot_change_origin() {
        let value = json!({"method":"PUT","path":"/issues/1","body":{"summary":"資料"},"check":{"path":"/issues/1?fields=updated","pointer":"/fields/updated","expected":"version-1"}});
        let request = Request::parse(&value).unwrap();
        assert_eq!(request.value(), value);
        for (key, bad) in [("path", json!("//other/issue")), ("path", json!("/x\nHeader: x")), ("pointer", json!("fields/updated")), ("expected", json!({"unbounded":"object"})), ("expected", Value::Null)] {
            let mut candidate = value.clone(); candidate["check"][key] = bad;
            assert!(Request::parse(&candidate).is_err());
        }
        let mut read = value; read["method"] = json!("GET"); read.as_object_mut().unwrap().remove("body");
        assert!(Request::parse(&read).is_err());
    }
    #[test]
    fn changed_control_choices_or_types_refuse_old_values() {
        let choice = Node::parse(&json!({"type":"select","id":"transition","options":["New"],"value":"New"})).unwrap();
        assert!(choice.accepts_value("transition", &json!("New")));
        assert!(!choice.accepts_value("transition", &json!("Old")));
        assert!(!choice.accepts_value("transition", &json!(true)));
        let changed = Node::parse(&json!({"type":"checkbox","id":"transition","value":false})).unwrap();
        assert!(!changed.accepts_value("transition", &json!("New")));
    }
}
