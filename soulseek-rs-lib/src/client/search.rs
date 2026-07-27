use super::{
    Arc, AtomicBool, Client, DEFAULT_WISHLIST_INTERVAL, Duration, HashMap,
    Instant, Ordering, Result, RwLockExt, Search, SearchResult, ServerMessage,
    SoulseekRs, info, md5, sleep,
};

impl Client {
    pub fn search(
        &self,
        query: &str,
        timeout: Duration,
    ) -> Result<Vec<SearchResult>> {
        self.search_with_cancel(query, timeout, None)
    }

    /// Send `query` as a wishlist search (server code 103) and return at once.
    ///
    /// Results accumulate under `query`, so the caller reads them back with
    /// [`Client::get_search_results`] after waiting however long it wants to.
    /// Starting several wishes and then waiting once is the whole point: waiting
    /// per wish would cost one full search window each.
    ///
    /// The server rate-limits these to the interval it announced — see
    /// [`Client::wishlist_interval`] — so a wish re-sent sooner than that comes
    /// back empty.
    ///
    /// # Errors
    /// [`SoulseekRs::NotConnected`] when there is no server connection.
    pub fn start_wishlist_search(&self, query: &str) -> Result<()> {
        self.send_search(query, true)
    }

    /// How long to wait between wishlist searches: what the server announced in
    /// code 104, or [`DEFAULT_WISHLIST_INTERVAL`] until it has.
    #[must_use]
    pub fn wishlist_interval(&self) -> Duration {
        self.context
            .read_safe()
            .ok()
            .and_then(|ctx| ctx.wishlist_interval)
            .map_or(DEFAULT_WISHLIST_INTERVAL, |seconds| {
                Duration::from_secs(u64::from(seconds))
            })
    }

    pub fn search_with_cancel(
        &self,
        query: &str,
        timeout: Duration,
        cancel_flag: Option<Arc<AtomicBool>>,
    ) -> Result<Vec<SearchResult>> {
        self.send_search(query, false)?;
        Self::collect_for(timeout, cancel_flag);
        Ok(self.get_search_results(query))
    }

    /// Register `query` and put its search on the wire. Returns as soon as the
    /// message is queued; nothing has answered yet.
    fn send_search(&self, query: &str, wishlist: bool) -> Result<()> {
        info!("Searching for {}", query);

        let Some(handle) = &self.server_handle else {
            return Err(SoulseekRs::NotConnected);
        };
        let hash = md5::md5(query);
        let token = u32::from_str_radix(&hash[0..5], 16)?;

        self.context.write_safe()?.searches.insert(
            query.to_string(),
            Search {
                token,
                results: Vec::new(),
            },
        );

        let query = query.to_string();
        let _ = handle.send(if wishlist {
            ServerMessage::WishlistSearch { token, query }
        } else {
            ServerMessage::FileSearch { token, query }
        });
        Ok(())
    }

    /// Let responses accumulate for `timeout`, or until cancelled.
    ///
    /// Peers answer a search over their own connections whenever they get round
    /// to it, so there is nothing to await — the window is the whole protocol.
    /// Late responders are often the better sources, which is why this runs the
    /// window out rather than returning on the first hit.
    pub fn collect_for(
        timeout: Duration,
        cancel_flag: Option<Arc<AtomicBool>>,
    ) {
        let start = Instant::now();
        while start.elapsed() < timeout {
            sleep(Duration::from_millis(100));
            if let Some(flag) = &cancel_flag
                && flag.load(Ordering::Relaxed)
            {
                info!("Search cancelled by user");
                return;
            }
        }
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

    /// Drop a search and everything it collected.
    ///
    /// Returns whether there was one to drop. A client that stays up for days
    /// would otherwise hold every result set it has ever seen, and a caller
    /// that has dismissed a search has no other way to say so.
    #[must_use]
    pub fn forget_search(&self, search_key: &str) -> bool {
        self.context
            .write_safe()
            .is_ok_and(|mut ctx| ctx.searches.remove(search_key).is_some())
    }

    #[must_use]
    pub fn get_all_searches(&self) -> HashMap<String, Search> {
        self.context
            .read_safe()
            .map(|ctx| ctx.searches.clone())
            .unwrap_or_default()
    }
}
