# Soulseek-rs 🦀

## ⚠️WARNING THIS IS UNDER DEVELOPMENT AND NOT READY FOR USE ⚠️

Souleek-rs is an experimental Soulseek client built in Rust. It's under
development and not yet ready for use. Soulseek is a closed-source P2P
file-sharing network from the 2000s. It's still used by music enthusiasts
around the world to share niche music.

## Project Goals

This project is a learning exercise to explore Rust. I've been using Soulseek
since the early 2000s, so it's close to my heart, and the Soulseek protocol is
a closed-source network protocol that provides a great opportunity to learn
about asynchronous and concurrent network programming and reverse engineering

It's not intended to be a production-ready client (yet).

Since it's a learning project, I have a self-imposed restriction not to use
external dependencies. This means I can't use any external crates that are not
part of the Rust standard library. This is a good challenge to learn how to
build complex systems with only the standard library.

## Planned Features

- [x] Search for files
- [x] Download files
- [ ] Configure credentials
- [ ] Configure download & upload directories
- [ ] Share files
- [ ] Browse user(s) files
- [ ] Chat in chatrooms
- [ ] Private messaging
- [ ] Headless mode daemon mode with remote control

## Project structure

Soulseek-rs is planned to be structured as a generic reusable library
(soulseek-rs-lib) with a CLI client (soulseek-rs).

The CLI client will have a simple command-line interface to interact with the
library. A rich TUI mode will be added in the future.

## Installation

To install Soulseek-rs, you'll need to have Rust installed on your system. Then,
you can clone the repository and build it with cargo:

```bash
git clone git@github.com:michel/soulseek-rs.git
cd soulseek-rs
cargo build --release
```

The binary will be available at `target/release/soulseek-rs`.

## Usage

```bash
./target/release/soulseek-rs "the weeknd Blinding Lights"
```

## Development

To run the project in development mode with debug output and trace output:

```bash
RUST_LOG=trace cargo run
```

## Development

To run the tests:

```bash
cargo test
```

To run the linter:

```bash
cargo clippy
```

To run the formatter:

```bash
cargo fmt
```

## License

This project is licensed under the MIT License — see the [LICENSE](./LICENSE)
file for details.
