//! Teaching a local coding agent this command surface: `skills`.
//!
//! A `cargo install` leaves a binary and nothing else, so the skill travels
//! inside it and is written out on demand. Installing over an older copy is
//! what updating means, which is why there is no separate verb for it.

use crate::cli::{SkillsArgs, SkillsCommand};
use crate::output::{CliError, CliResult, Out, SkillRecord};
use directories::UserDirs;
use std::path::{Path, PathBuf};

/// The skill as shipped, and what the drift test holds the CLI to.
const SKILL: &str = include_str!("../SKILL.md");

/// The line `uninstall` requires before deleting anything.
const FRONTMATTER_NAME: &str = "name: soulseek-rs";

/// A skills directory to write into, labelled by whose it is.
struct Target {
    agent: &'static str,
    skills: PathBuf,
}

impl Target {
    fn dir(&self) -> PathBuf {
        self.skills.join("soulseek-rs")
    }

    fn file(&self) -> PathBuf {
        self.dir().join("SKILL.md")
    }

    fn record(&self, action: &str) -> SkillRecord {
        SkillRecord {
            agent: self.agent.to_string(),
            path: self.file().display().to_string(),
            action: action.to_string(),
        }
    }
}

pub fn run(out: &Out, command: &SkillsCommand) -> CliResult {
    match command {
        SkillsCommand::Install(args) => install(out, &targets(args)?),
        SkillsCommand::Uninstall(args) => uninstall(out, &targets(args)?),
        SkillsCommand::List(args) => {
            list(out, &targets(args)?);
            Ok(())
        }
    }
}

fn install(out: &Out, targets: &[Target]) -> CliResult {
    for target in targets {
        let file = target.file();
        let action = if current(&file) {
            "unchanged"
        } else {
            let dir = target.dir();
            std::fs::create_dir_all(&dir).map_err(|e| unwritable(&dir, &e))?;
            std::fs::write(&file, SKILL).map_err(|e| unwritable(&file, &e))?;
            "installed"
        };
        out.emit(&target.record(action));
    }
    out.status("restart your agent, or start a new session, to pick it up");
    Ok(())
}

/// Delete only what this command wrote. A mistyped `--dir` names somebody
/// else's directory far more often than it names ours, so the skill file has
/// to identify itself first, and only that file goes: a folder holding
/// anything else survives the empty-directory sweep that follows.
fn uninstall(out: &Out, targets: &[Target]) -> CliResult {
    for target in targets {
        let file = target.file();
        let action = if ours(&file) {
            std::fs::remove_file(&file).map_err(|e| unwritable(&file, &e))?;
            let _ = std::fs::remove_dir(target.dir());
            "removed"
        } else {
            "absent"
        };
        out.emit(&target.record(action));
    }
    Ok(())
}

fn list(out: &Out, targets: &[Target]) {
    for target in targets {
        let action = match std::fs::read_to_string(target.file()) {
            Ok(found) if found == SKILL => "unchanged",
            Ok(_) => "outdated",
            Err(_) => "absent",
        };
        out.emit(&target.record(action));
    }
}

/// Where to write. Naming a directory aims the command at exactly that
/// directory: a run that says where to go should not also touch every agent it
/// happens to find. Detection is what answers the question when nothing does.
fn targets(args: &SkillsArgs) -> CliResult<Vec<Target>> {
    let chosen: Vec<Target> = args
        .dir
        .iter()
        .map(|dir| Target {
            agent: "custom",
            skills: dir.clone(),
        })
        .collect();
    if !chosen.is_empty() {
        return Ok(chosen);
    }

    let home = UserDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .ok_or_else(|| {
            CliError::usage(
                "cannot find your home directory; name a skills directory \
                 with --dir",
            )
        })?;

    let found = detected(&home, &config_home(&home));
    if found.is_empty() {
        return Err(CliError::no_results(
            "no coding agent found in your home directory; name a skills \
             directory with --dir",
        ));
    }
    Ok(found)
}

/// The agents that read a plain `SKILL.md`, kept to the ones actually on this
/// machine: an agent counts as present when its own directory is, not when it
/// already has a skills folder.
fn detected(home: &Path, config_home: &Path) -> Vec<Target> {
    [
        ("claude", home.join(".claude"), "skills"),
        ("opencode", config_home.join("opencode"), "skill"),
    ]
    .into_iter()
    .filter(|(_, agent_dir, _)| agent_dir.is_dir())
    .map(|(agent, agent_dir, skills)| Target {
        agent,
        skills: agent_dir.join(skills),
    })
    .collect()
}

/// Where an agent keeps its own configuration, which is the XDG location on
/// every platform — unlike this client's config, which follows each platform's
/// own convention (see `persist::paths`).
fn config_home(home: &Path) -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| home.join(".config"), PathBuf::from)
}

fn current(file: &Path) -> bool {
    std::fs::read_to_string(file).is_ok_and(|found| found == SKILL)
}

fn ours(file: &Path) -> bool {
    std::fs::read_to_string(file)
        .is_ok_and(|found| found.contains(FRONTMATTER_NAME))
}

fn unwritable(path: &Path, error: &std::io::Error) -> CliError {
    CliError::usage(format!("cannot write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;
    use clap::CommandFactory;

    /// Every command this binary answers, spelled the way a user types it.
    /// Groups contribute their verbs rather than themselves — nobody runs a
    /// bare `room`.
    fn command_paths(command: &clap::Command, prefix: &str) -> Vec<String> {
        let mut paths = Vec::new();
        for sub in command.get_subcommands() {
            if sub.get_name() == "help" {
                continue;
            }
            let path = format!("{prefix}{}", sub.get_name());
            if sub.get_subcommands().next().is_some() {
                paths.extend(command_paths(sub, &format!("{path} ")));
            } else {
                paths.push(path);
            }
        }
        paths
    }

    /// The skill is only worth shipping while it still describes the binary,
    /// so a new command fails the build until it is documented.
    ///
    /// Looking for the name alone would pass on coincidence: half of these are
    /// also JSON keys the document lists, and a command named in passing in the
    /// prose would satisfy it too. What counts is an entry in the command
    /// section, which is the only place written as a bold quoted invocation.
    #[test]
    fn the_skill_documents_every_command() {
        let paths = command_paths(&Cli::command(), "");
        assert!(paths.len() > 10, "expected the full command surface");
        for path in paths {
            assert!(
                SKILL.contains(&format!("**`{path}`"))
                    || SKILL.contains(&format!("**`{path} ")),
                "SKILL.md has no `{path}` entry (expected **`{path} …`**)"
            );
        }
    }

    #[test]
    fn the_skill_tells_agents_to_ask_for_json() {
        assert!(SKILL.contains("--json"));
        assert!(SKILL.contains(FRONTMATTER_NAME));
    }

    #[test]
    fn only_agents_that_are_installed_become_targets() {
        let home = tempfile::tempdir().expect("tempdir");
        let config = home.path().join(".config");
        assert!(detected(home.path(), &config).is_empty());

        std::fs::create_dir_all(home.path().join(".claude")).expect("claude");
        std::fs::create_dir_all(config.join("opencode")).expect("opencode");
        let found = detected(home.path(), &config);

        assert_eq!(found.len(), 2);
        assert_eq!(found[0].skills, home.path().join(".claude/skills"));
        assert_eq!(found[1].skills, config.join("opencode/skill"));
    }

    #[test]
    fn a_file_we_did_not_write_is_never_ours_to_delete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("SKILL.md");
        std::fs::write(&file, "---\nname: something-else\n---\n").expect("w");

        assert!(!ours(&file));
        assert!(!ours(&dir.path().join("missing.md")));

        std::fs::write(&file, SKILL).expect("write");
        assert!(ours(&file));
        assert!(current(&file));
    }
}
