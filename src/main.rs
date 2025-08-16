fn main() {
    use soulseek_rs::{Client, PeerAddress};
    use std::time::Duration;

    let mut client = Client::new(
        PeerAddress::new(String::from("server.slsknet.org"), 2242),
        String::from("insane_in_the_brain3"),
        String::from("13375137"),
    );

    client.connect();
    match client.login() {
        Ok(_) => match client
            .search("Super flu Believe", Duration::from_secs(20))
        {
            Ok(results) => {
                if let Some(result) = results.iter().find(|r| {
                    !r.files.is_empty()
                    // && r.username != "Mus4Mus022"
                    // && r.username != "Slackman2505"
                    // && r.username == "MisterDanielson"
                }) {
                    let file = result.files[0].clone();
                    match client.download(file.name, file.username, file.size) {
                        Ok(download_result) => {
                            println!("Download result: {:?}", download_result);
                        }
                        Err(e) => {
                            eprintln!("Failed to download: {}", e);
                        }
                    }
                } else {
                    eprint!("No results")
                }
            }
            Err(e) => {
                eprintln!("Failed to search: {}", e);
            }
        },
        Err(e) => {
            eprintln!("Failed to login: {}", e);
        }
    }
}
