# soulseek-rs-lib

A Rust library for implementing the Soulseek peer-to-peer protocol.

Website: <https://re-invention.nl/soulseek-rs/>

## About

This library provides the core functionality for interacting with the Soulseek
network. It can be used to build custom Soulseek clients or bots.

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
soulseek-rs-lib = "8"
```

## Example

### Simple Usage

```rust,no_run
use soulseek_rs::Client;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create and connect to Soulseek server
    let mut client = Client::new("username", "password");

    client.connect()?;
    client.login()?;

    // Search for files
    let results = client.search("Alex Kassian lifestream", Duration::from_secs(10))?;

    // Download first available file
    if let Some(result) = results.iter().find(|r| !r.files.is_empty()) {
        let file = &result.files[0];
        let (_download, status) = client.download(
            file.name.clone(),
            file.username.clone(),
            file.size,
            "~/Downloads".to_string(),
        )?;

        // `download` returns as soon as the request is queued: the transfer is
        // finished when the channel says so, not when this call returns.
        for update in status {
            println!("{}: {update:?}", file.name);
        }
    }

    Ok(())
}
```

### Advanced Configuration

```rust,no_run
use soulseek_rs::{Client, ClientSettings, PeerAddress};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client with custom settings
    let settings = ClientSettings {
        server_address: PeerAddress::new("server.slsknet.org".to_string(), 2416),
        enable_listen: true,
        listen_port: 3000,
        ..ClientSettings::new("username", "password")
    };

    let mut client = Client::with_settings(settings);
    client.connect()?;
    client.login()?;

    Ok(())
}
```

### Asking about a peer

The server-backed lookups are request/poll pairs: you ask, then read the
answer once it lands. `request_user_info` clears any previous answer first, so
the loop below observes this request rather than the last one.

```rust,no_run
use soulseek_rs::Client;
use std::thread::sleep;
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = Client::new("username", "password");
    client.connect()?;
    client.login()?;

    client.request_user_info("someuser")?;

    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(info) = client.user_info("someuser") {
            // Presence and stats are separate replies, so each is an Option:
            // a reply that never arrived is None, not "offline" or "shares nothing".
            println!("{:?} {:?}", info.presence, info.stats);
            break;
        }
        sleep(Duration::from_millis(100));
    }

    // Room membership is kept current from join and leave events.
    println!("{:?}", client.room_members("lobby"));

    Ok(())
}
```

`ServerMessage` and `ClientOperation` are `#[non_exhaustive]`: every protocol
message the client learns to handle adds a variant, so match them with a
wildcard arm.
