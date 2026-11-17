//! The source of Ctrl+R: the history atuin keeps (`atuin search`). Atuin's own shell hooks
//! record the commands; lig only reads them.

use crate::engine::{Kind, Suggestion};
use std::process::{Command, Stdio};

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

/// The commands that ran without error matching `line` (atuin's own search mode and
/// ranking), best first; picking one replaces the line. Entries atuin imported have no exit
/// status (-1): they stay. Atuin is found and run with the shell's `path`.
pub fn search(line: &str, path: Option<&str>, scope: &Scope) -> Vec<Suggestion> {
    let query = line.trim_start();
    let mut cmd = Command::new("atuin");
    cmd.env("PATH", shell_path(path));
    // Atuin aborts without a session, and its shell hook exports one only inside the shell,
    // after lig has started. A global search doesn't use it, so any value will do.
    if std::env::var_os("ATUIN_SESSION").is_none() {
        cmd.env("ATUIN_SESSION", "lig");
    }
    cmd.args([
        "search",
        "--filter-mode=global",
        "--exit=0",
        "--exit=-1",
        "--limit",
        LIMIT,
        "--print0",
        "--format",
        "{relativetime}\t{command}",
    ]);
    if let Scope::Dir(dir) = scope {
        cmd.arg("--cwd").arg(dir);
    }
    if !query.is_empty() {
        cmd.arg("--").arg(query);
    }
    let out = match cmd.stdin(Stdio::null()).stderr(Stdio::null()).output() {
        Ok(out) if out.status.success() => out.stdout,
        Ok(_) => return vec![],
        Err(e) => {
            crate::log(&format!("atuin: {e}"));
            return vec![];
        }
    };
    let delete = line.chars().count();
    // Atuin prints the best match last.
    out.split(|&b| b == 0)
        .rev()
        .filter_map(|record| {
            let record = String::from_utf8_lossy(record);
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
