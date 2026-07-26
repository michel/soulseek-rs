# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [9.0.0](https://github.com/michel/soulseek-rs/compare/v8.0.0...v9.0.0) - 2026-07-26

### Added

- install tab completion for bash, zsh, and fish ([#16](https://github.com/michel/soulseek-rs/pull/16))

## [8.0.0](https://github.com/michel/soulseek-rs/compare/v7.1.0...v8.0.0) - 2026-07-26

### Added

- log in with soulseek-rs's own client major version (176) ([#13](https://github.com/michel/soulseek-rs/pull/13))

## [7.1.0](https://github.com/michel/soulseek-rs/compare/v7.0.0...v7.1.0) - 2026-07-25

### Added

- *(tui)* give the panes the colour the demo promised

### Fixed

- *(lib)* give the login verdict the wait the caller already allows

## [7.0.0](https://github.com/michel/soulseek-rs/compare/v6.0.0...v7.0.0) - 2026-07-25

### Added

- *(cli)* ship the agent skill inside the binary
- *(cli)* expose sharing, peer lookup and settings as one-shot commands
- *(cli)* [**breaking**] replace the one-shot commands with a scriptable surface
- *(ui)* theme the TUI with the soulseek-rs design system

### Fixed

- *(peer)* serve full browse listings + harden the listener; TUI transfer/UX work
- *(persist)* use XDG paths on macOS
- *(peer)* log disconnect cause; bind e2e browse listener on all interfaces

### Other

- link the website from the crates and the READMEs
- clear the quality-scan findings
- document the skills command and the scriptable surface
- pin soulseek-rs-lib install snippets to 6

## [6.0.0](https://github.com/michel/soulseek-rs/compare/v5.0.0...v6.0.0) - 2026-07-24

### Breaking changes

Library only. If you install the `soulseek-rs` binary there is nothing to do —
these affect crates depending on `soulseek-rs-lib` 5.x. Sharing moved from a
single optional directory to a list, which changed three public items.

- `ClientSettings::shared_directory: Option<String>` is now
  `shared_directories: Vec<String>`. `None` becomes `vec![]`, `Some(dir)`
  becomes `vec![dir]`.
- `Client::shared_directory() -> Option<&str>` is replaced by
  `Client::shared_directories() -> Vec<String>`. The new
  `Client::set_shared_directories(Vec<String>)` updates the set at runtime.
- `peer::upload_peer::serve_file` takes two more parameters,
  `bytes_sent: &AtomicU64` and `cancel: &AtomicBool`, which drive upload
  progress and cancellation. Pass freshly constructed values to keep the old
  behaviour.

### Added

- settings popup and uploads in the transfers pane
- update shared directories at runtime
- multiple shared directories
- conventional default download and share folders
- set aside unusable state files as .bak instead of overwriting
- persist and restore TUI state across restarts
- versioned JSON state stores for downloads, searches, rooms
- in-TUI login/registration screen
- password resolution via OS keychain and password_cmd
- config.toml settings layer (CLI > env > file > defaults)
- track uploads with progress and cancellation

### Fixed

- drop libdbus dependency from keyring Linux backend
- clippy cleanups in upload tracking and share plumbing

### Other

- persistence e2e and registration-relogin e2e; lint clean
