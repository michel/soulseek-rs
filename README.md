# Soulseek-rs 🦀

## ⚠️WARNING THIS IS UNDER DEVELOPMENT AND NOT READY FOR USE ⚠️

Souleek-rs is an experimental Soulseek library and client built in Rust. It's
under development and not yet ready for use. Soulseek is a closed-source P2P
file-sharing network from the 2000s. It's still used by music enthusiasts
around the world to share niche music.

## Project Goals

This project is a learning exercise to explore Rust. I've been using Soulseek
since the early 2000s, so it's close to my heart, and the Soulseek protocol is
a closed-source network protocol that provides a great opportunity to learn
about asynchronous and concurrent network programming and reverse engineering

It's not intended to be a production-ready client (yet).

Since it's a learning project, I have a self-imposed restriction not to use
external dependencies in the library. This means I can't use any external
crates that are not part of the Rust standard library. This is a good challenge
to learn how to build complex systems with only the standard library.

In the client crate, external dependencies are allowed for building a rich
experience. For me, this is a good balance between learning and practicality.

## Planned Features

- [x] Search for files
- [x] Download files
- [x] Configure credentials
- [x] TUI for searching and downloading files
- [ ] Configure download & upload directories
- [ ] Share files
- [ ] Browse user(s) files
- [ ] Chat in chatrooms
- [ ] Private messaging
- [ ] Headless mode daemon mode with remote control

## Project Structure

This project is organized as a Cargo workspace with two crates:

- **soulseek-rs-lib** - The core library implementing the Soulseek protocol
- **soulseek-rs** - A CLI client built on top of the library

This structure allows:

- Other developers to build custom Soulseek clients using `soulseek-rs-lib`
- Users to install the ready-made client via `cargo install soulseek-rs`
- Clean separation of concerns between protocol implementation and user interface

## Installation

### For Users

```bash
cargo install soulseek-rs
```

### For Developers

Clone and build from source:

```bash
git clone git@github.com:michel/soulseek-rs.git
cd soulseek-rs
cargo build --release
```

The binary will be available at `target/release/soulseek-rs`.

### For Library Users

To build your own Soulseek client, add to your `Cargo.toml`:

```toml
[dependencies]
soulseek-rs-lib = "0.1.0"
```

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
