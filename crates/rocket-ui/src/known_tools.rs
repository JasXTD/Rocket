//! Ready-made arguments for tools people already have. Choosing one only fills in the
//! profile form: nothing here runs a program, and finding one only looks for a file in its
//! usual install folders. Each line is one argument, and a placeholder is a whole line, so
//! a tool must take its paths as separate arguments.
use rocket_git::ExternalToolKind;
use std::path::PathBuf;

pub struct KnownTool {
    /// Its automation name, as in `tools-known-vscode`.
    pub id: &'static str,
    pub name: &'static str,
    /// The program, which must stay open until the tool closes. Launchers that hand the
    /// file to a running copy, such as Beyond Compare's BCompare.exe, can't be used.
    pub file: &'static str,
    /// An environment variable and the folder under it, in the order they are tried.
    folders: &'static [(&'static str, &'static str)],
    editor: &'static [&'static str],
    diff: &'static [&'static str],
    merge: &'static [&'static str],
}

const PROGRAMS: &str = "ProgramFiles";
const PROGRAMS_X86: &str = "ProgramFiles(x86)";
const LOCAL: &str = "LOCALAPPDATA";

pub const TOOLS: [KnownTool; 11] = [
    KnownTool {
        id: "vscode",
        name: "VS Code",
        file: "Code.exe",
        folders: &[(LOCAL, r"Programs\Microsoft VS Code"), (PROGRAMS, "Microsoft VS Code")],
        // -n gives the tool its own window, which closing ends the wait.
        editor: &["-n", "--wait", "{file}"],
        diff: &["-n", "--wait", "--diff", "{left}", "{right}"],
        merge: &["-n", "--wait", "--merge", "{theirs}", "{ours}", "{base}", "{result}"],
    },
    KnownTool {
        id: "sublime-text",
        name: "Sublime Text",
        file: "subl.exe",
        folders: &[(PROGRAMS, "Sublime Text")],
        editor: &["--new-window", "--wait", "{file}"],
        diff: &[],
        merge: &[],
    },
    KnownTool {
        id: "notepad-plus-plus",
        name: "Notepad++",
        file: "notepad++.exe",
        folders: &[(PROGRAMS, "Notepad++"), (PROGRAMS_X86, "Notepad++")],
        // Without -multiInst the file goes to a running copy and this one exits.
        editor: &["-multiInst", "-nosession", "{file}"],
        diff: &[],
        merge: &[],
    },
    KnownTool {
        id: "beyond-compare",
        name: "Beyond Compare",
        file: "BComp.exe",
        folders: &[(PROGRAMS, "Beyond Compare 5"), (PROGRAMS, "Beyond Compare 4")],
        editor: &[],
        diff: &["{left}", "{right}"],
        merge: &["{ours}", "{theirs}", "{base}", "{result}"],
    },
    KnownTool {
        id: "winmerge",
        name: "WinMerge",
        file: "WinMergeU.exe",
        folders: &[(PROGRAMS, "WinMerge"), (PROGRAMS_X86, "WinMerge"), (LOCAL, r"Programs\WinMerge")],
        // /s- keeps it out of a running copy when that option is on.
        editor: &[],
        diff: &["/e", "/u", "/s-", "/wl", "/wr", "{left}", "{right}"],
        merge: &["/e", "/u", "/s-", "{theirs}", "{base}", "{ours}", "/o", "{result}"],
    },
    KnownTool {
        id: "kdiff3",
        name: "KDiff3",
        file: "kdiff3.exe",
        folders: &[(PROGRAMS, r"KDiff3\bin"), (PROGRAMS, "KDiff3")],
        editor: &[],
        diff: &["{left}", "{right}"],
        merge: &["{base}", "{ours}", "{theirs}", "-o", "{result}"],
    },
    KnownTool {
        id: "meld",
        name: "Meld",
        file: "Meld.exe",
        folders: &[(PROGRAMS, "Meld"), (PROGRAMS_X86, "Meld"), (LOCAL, r"Programs\Meld")],
        editor: &[],
        diff: &["{left}", "{right}"],
        merge: &["--output", "{result}", "{ours}", "{base}", "{theirs}"],
    },
    KnownTool {
        id: "p4merge",
        name: "P4Merge",
        file: "p4merge.exe",
        folders: &[(PROGRAMS, "Perforce")],
        editor: &[],
        diff: &["{left}", "{right}"],
        merge: &["{base}", "{theirs}", "{ours}", "{result}"],
    },
    KnownTool {
        id: "tortoisegitmerge",
        name: "TortoiseGitMerge",
        file: "TortoiseGitMerge.exe",
        folders: &[(PROGRAMS, r"TortoiseGit\bin")],
        editor: &[],
        diff: &["{left}", "{right}"],
        // Git's own mergetool uses this form; /merged:path can't be a placeholder line.
        merge: &["-base", "{base}", "-mine", "{ours}", "-theirs", "{theirs}", "-merged", "{result}"],
    },
    KnownTool {
        id: "sublime-merge",
        name: "Sublime Merge",
        file: "smerge.exe",
        folders: &[(PROGRAMS, "Sublime Merge")],
        editor: &[],
        diff: &[],
        merge: &["mergetool", "{base}", "{ours}", "{theirs}", "-o", "{result}"],
    },
    KnownTool {
        id: "araxis-merge",
        name: "Araxis Merge",
        file: "Compare.exe",
        folders: &[(PROGRAMS, r"Araxis\Araxis Merge")],
        editor: &[],
        diff: &["/wait", "/2", "{left}", "{right}"],
        merge: &["/wait", "/3", "/a2", "{theirs}", "{base}", "{ours}", "{result}"],
    },
];

impl KnownTool {
    pub fn arguments(&self, kind: ExternalToolKind) -> &'static [&'static str] {
        match kind {
            ExternalToolKind::Editor => self.editor,
            ExternalToolKind::Diff => self.diff,
            ExternalToolKind::Merge => self.merge,
        }
    }
    /// Its program in the usual folders. PATH and the registry are left out: a tool
    /// elsewhere is simply not found, and Browse still reaches it.
    pub fn find(&self) -> Option<PathBuf> {
        self.folders
            .iter()
            .filter_map(|(variable, folder)| {
                let root = std::env::var_os(variable)?;
                Some(PathBuf::from(root).join(folder).join(self.file))
            })
            .find(|path| path.is_file())
    }
}

/// The tools with arguments for this kind.
pub fn offered(kind: ExternalToolKind) -> impl Iterator<Item = &'static KnownTool> {
    TOOLS.iter().filter(move |tool| !tool.arguments(kind).is_empty())
}

/// The tool a profile was filled from, going by its name.
pub fn named(name: &str, kind: ExternalToolKind) -> Option<&'static KnownTool> {
    offered(kind).find(|tool| tool.name.eq_ignore_ascii_case(name.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_profiles::KINDS;
    use rocket_git::ExternalToolConfig;

    #[test]
    fn every_listed_line_passes_the_profile_check() {
        for kind in KINDS {
            assert!(offered(kind).count() >= 3, "{kind:?}");
            for tool in offered(kind) {
                let config = ExternalToolConfig {
                    executable: format!(r"C:\Tools\{}", tool.file),
                    arguments: tool.arguments(kind).join("\n"),
                };
                config.validate(kind).unwrap_or_else(|_| panic!("{} {kind:?}", tool.name));
            }
        }
    }
    #[test]
    fn the_merge_lines_match_each_tools_documented_order() {
        let merge = |id: &str| TOOLS.iter().find(|t| t.id == id).unwrap().merge.join(" ");
        assert_eq!(merge("vscode"), "-n --wait --merge {theirs} {ours} {base} {result}");
        assert_eq!(merge("beyond-compare"), "{ours} {theirs} {base} {result}");
        assert_eq!(merge("kdiff3"), "{base} {ours} {theirs} -o {result}");
        assert_eq!(merge("meld"), "--output {result} {ours} {base} {theirs}");
        assert_eq!(merge("p4merge"), "{base} {theirs} {ours} {result}");
        assert_eq!(merge("sublime-merge"), "mergetool {base} {ours} {theirs} -o {result}");
    }
    #[test]
    fn ids_and_names_are_unique_and_every_program_is_an_exe() {
        for (i, tool) in TOOLS.iter().enumerate() {
            assert!(tool.file.ends_with(".exe"), "{}", tool.name);
            assert!(!tool.folders.is_empty(), "{}", tool.name);
            for other in &TOOLS[i + 1..] {
                assert_ne!(tool.id, other.id);
                assert_ne!(tool.name, other.name);
            }
        }
    }
    #[test]
    fn a_program_is_found_only_in_its_folders_and_only_as_a_file() {
        let root = std::env::temp_dir().join(format!("rocket-known-tools-{}", std::process::id()));
        let tool = &TOOLS[0];
        assert_eq!(tool.id, "vscode");
        let folder = root.join(r"Programs\Microsoft VS Code");
        std::fs::create_dir_all(folder.join(tool.file)).unwrap();
        let found = |root: &std::path::Path| {
            [root.join(tool.folders[0].1).join(tool.file)].into_iter().find(|p| p.is_file())
        };
        assert_eq!(found(&root), None, "a folder with that name is not the program");
        std::fs::remove_dir(folder.join(tool.file)).unwrap();
        std::fs::write(folder.join(tool.file), b"never run").unwrap();
        assert_eq!(found(&root), Some(folder.join(tool.file)));
        std::fs::remove_dir_all(root).unwrap();
    }
}
