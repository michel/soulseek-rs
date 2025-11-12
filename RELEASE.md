# Release Guide

This guide explains how to publish the `soulseek-rs-lib` and `soulseek-rs` crates to crates.io.

## Prerequisites

1. Create an account on [crates.io](https://crates.io/)
2. Generate an API token: https://crates.io/me
3. Login to cargo: `cargo login <your-token>`

## Publishing Process

### Step 1: Publish the Library First

The client depends on the library, so the library must be published first.

```bash
cd soulseek-rs-lib
cargo publish --dry-run  # Test the publish
cargo publish            # Actually publish
```

### Step 2: Wait for crates.io to Index

After publishing the library, wait 1-2 minutes for crates.io to index it before publishing the client.

### Step 3: Update Client Dependency (for releases)

When publishing to crates.io, the client's `Cargo.toml` should reference the published library version, not the local path.

Edit `soulseek-rs/Cargo.toml`:

```toml
[dependencies]
# For local development:
soulseek-rs-lib = { version = "0.1.0", path = "../soulseek-rs-lib" }

# For publishing (comment out path):
# soulseek-rs-lib = "0.1.0"
```

Or use both with optional features:

```toml
[dependencies]
soulseek-rs-lib = "0.1.0"

[dev-dependencies]
soulseek-rs-lib = { version = "0.1.0", path = "../soulseek-rs-lib" }
```

### Step 4: Publish the Client

```bash
cd ../soulseek-rs
cargo publish --dry-run  # Test the publish
cargo publish            # Actually publish
```

## Version Management

When bumping versions:

1. Update the version in root `Cargo.toml` under `[workspace.package]`
2. Both crates will inherit this version automatically
3. Publish library first, then client

## Automated Releases (Optional)

You can automate this with GitHub Actions. Create `.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

jobs:
  publish:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Publish library
        run: |
          cd soulseek-rs-lib
          cargo publish --token ${{ secrets.CARGO_TOKEN }}

      - name: Wait for crates.io
        run: sleep 60

      - name: Publish client
        run: |
          cd soulseek-rs
          cargo publish --token ${{ secrets.CARGO_TOKEN }}
```

## Testing Installation

After publishing, test the installation:

```bash
cargo install soulseek-rs
soulseek-rs --help  # Should work
```

## For Developers Building Custom Clients

Developers can use the library by adding to their `Cargo.toml`:

```toml
[dependencies]
soulseek-rs-lib = "0.1.0"
```

They'll get access to all the types and functionality exported in `lib.rs`.
