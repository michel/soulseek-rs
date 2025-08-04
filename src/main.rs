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
            .search("Super flu Believe Believe", Duration::from_secs(10))
        {
            Ok(results) => {
                if let Some(file) = results
                    .iter()
                    .find(|res| res.username == "betsos76")
                    .map(|res| &res.files[0])
                {
                    match client.download(
                        file.name.to_string(),
                        file.username.to_string(),
                        file.size,
                    ) {
                        Ok(download_result) => {
                            println!("Download result: {:?}", download_result);
                        }
                        Err(e) => {
                            eprintln!("Failed to download: {}", e);
                        }
                    }
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
