//! Tab completion for the shell: `completions`.
//!
//! The script is generated from the same `clap` definition the binary parses
//! with, so it never drifts. Installing means putting it where the shell
//! already looks — which is enough on its own only for fish; bash and zsh each
//! get one `source` line appended to their rc file, marked so `uninstall` can
//! take back exactly what was added and nothing else.

use crate::cli::{Cli, CompletionsArgs, CompletionsCommand, Shell};
use crate::output::{CliError, CliResult, CompletionRecord, Out};
use clap::CommandFactory;
use directories::UserDirs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The command being completed, and the file name the scripts are written as.
const BIN: &str = "soulseek-rs";

/// What every line this command writes into an rc file ends with, and the only
/// thing `uninstall` will delete from one.
const MARKER: &str = "# soulseek-rs completions";

/// The directories the install locations hang off, resolved once so the rest
/// of the module is a pure function of them.
struct Dirs {
    home: PathBuf,
    data: PathBuf,
    config: PathBuf,
}

/// One shell's completion script, and the rc file that has to load it.
struct Target {
    shell: Shell,
    script: PathBuf,
    /// `None` for fish, whose completions directory is scanned automatically.
    rc: Option<PathBuf>,
}

impl Target {
    fn record(&self, action: &str) -> CompletionRecord {
        CompletionRecord {
            shell: self.shell.name().to_string(),
            path: self.script.display().to_string(),
            action: action.to_string(),
        }
    }

    fn source_line(&self) -> String {
        format!("source \"{}\" {MARKER}", self.script.display())
    }
}

impl Shell {
    const fn name(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }

    const fn generator(self) -> clap_complete::Shell {
        match self {
            Self::Bash => clap_complete::Shell::Bash,
            Self::Zsh => clap_complete::Shell::Zsh,
            Self::Fish => clap_complete::Shell::Fish,
        }
    }
}

pub fn run(out: &Out, command: &CompletionsCommand) -> CliResult {
    match command {
        CompletionsCommand::Install(args) => install(out, &targets(args)?),
        CompletionsCommand::Uninstall(args) => uninstall(out, &targets(args)?),
    }
}

fn install(out: &Out, targets: &[Target]) -> CliResult {
    for target in targets {
        let mut changed = write_script(&target.script, &script(target.shell))?;
        if let Some(rc) = &target.rc {
            changed |= add_source_line(rc, &target.source_line())?;
        }
        out.emit(&target.record(if changed {
            "installed"
        } else {
            "unchanged"
        }));
    }
    out.status("open a new shell to pick it up");
    Ok(())
}

fn uninstall(out: &Out, targets: &[Target]) -> CliResult {
    for target in targets {
        let mut changed = remove_script(&target.script)?;
        if let Some(rc) = &target.rc {
            changed |= remove_source_lines(rc)?;
        }
        out.emit(&target.record(if changed { "removed" } else { "absent" }));
    }
    Ok(())
}

/// The completion script as this binary's own argument parser defines it.
fn script(shell: Shell) -> Vec<u8> {
    let mut buffer = Vec::new();
    clap_complete::generate(
        shell.generator(),
        &mut Cli::command(),
        BIN,
        &mut buffer,
    );
    buffer
}

/// Which shells to act on. Naming shells aims the command at exactly those;
/// detection is what answers the question when nothing does.
fn targets(args: &CompletionsArgs) -> CliResult<Vec<Target>> {
    let dirs = Dirs::detect()?;
    let shells = if args.shell.is_empty() {
        let found = detected(&dirs, std::env::var("SHELL").ok().as_deref());
        if found.is_empty() {
            return Err(CliError::no_results(
                "no bash, zsh, or fish setup found in your home directory; \
                 name one with --shell",
            ));
        }
        found
    } else {
        args.shell.clone()
    };
    Ok(shells
        .into_iter()
        .map(|shell| target(shell, &dirs))
        .collect())
}

/// Where each shell looks. bash and zsh both read a per-user directory that
/// nothing puts on the search path by default, hence the rc line; fish scans
/// its own completions directory unprompted.
fn target(shell: Shell, dirs: &Dirs) -> Target {
    let (script, rc) = match shell {
        Shell::Bash => (
            dirs.data.join("bash-completion/completions").join(BIN),
            Some(bash_rc(&dirs.home)),
        ),
        Shell::Zsh => (
            dirs.data.join("zsh/site-functions").join(format!("_{BIN}")),
            Some(dirs.home.join(".zshrc")),
        ),
        Shell::Fish => (
            dirs.config
                .join("fish/completions")
                .join(format!("{BIN}.fish")),
            None,
        ),
    };
    Target { shell, script, rc }
}

/// A shell counts as present when it has left configuration behind, or when it
/// is the one the user is logged into — a fresh macOS zsh account has no
/// `.zshrc` at all until something writes one.
fn detected(dirs: &Dirs, login_shell: Option<&str>) -> Vec<Shell> {
    let running =
        |name: &str| login_shell.is_some_and(|path| path.ends_with(name));
    [
        (
            Shell::Bash,
            dirs.home.join(".bashrc").exists()
                || dirs.home.join(".bash_profile").exists()
                || running("bash"),
        ),
        (
            Shell::Zsh,
            dirs.home.join(".zshrc").exists() || running("zsh"),
        ),
        (
            Shell::Fish,
            dirs.config.join("fish").is_dir() || running("fish"),
        ),
    ]
    .into_iter()
    .filter_map(|(shell, present)| present.then_some(shell))
    .collect()
}

/// Interactive bash reads `.bashrc`, except on machines where only the login
/// file exists — which is the common macOS shape.
fn bash_rc(home: &Path) -> PathBuf {
    let profile = home.join(".bash_profile");
    let rc = home.join(".bashrc");
    if !rc.exists() && profile.exists() {
        profile
    } else {
        rc
    }
}

impl Dirs {
    fn detect() -> CliResult<Self> {
        let home = UserDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .ok_or_else(|| {
                CliError::usage("cannot find your home directory")
            })?;
        Ok(Self {
            data: xdg("XDG_DATA_HOME", &home, ".local/share"),
            config: xdg("XDG_CONFIG_HOME", &home, ".config"),
            home,
        })
    }
}

/// Shells follow the XDG layout on every platform, unlike this client's own
/// config, which follows each platform's convention (see `persist::paths`).
fn xdg(variable: &str, home: &Path, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .filter(|dir| !dir.is_empty())
        .map_or_else(|| home.join(fallback), PathBuf::from)
}

fn write_script(path: &Path, script: &[u8]) -> CliResult<bool> {
    if std::fs::read(path).is_ok_and(|found| found == script) {
        return Ok(false);
    }
    create_parent(path)?;
    std::fs::write(path, script).map_err(|e| unwritable(path, &e))?;
    Ok(true)
}

fn remove_script(path: &Path) -> CliResult<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(unwritable(path, &e)),
    }
}

fn add_source_line(rc: &Path, line: &str) -> CliResult<bool> {
    let existing = std::fs::read_to_string(rc).unwrap_or_default();
    if existing.lines().any(|found| found == line) {
        return Ok(false);
    }
    create_parent(rc)?;
    let separator = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rc)
        .map_err(|e| unwritable(rc, &e))?;
    writeln!(file, "{separator}{line}").map_err(|e| unwritable(rc, &e))?;
    Ok(true)
}

/// Take back only the marked lines. Somebody else's rc file is a bad thing to
/// rewrite from memory, so every other line is preserved verbatim.
fn remove_source_lines(rc: &Path) -> CliResult<bool> {
    let Ok(existing) = std::fs::read_to_string(rc) else {
        return Ok(false);
    };
    let kept: Vec<&str> = existing
        .lines()
        .filter(|line| !line.ends_with(MARKER))
        .collect();
    if kept.len() == existing.lines().count() {
        return Ok(false);
    }
    let mut rewritten = kept.join("\n");
    if !rewritten.is_empty() {
        rewritten.push('\n');
    }
    std::fs::write(rc, rewritten).map_err(|e| unwritable(rc, &e))?;
    Ok(true)
}

fn create_parent(path: &Path) -> CliResult {
    match path.parent() {
        Some(parent) => {
            std::fs::create_dir_all(parent).map_err(|e| unwritable(parent, &e))
        }
        None => Ok(()),
    }
}

fn unwritable(path: &Path, error: &std::io::Error) -> CliError {
    CliError::usage(format!("cannot write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(home: &Path) -> Dirs {
        Dirs {
            home: home.to_path_buf(),
            data: home.join(".local/share"),
            config: home.join(".config"),
        }
    }

    /// The rc line only works because sourcing the script registers the
    /// completion rather than just defining a function. zsh is the one that
    /// could plausibly go the other way: an autoload-style `#compdef` file does
    /// nothing on its own, so the install rests on the footer clap emits.
    #[test]
    fn a_sourced_script_registers_the_command() {
        let zsh = String::from_utf8(script(Shell::Zsh)).expect("utf-8");
        assert!(zsh.contains("compdef _soulseek-rs soulseek-rs"));
        let bash = String::from_utf8(script(Shell::Bash)).expect("utf-8");
        assert!(bash.contains("complete -F _soulseek-rs"));
    }

    #[test]
    fn each_shell_lands_where_it_already_looks() {
        let home = PathBuf::from("/home/me");
        let dirs = dirs(&home);
        for (shell, script, rc) in [
            (
                Shell::Bash,
                ".local/share/bash-completion/completions/soulseek-rs",
                Some(".bashrc"),
            ),
            (
                Shell::Zsh,
                ".local/share/zsh/site-functions/_soulseek-rs",
                Some(".zshrc"),
            ),
            // fish scans its own completions directory, so it needs no rc line.
            (
                Shell::Fish,
                ".config/fish/completions/soulseek-rs.fish",
                None,
            ),
        ] {
            let target = target(shell, &dirs);
            assert_eq!(target.script, home.join(script));
            assert_eq!(target.rc, rc.map(|file| home.join(file)));
        }
    }

    #[test]
    fn only_shells_this_machine_uses_become_targets() {
        let home = tempfile::tempdir().expect("tempdir");
        let dirs = dirs(home.path());
        assert!(detected(&dirs, None).is_empty());
        assert_eq!(detected(&dirs, Some("/bin/zsh")), [Shell::Zsh]);

        std::fs::write(home.path().join(".bashrc"), "").expect("bashrc");
        std::fs::create_dir_all(dirs.config.join("fish")).expect("fish");
        assert_eq!(detected(&dirs, None), [Shell::Bash, Shell::Fish]);
    }

    #[test]
    fn bash_falls_back_to_the_login_file_when_that_is_all_there_is() {
        let home = tempfile::tempdir().expect("tempdir");
        assert_eq!(bash_rc(home.path()), home.path().join(".bashrc"));

        std::fs::write(home.path().join(".bash_profile"), "").expect("write");
        assert_eq!(bash_rc(home.path()), home.path().join(".bash_profile"));

        std::fs::write(home.path().join(".bashrc"), "").expect("write");
        assert_eq!(bash_rc(home.path()), home.path().join(".bashrc"));
    }

    #[test]
    fn installing_twice_adds_one_line_and_uninstalling_takes_only_that() {
        let home = tempfile::tempdir().expect("tempdir");
        let rc = home.path().join(".zshrc");
        std::fs::write(&rc, "export EDITOR=vi").expect("rc");
        let target = target(Shell::Zsh, &dirs(home.path()));
        let line = target.source_line();

        assert!(add_source_line(&rc, &line).expect("add"));
        assert!(!add_source_line(&rc, &line).expect("add again"));
        let after = std::fs::read_to_string(&rc).expect("read");
        assert_eq!(after, format!("export EDITOR=vi\n{line}\n"));

        assert!(remove_source_lines(&rc).expect("remove"));
        assert!(!remove_source_lines(&rc).expect("remove again"));
        assert_eq!(
            std::fs::read_to_string(&rc).expect("read"),
            "export EDITOR=vi\n"
        );
    }

    #[test]
    fn a_script_is_written_once_and_removed_once() {
        let home = tempfile::tempdir().expect("tempdir");
        let path = target(Shell::Fish, &dirs(home.path())).script;
        let fish = script(Shell::Fish);

        assert!(write_script(&path, &fish).expect("write"));
        assert!(!write_script(&path, &fish).expect("write again"));
        assert_eq!(std::fs::read(&path).expect("read"), fish);

        assert!(remove_script(&path).expect("remove"));
        assert!(!remove_script(&path).expect("remove again"));
    }
}
