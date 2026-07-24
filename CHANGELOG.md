# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [6.0.0](https://github.com/michel/soulseek-rs/compare/v5.0.0...v6.0.0) - 2026-07-24

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
