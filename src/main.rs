fn main() {
    use soulseek_download::{Client, PeerAddress};
    use std::time::Duration;

    let mut client = Client::new(
        PeerAddress::new(String::from("server.slsknet.org"), 2242),
        String::from("insane_in_the_brain3"),
        String::from("13375137"),
    );

    client.connect();
    match client.login() {
        Ok(_) => {
            let results = client.search("Fantazia", Duration::from_secs(10));
            println!("Search results: {} - {:?} ", results.len(), results);
            if !results.is_empty() && !results[0].files.is_empty() {
                let file = results[0].files[0].clone();
                let download_result = client.download(file.name, file.username);
                println!("Download result: {:?}", download_result);
            }
        }
        Err(e) => {
            eprintln!("Failed to login: {}", e);
        }
    }
}
