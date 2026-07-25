# Shipping an agent skill with the binary

## Problem

The one-shot command surface is built for agents — stdout is data, `--json`
emits NDJSON, the exit code is the verdict — but an agent has no way to learn
that. It discovers `--help`, which lists flags and says nothing about the JSON
record shapes, the meaning of exit 4, or when `get` beats `search` + `download`.

The conventional answer is an agent skill: a `SKILL.md` with YAML frontmatter,
placed in a directory the agent scans. The problem is delivery. Someone who ran
`cargo install soulseek-rs` has a binary and no repository, so there is no file
on their disk to copy or link. The skill has to travel inside the binary.

## Decisions

**One format.** `SKILL.md`, written verbatim. Claude Code and opencode read it
natively; `--dir` covers anything else. No per-agent converters — a Cursor
`.mdc` renderer and a `GEMINI.md` section-splicer would be six formats and six
upstream paths to chase, inside a Soulseek client.

**Embedded, not packaged.** `include_str!("../SKILL.md")`. The binary is the
only artifact `cargo install` produces, so it carries the payload.

**No `update` verb.** `install` writes the embedded copy over whatever is
there, which is what update means. Identical bytes report `unchanged`.

**No symlink option.** With the skill embedded there is no source file to point
a link at.

**No interactive prompt.** This project's rule is that every feature runs
headless. Detection answers "which agents" better than a question does: install
into every agent directory that already exists, and report each one as a record.
It also keeps `dialoguer` out of the dependency list.

## Surface

```
soulseek-rs skills install    [--dir PATH]...
soulseek-rs skills uninstall  [--dir PATH]...
soulseek-rs skills list       [--dir PATH]...
```

`skills` plural, matching the existing `shares` group. None of the three needs
credentials or a network, so `main::run` answers them before demanding a login,
alongside `config` and `shares list`.

Targets, as a function of the home directory:

| Agent      | Skills directory                  | Present when             |
| ---------- | --------------------------------- | ------------------------ |
| `claude`   | `~/.claude/skills`                | `~/.claude` exists       |
| `opencode` | `$XDG_CONFIG_HOME/opencode/skill` | `…/opencode` exists      |

An agent counts as installed when its *parent* directory exists — the agent
being on the machine, not the skills directory already having been created.

`--dir PATH` names a target explicitly, reported as `custom`, and *replaces*
detection rather than adding to it: a run that says where to go should not also
write into every agent it happens to find, and it keeps the tests off the
developer's real home directory. Committing the skill into a repository is
`--dir .claude/skills` — a dedicated `--project` flag was cut, because it was
that same write under a nicer label and it hardcoded one agent's layout next to
a table that already knows two.

Each target receives `<dir>/soulseek-rs/SKILL.md`.

## Records

```rust
pub struct SkillRecord {
    pub agent: String,
    pub path: String,
    pub action: String,
}
```

`action` is `installed` (written), `unchanged` (already identical), `outdated`
(present but differs — `list` only), `removed`, or `absent`. Text rendering is
status-first like `ShareRecord` and `PortmapRecord`:

```
installed  claude    /Users/you/.claude/skills/soulseek-rs/SKILL.md
unchanged  opencode  /Users/you/.config/opencode/skill/soulseek-rs/SKILL.md
```

Exit codes reuse the existing ladder: `0` did something, `4` no agent detected
and no `--dir` given, `2` a target that cannot be written, `1` unexpected IO.

`uninstall` removes `<dir>/soulseek-rs/` only when it holds a `SKILL.md`
declaring `name: soulseek-rs`, so a mistyped `--dir` cannot delete an unrelated
directory. Anything not there reports `absent` and exits 0.

## Contents of SKILL.md

The frontmatter `description` is what decides whether the skill ever fires, so
it names the tasks a user would ask for rather than the commands.

The body, in the order an agent needs it:

1. The contract — always pass `--json`, stdout is NDJSON, never parse stderr,
   branch on the exit code.
2. The exit table, with what to *do* about each code.
3. One line per command with the JSON keys it emits — the part `--help` cannot
   give you — including the two commands that print nothing.
4. Idioms: `get` over `search` + `download`; `search --json | jq | download
   --stdin`.
5. Bounds: `--follow` never returns, so an agent passes `--duration`.
6. Credentials: never a password in argv.

## Drift

A skill that describes a command that no longer exists is worse than no skill.
A unit test walks the clap command tree and asserts every subcommand name
appears in the embedded `SKILL.md`, plus that the file mentions `--json`.
Adding a subcommand fails the build until it is documented. Judgement stays
hand-written; coverage is mechanical.

## Verification

- Unit: target resolution against an injected home directory, so no test reads
  the developer's real one.
- Unit: the coverage test above.
- E2E, in the server-free half of `cli_e2e.rs`: `install --dir <tmp>` writes the
  file, a second run reports `unchanged`, `uninstall` removes it, and a `--dir`
  holding a foreign `SKILL.md` is left alone.

## Documentation

An `### Agent skills` section in the README, one line in the `EXAMPLES` block in
`cli.rs`, and `skills` documented inside `SKILL.md` itself — which the coverage
test enforces, so the skill cannot ship without describing its own installer.
