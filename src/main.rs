fn main() {
    use soulseek_rs::{Client, PeerAddress};
    use std::time::Duration;

    let mut client = Client::new(
        PeerAddress::new(String::from("server.slsknet.org"), 2242),
        String::from("xxxxxx"),
        String::from("xxxxxx"),
        false,
        None,
    );

    client.connect();
    match client.login() {
        Ok(_) => {
            match client.search("michel test file", Duration::from_secs(4)) {
                Ok(results) => {
                    if let Some(result) =
                        results.iter().find(|r| !r.files.is_empty())
                    {
                        let file = result.files[0].clone();
                        match client.download(
                            file.name,
                            file.username,
                            file.size,
                            "~/Downloads".to_string(),
                        ) {
                            Ok(download_result) => {
                                println!(
                                    "Download result: {download_result:?}"
                                );
                            }
                            Err(e) => {
                                eprintln!("Failed to download: {e}");
                            }
                        }
                    } else {
                        eprint!("No results");
                    }
                }
                Err(e) => {
                    eprintln!("Failed to search: {e}");
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to login: {e}");
        }
    }
}
