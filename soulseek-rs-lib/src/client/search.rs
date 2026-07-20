use super::{
    Arc, AtomicBool, Client, Duration, HashMap, Instant, Ordering, Result,
    RwLockExt, Search, SearchResult, ServerMessage, SoulseekRs, info, md5,
    sleep,
};

impl Client {
    pub fn search(
        &self,
        query: &str,
        timeout: Duration,
    ) -> Result<Vec<SearchResult>> {
        self.search_with_cancel(query, timeout, None)
    }

    pub fn search_with_cancel(
        &self,
        query: &str,
        timeout: Duration,
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<Vec<SearchResult>> {
        info!("Searching for {}", query);

        if let Some(handle) = &self.server_handle {
            let hash = md5::md5(query);
            let token = u32::from_str_radix(&hash[0..5], 16)?;

            self.context.write_safe()?.searches.insert(
                query.to_string(),
                Search {
                    token,
                    results: Vec::new(),
                },
            );

            let _ = handle.send(ServerMessage::FileSearch {
                token,
                query: query.to_string(),
            });
        } else {
            return Err(SoulseekRs::NotConnected);
        }

        let start = Instant::now();
        loop {
            sleep(Duration::from_millis(100));

            // Check if cancelled
            if let Some(ref flag) = cancel_flag
                && flag.load(Ordering::Relaxed)
            {
                info!("Search cancelled by user");
                break;
            }

            // Check if timeout reached
            if start.elapsed() >= timeout {
                break;
            }
        }

        Ok(self.get_search_results(query))
    }

    #[must_use]
    pub fn get_search_results_count(&self, search_key: &str) -> usize {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| {
                ctx.searches.get(search_key).map(|s| s.results.len())
            })
            .unwrap_or(0)
    }

    #[must_use]
    pub fn get_search_results(&self, search_key: &str) -> Vec<SearchResult> {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| {
                ctx.searches.get(search_key).map(|s| s.results.clone())
            })
            .unwrap_or_default()
    }

    /// Non-blocking variant that returns None if the lock is unavailable
    #[must_use]
    pub fn try_get_search_results(
        &self,
        search_key: &str,
    ) -> Option<Vec<SearchResult>> {
        self.context.try_read().ok().and_then(|ctx| {
            ctx.searches.get(search_key).map(|s| s.results.clone())
        })
    }

    #[must_use]
    pub fn get_all_searches(&self) -> HashMap<String, Search> {
        self.context
            .read_safe()
            .map(|ctx| ctx.searches.clone())
            .unwrap_or_default()
    }
}
