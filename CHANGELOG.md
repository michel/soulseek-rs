# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [14.1.1](https://github.com/michel/soulseek-rs/compare/v14.1.0...v14.1.1) - 2026-08-17

### Fixed

- *(peer)* survive download token races on the listener path
- *(peer)* wait for the transfer token before the download lookup

### Other

- Merge pull request #44 from michel/issue-42-download-token-races

## [14.1.0](https://github.com/michel/soulseek-rs/compare/v14.0.1...v14.1.0) - 2026-08-14

### Added

- *(tui)* wrap long chat messages instead of clipping them
- *(tui)* navigate and confirm while filtering search results

### Other

- rustfmt in chat wrap test
- *(client)* dedupe peer registration and order eviction by id

## [14.0.1](https://github.com/michel/soulseek-rs/compare/v14.0.0...v14.0.1) - 2026-08-13

### Fixed

- *(client)* survive the review findings on the thread-flood fix
- *(client)* bound the threads a peer flood can create
- *(peer)* handle UploadDenied so refused downloads fail fast
- *(peer)* forward UploadFailed to the download store
- *(actor)* leave Disconnected state behind after every disconnect

### Other

- Merge branch 'develop': peer failure forwarding and thread-flood fixes

## [14.0.0](https://github.com/michel/soulseek-rs/compare/v13.0.0...v14.0.0) - 2026-08-10

### Fixed

- survive busy sessions without exhausting file descriptors
- *(lib)* cap what a search collects

### Other

- Merge branch 'develop': fd-exhaustion fixes
- *(daemon)* answer search polls with counts instead of copies

## [13.0.0](https://github.com/michel/soulseek-rs/compare/v12.0.0...v13.0.0) - 2026-07-29

### Added

- *(daemon)* make the download folder a live setting in both modes
- *(daemon)* make every window a view of one shared session
- *(daemon)* say what to do next on startup, and prove multiple clients agree
- *(config)* remember which daemon to use, and document the download box
- *(daemon)* route commands and the TUI through a running daemon
- *(daemon)* resident session, auth-gated control socket, and event fan-out
- *(daemon)* wire protocol with a type-safe method vocabulary and a generated OpenRPC contract

### Fixed

- *(daemon)* version the contract by its protocol, not by the crate
- *(daemon)* ask the OS for enough open files, and stop spinning when it says no
- *(daemon)* a thread the OS refuses must not take the listener with it
- *(shares)* let add and remove reach a running daemon
- *(daemon)* correct the published contract and harden the control socket

### Other

- *(daemon)* match a reply by its id instead of taking the next line
- cut what a whole-repo audit proved dead, padded, or hand-rolled
- *(skill)* choose the track yourself; narrow the results instead of skipping them
- *(skill)* make the cheap path the default and stop moving paths by hand
- *(skill)* stream picks into transfers instead of batching them
- *(skill)* raise the batch limits to what the client is actually built for
- *(skill)* add the search ladder, and one budget both batch routes obey
- *(skill)* teach the agent to fetch a list without getting the user banned
- *(skill)* make the daemon the default way an agent works
- *(daemon)* route the suite at its chokepoint and cover the protocol directly
- *(daemon)* drive the whole CLI suite against a daemon, and fix what it caught
- publish the daemon protocol and describe daemon mode in the README
- *(api)* put a SessionApi seam between the commands, the TUI, and the client

## [12.0.0](https://github.com/michel/soulseek-rs/compare/v11.0.0...v12.0.0) - 2026-07-26

### Added

- lift the client's concurrency ceiling from ~15 peers to 1024

### Fixed

- *(serve)* report a queued upload even when it is served between two polls

### Other

- raise transfer defaults and stop uploads sleeping on their slot
- add a code quality baseline
- *(lib)* move the client tests into their own file

## [11.0.0](https://github.com/michel/soulseek-rs/compare/v10.0.0...v11.0.0) - 2026-07-26

### Fixed

- [**breaking**] make concurrent invocations work instead of silently seeing nothing ([#22](https://github.com/michel/soulseek-rs/pull/22))

## [10.0.0](https://github.com/michel/soulseek-rs/compare/v9.1.0...v10.0.0) - 2026-07-26

### Added

- [**breaking**] wishlist, upload-queue privilege recognition, and search filters ([#20](https://github.com/michel/soulseek-rs/pull/20))

## [9.1.0](https://github.com/michel/soulseek-rs/compare/v9.0.0...v9.1.0) - 2026-07-26

### Added

- resume an interrupted download instead of refetching it ([#18](https://github.com/michel/soulseek-rs/pull/18))

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
