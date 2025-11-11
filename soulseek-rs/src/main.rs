use soulseek_rs::Client;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create and connect to Soulseek server
    let mut client = Client::new("username", "password");

    client.connect();
    client.login()?;

    // Search for files
    let results =
        client.search("Alex Kassian lifestream", Duration::from_secs(10))?;

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
