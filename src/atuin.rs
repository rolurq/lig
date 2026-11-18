//! The source of Ctrl+R: the history atuin keeps (`atuin search`). Atuin's own shell hooks
//! record the commands; lig only reads them.

use crate::engine::{Kind, Suggestion};
use crate::exec::Cmd;
use std::path::Path;
use std::time::Duration;

/// How many matches Ctrl+R shows.
const LIMIT: &str = "50";

/// The `PATH` atuin is looked up and run with. The shell's own is the one that counts: rc files
/// extend it after lig starts. Lig's is the fallback until the shell reports it.
fn shell_path(shell: Option<&str>) -> String {
    shell.map_or_else(|| std::env::var("PATH").unwrap_or_default(), str::to_owned)
}

/// Whether `atuin` is on the shell's `PATH`.
pub fn installed(path: Option<&str>) -> bool {
    crate::help::which("atuin", &shell_path(path)).is_some()
}

/// What `search` looks through.
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    Global,
    /// Only what ran in this folder.
    Dir(std::path::PathBuf),
}

/// The commands matching `line` (atuin's own search mode and ranking), best first; picking
/// one replaces the line. A global search (Ctrl+R) keeps only commands that ran without
/// error; entries atuin imported have no exit status (-1) and stay. A folder search (↑)
/// keeps failed commands too: retyping one after fixing a typo is what it is for. Atuin is
/// found and run with the shell's `path`, and killed after `timeout`: the one worker also
/// answers completions, which a stuck search would hold up.
pub fn search(line: &str, path: Option<&str>, scope: &Scope, timeout: Duration) -> Vec<Suggestion> {
    let query = line.trim_start();
    let mut argv: Vec<String> = [
        "atuin",
        "search",
        "--filter-mode=global",
        "--limit",
        LIMIT,
        "--print0",
        "--format",
        "{relativetime}\t{command}",
    ]
    .map(String::from)
    .into();
    match scope {
        Scope::Global => argv.extend(["--exit=0".into(), "--exit=-1".into()]),
        Scope::Dir(dir) => argv.extend(["--cwd".into(), dir.to_string_lossy().into_owned()]),
    }
    if !query.is_empty() {
        argv.extend(["--".into(), query.into()]);
    }
    let mut env = vec![("PATH".to_string(), shell_path(path))];
    // Atuin aborts without a session, and its shell hook exports one only inside the shell,
    // after lig has started. A global search doesn't use it, so any value will do.
    if std::env::var_os("ATUIN_SESSION").is_none() {
        env.push(("ATUIN_SESSION".into(), "lig".into()));
    }
    let out = crate::exec::run(Cmd::Argv(&argv), Path::new(""), &env, timeout, None);
    if out.status != 0 {
        crate::log(&format!("atuin: exit {}: {}", out.status, out.stderr.trim()));
        return vec![];
    }
    let delete = line.chars().count();
    // Atuin prints the best match last.
    out.stdout
        .split('\0')
        .rev()
        .filter_map(|record| {
            let (age, command) = record.split_once('\t')?;
            Some(Suggestion {
                label: command.replace('\n', " ⏎ "),
                description: age.trim().to_string(),
                kind: Kind::Command,
                icon: Some(crate::spec::Icon::History),
                insert: command.to_string(),
                delete,
                back: 0,
                query: query.to_string(),
            })
        })
        .collect()
}
