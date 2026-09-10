//! Answering a search against our shares, and what the client keeps of
//! the answers it gets back.

use super::*;

#[test]
fn build_search_response_matches_shares_and_echoes_token() {
    let dir = std::env::temp_dir()
        .join(format!("soulseek-searchresp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe_xyzzy.bin"), b"data").unwrap();
    let shares = Shares::scan(&dir).unwrap();

    let response =
        build_search_response(&shares, "me", 99, "xyzzy", true, 0, 0, &[])
            .expect("a matching share yields a response");
    let mut decoded =
        crate::message::Message::new_with_data(response.get_buffer());
    decoded.set_pointer(8);
    let result = SearchResult::new_from_message(&mut decoded).unwrap();
    assert_eq!(result.username, "me");
    assert_eq!(result.token, 99);
    assert!(result.files.iter().any(|f| f.name.contains("probe_xyzzy")));

    assert!(
        build_search_response(&shares, "me", 1, "nomatch", true, 0, 0, &[])
            .is_none()
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn search_file_counts_cover_every_search_without_the_results() {
    // The daemon polls these counts many times a second; they must come from
    // a walk of the cache, never a copy of it.
    let client = Client::new("u", "p");
    {
        let mut context = client.context.write().unwrap();
        context.searches.insert(
            "aphex twin".to_string(),
            Search {
                token: 1,
                results: vec![peer_files(2), peer_files(1)],
            },
        );
        context.searches.insert(
            "nothing yet".to_string(),
            Search {
                token: 2,
                results: Vec::new(),
            },
        );
    }

    let mut counts = client.search_file_counts();
    counts.sort();
    assert_eq!(
        counts,
        [
            ("aphex twin".to_string(), 3),
            ("nothing yet".to_string(), 0)
        ]
    );
}

#[test]
fn a_search_stops_collecting_once_it_has_enough_responses() {
    // A popular query on the live network draws answers for minutes —
    // nearly a million files and gigabytes of memory for one search.
    // Surplus responders are dropped, not archived.
    let mut search = Search {
        token: 1,
        results: Vec::new(),
    };
    for _ in 0..(crate::types::MAX_SEARCH_RESPONSES + 50) {
        search.accept(peer_files(1));
    }
    assert_eq!(search.results.len(), crate::types::MAX_SEARCH_RESPONSES);
}

#[test]
fn a_flood_of_files_fills_a_search_before_the_response_cap() {
    // A handful of whales with huge matching collections must not add up
    // to an unbounded set just because the responses are few.
    let mut search = Search {
        token: 1,
        results: Vec::new(),
    };
    for _ in 0..10 {
        search.accept(peer_files(crate::types::MAX_SEARCH_FILES / 2));
    }
    assert_eq!(
        search.results.len(),
        2,
        "two half-cap responses fill the search; the rest are dropped"
    );
}

#[test]
fn a_file_the_server_excludes_is_left_out_of_a_reply() {
    // The server's excluded phrases (code 160) police what travels the search
    // network: a matching file whose path carries one must not be offered,
    // and a reply with nothing left is not sent at all.
    let dir = std::env::temp_dir()
        .join(format!("soulseek-excluded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Spam_Xyzzy.bin"), b"data").unwrap();
    let shares = Shares::scan(&dir).unwrap();

    assert!(
        build_search_response(&shares, "me", 1, "xyzzy", true, 0, 0, &[])
            .is_some(),
        "with no exclusions the file is offered"
    );
    assert!(
        build_search_response(
            &shares,
            "me",
            1,
            "xyzzy",
            true,
            0,
            0,
            &["spam".to_string()],
        )
        .is_none(),
        "an excluded phrase in the path, matched case-insensitively, \
         withholds the file — and with no files there is no reply"
    );
    assert!(
        build_search_response(
            &shares,
            "me",
            1,
            "xyzzy",
            true,
            0,
            0,
            &["unrelated".to_string()],
        )
        .is_some(),
        "an exclusion the path does not carry changes nothing"
    );

    // The phrases are lowercased where they arrive, so a server that sends
    // one capitalised still matches a path.
    let mut ctx = ClientContext::new();
    ctx.set_excluded_search_phrases(vec!["SPAM".to_string()]);
    assert!(
        build_search_response(
            &shares,
            "me",
            1,
            "xyzzy",
            true,
            0,
            0,
            &ctx.excluded_search_phrases(),
        )
        .is_none(),
        "a capitalised phrase from the server still withholds the file"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
