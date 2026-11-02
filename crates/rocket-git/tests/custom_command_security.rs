//! Pure definition checks use inert path strings and never contact network shares.
use rocket_git::*;
use std::{collections::BTreeMap, path::PathBuf};

fn command() -> CustomCommand {
    CustomCommand {
        name: "Review 文 🚀".into(),
        program: if cfg!(windows) {
            r"C:\Tools\fixture.exe"
        } else {
            "/tools/fixture"
        }
        .into(),
        arguments: "show\n{input:text}".into(),
        inputs: vec![CommandInput {
            key: "text".into(),
            label: "Text".into(),
            default_value: "ordinary".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn hidden_characters_cannot_enter_any_definition_field() {
    for hidden in [
        '\u{ad}', '\u{61c}', '\u{200b}', '\u{200c}', '\u{200d}', '\u{200e}', '\u{200f}',
        '\u{2028}', '\u{2029}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}', '\u{202e}',
        '\u{2060}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}', '\u{feff}', '\u{fff9}',
        '\u{fffb}',
    ] {
        for field in 0..7 {
            let mut c = command();
            match field {
                0 => c.name.push(hidden),
                1 => c.group = format!("Tools{hidden}/Review"),
                2 => c.program = c.program.replace("fixture", &format!("fix{hidden}ture")),
                3 => c.arguments.push_str(&format!("\n--opt{hidden}ion")),
                4 => c.inputs[0].label.push(hidden),
                5 => c.inputs[0].default_value.push(hidden),
                _ => c.inputs[0].checked_value = format!("--flag{hidden}"),
            }
            assert!(
                c.validate().is_err(),
                "field {field} accepted U+{:04X}",
                hidden as u32
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn imported_network_and_device_program_paths_are_rejected_without_io() {
    for path in [
        r"\\example.invalid\share\tool.exe",
        "//example.invalid/share/tool.exe",
        r"\\example.invalid@SSL\DavWWWRoot\tool.exe",
        r"\\?\UNC\example.invalid\share\tool.exe",
        r"\\.\UNC\example.invalid\share\tool.exe",
        r"\\.\pipe\tool.exe",
        r"\\?\GLOBALROOT\Device\tool.exe",
        r"\??\UNC\example.invalid\share\tool.exe",
        r"C:\Tools\file:stream.exe",
        r"C:relative.exe",
        r"\rooted.exe",
    ] {
        let mut c = command();
        c.program = path.into();
        assert!(c.validate().is_err(), "accepted {path:?}");
    }
    for path in [
        r"C:\Tools\fixture.exe",
        r"C:/Tools/文 tool.EXE",
        r"\\?\C:\Tools\fixture.exe",
    ] {
        let mut c = command();
        c.program = path.into();
        assert!(c.validate().is_ok(), "{path:?}");
    }
}

#[test]
fn ordinary_unicode_and_line_separators_preserve_literal_arguments() {
    let mut c = command();
    c.group = "工具 / العربية".into();
    c.inputs[0].label = "עברית".into();
    c.arguments = "show\r\n{input:text}\r\n--literal=a;b && c\r\n".into();
    c.inputs[0].default_value = "作者 العربية עברית 界 🚀".into();
    c.validate().unwrap();
    let context = CommandContext {
        repository: PathBuf::from(if cfg!(windows) { r"C:\repo" } else { "/repo" }),
        ..Default::default()
    };
    let expanded = c.expand(&context).unwrap();
    assert_eq!(
        expanded,
        vec!["show", "作者 العربية עברית 界 🚀", "--literal=a;b && c"]
            .into_iter()
            .map(std::ffi::OsString::from)
            .collect::<Vec<_>>()
    );
}

#[test]
fn runtime_form_values_cannot_hide_arguments() {
    let c = command();
    let context = CommandContext::default();
    for value in [
        "visible\u{202e}hidden",
        "visible\u{200b}hidden",
        "visible\u{2066}hidden",
    ] {
        let values = BTreeMap::from([("text".into(), value.into())]);
        assert!(c.expand_with_values(&context, &values).is_err());
    }
}

#[test]
fn repeated_defaults_cannot_expand_beyond_the_existing_execution_limit() {
    let mut c = command();
    c.inputs[0].default_value = "x".repeat(4096);
    c.arguments = "{input:text}".repeat(1000);
    c.validate().unwrap();
    assert!(
        c.expand(&CommandContext::default())
            .unwrap_err()
            .contains("32000")
    );
    c.arguments = "{input:text}".repeat(7);
    assert!(c.expand(&CommandContext::default()).is_ok());
    let shown = custom_command_display("文\u{202e}\\u{202e}");
    assert_eq!(shown, r"文\u{202e}\\u{202e}");
}
