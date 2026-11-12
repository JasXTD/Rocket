//! Literal labels passed through shared controls and model display methods.
//! Keep these synchronized with the Agents view; protocol keys are not translated.
use crate::i18n::mark;

const _: &[&str] = &[
    // What a provider reports (`events::Activity::label`).
    mark("Activity not connected"), mark("Ready for input"), mark("Working"),
    mark("Needs your attention"), mark("Turn finished"), mark("Turn interrupted"),
    mark("Activity connection unavailable"),
    // What a session's process is doing (`Phase::label`).
    mark("Restored"),
    mark("Saving session"),
    mark("Starting"),
    mark("Setting up"),
    mark("Running"),
    mark("Stopping"),
    mark("Stopped"),
    mark("Exited"),
    mark("Failed"),
    // Programs (`Provider::label`) and a terminal's name.
    mark("Claude Code"),
    mark("Codex"),
    mark("Custom command"),
    mark("Setup"),
    // The list's groups and the selected session's tabs.
    mark("Needs you"),
    mark("Not running"),
    mark("Terminal"),
    mark("Changes"),
    mark("Integrate"),
    mark("Details"),
    // Buttons whose label depends on the state.
    mark("Stop"),
    mark("Check integration"),
    mark("Check again"),
    // Where changed files stand (`changes::Area::label`).
    mark("Staged"),
    mark("Unstaged"),
    mark("Untracked"),
    mark("Conflicted"),
    // The session dialog: titles, subtitles, field labels and card titles.
    mark("New agent session"),
    mark("Review and restart"),
    mark("Resuming conversation"),
    mark("Remove worktree"),
    mark("Review session"),
    mark("Its own branch and folder for one task"),
    mark("Check what will be created and run"),
    mark("A new conversation in the same worktree"),
    mark("Check what will run"),
    mark("Task name"),
    mark("Base"),
    mark("New branch"),
    mark("Branch"),
    mark("Worktree folder"),
    mark("Agent executable"),
    mark("Arguments · one per line, no shell expansion"),
    mark("Setup executable"),
    mark("Setup arguments"),
    mark("Your own program"),
    mark("Found"),
    mark("Not found"),
    mark("Creates"),
    mark("Removes"),
    mark("Worktree"),
    // Shown by the shared terminal pane for an agent session.
    mark("Type in this session…"),
];
