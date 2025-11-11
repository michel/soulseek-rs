# soulseek-rs-lib

A Rust library for implementing the Soulseek peer-to-peer protocol.

## About

This library provides the core functionality for interacting with the Soulseek
network. It can be used to build custom Soulseek clients or bots.

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
soulseek-rs-lib = "0.1.0"
```

## Example

### Simple Usage

```rust
use soulseek_rs::Client;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create and connect to Soulseek server
    let mut client = Client::new("username", "password");

    client.connect();
    client.login()?;

    // Search for files
    let results = client.search("Alex Kassian lifestream", Duration::from_secs(10))?;

    // Download first available file
    if let Some(result) = results.iter().find(|r| !r.files.is_empty()) {
        let file = &result.files[0];
        client.download(
            file.name.clone(),
            file.username.clone(),
            file.size,
            "~/Downloads".to_string(),
        )?;
        println!("Downloaded: {}", file.name);
    }

    Ok(())
}
```

### Advanced Configuration

```rust
use soulseek_rs::{Client, ClientSettings, PeerAddress};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client with custom settings
    let settings = ClientSettings {
        server_address: PeerAddress::new("server.slsknet.org".to_string(), 2242),
        enable_listen: true,
        listen_port: 3000,
        ..ClientSettings::new("username", "password")
    };

    let mut client = Client::with_settings(settings);
    client.connect();
    client.login()?;

    Ok(())
}
```
