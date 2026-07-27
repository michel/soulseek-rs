//! Standing searches: `wish`.
//!
//! A wishlist is what makes a client keep looking for something it did not find
//! today. The server treats it as a distinct message (code 103) and rate-limits
//! it to the interval it announces at login, so the schedule is the server's to
//! set — we only decide which queries to send and what to print.

use super::{Ctx, Session, settings::Store, transfer};
use crate::cli::{SortKey, WishRunArgs};
use crate::output::{CliError, CliResult, Out, WishRecord};
use crate::persist::config::FileConfig;
use crate::api::SessionApi;
use soulseek_rs::Client;
use std::time::Instant;

pub fn add(out: &Out, store: &Store, query: &str) -> CliResult {
    let mut updated = store.config.clone();
    if !updated.add_wish(query) {
        if query.trim().is_empty() {
            return Err(CliError::usage("a wish needs a query"));
        }
        out.status(&format!("already wishing for '{}'", query.trim()));
        return Ok(());
    }
    store.save(&updated)?;
    out.emit(&WishRecord {
        query: query.trim().to_string(),
    });
    Ok(())
}

pub fn remove(out: &Out, store: &Store, query: &str) -> CliResult {
    let mut updated = store.config.clone();
    if !updated.remove_wish(query) {
        return Err(CliError::no_results(format!(
            "'{}' is not on the wishlist",
            query.trim()
        )));
    }
    store.save(&updated)?;
    out.status(&format!("no longer wishing for '{}'", query.trim()));
    Ok(())
}

pub fn list(out: &Out, store: &Store) -> CliResult {
    let wishes = store.config.wishes();
    if wishes.is_empty() {
        return Err(CliError::no_results("the wishlist is empty"));
    }
    for query in wishes {
        out.emit(&WishRecord { query });
    }
    Ok(())
}

pub fn run(ctx: &Ctx, store: &Store, args: &WishRunArgs) -> CliResult {
    let wishes = selected(&store.config, args.only.as_deref())?;
    let session = Session::open(ctx)?;

    if !sweep(ctx, session.client.as_ref(), &wishes, &Sweep::from(args)) {
        return Err(CliError::no_results(
            "no wish matched anything on the network",
        ));
    }
    Ok(())
}

/// How to narrow and order what a sweep prints — the parts of `wish run`'s flags
/// that outlive the command, so `serve --wishlist` can sweep without inventing
/// a set of command-line arguments it never received.
struct Sweep {
    filter: transfer::Filter,
    sort: SortKey,
    limit: usize,
}

impl Default for Sweep {
    fn default() -> Self {
        Self {
            filter: transfer::Filter::default(),
            sort: SortKey::Best,
            limit: 0,
        }
    }
}

impl From<&WishRunArgs> for Sweep {
    fn from(args: &WishRunArgs) -> Self {
        Self {
            filter: transfer::Filter::from(&args.filter),
            sort: args.sort,
            limit: args.limit,
        }
    }
}

/// Re-run the wishlist for as long as `serve` stays online, on the interval the
/// server set. Called from the serve loop rather than owning it, so uploads keep
/// being reported between sweeps.
pub struct Sweeper {
    wishes: Vec<String>,
    next: Instant,
}

impl Sweeper {
    /// `None` when there is nothing to look for, so `serve` can say so once
    /// instead of waking up forever with an empty list.
    #[must_use]
    pub fn new(wishes: Vec<String>) -> Option<Self> {
        (!wishes.is_empty()).then(|| Self {
            wishes,
            // The first sweep is immediate: a wishlist that waits twelve
            // minutes before its first look is indistinguishable from broken.
            next: Instant::now(),
        })
    }

    /// Search every wish if the interval has elapsed. Does nothing otherwise,
    /// so the caller can poll it as often as it likes.
    ///
    /// this blocks the serve loop for one search window, so uploads go
    /// unreported and `--duration` overshoots by that much. Move the sweep to its
    /// own thread with a channel of records if that ever matters; what mattered
    /// was that it used to be one window *per wish*.
    pub fn tick(&mut self, ctx: &Ctx, client: &dyn SessionApi) {
        if Instant::now() < self.next {
            return;
        }
        sweep(ctx, client, &self.wishes, &Sweep::default());
        let interval = client.wishlist_interval();
        self.next = Instant::now() + interval;
        ctx.out
            .status(&format!("next wishlist sweep in {}s", interval.as_secs()));
    }
}

/// Search every wish and print what survived the filters. Returns whether
/// anything did.
///
/// All the searches go out before anything is waited for: peers answer whenever
/// they get round to it, so one shared window covers every wish, where waiting
/// per wish would cost a full window each.
fn sweep(
    ctx: &Ctx,
    client: &dyn SessionApi,
    wishes: &[String],
    how: &Sweep,
) -> bool {
    let mut sent = Vec::new();
    for wish in wishes {
        match client.start_wishlist_search(wish) {
            Ok(()) => sent.push(wish),
            Err(e) => ctx
                .out
                .warn(&format!("'{wish}' could not be searched: {e}")),
        }
    }
    if sent.is_empty() {
        return false;
    }

    ctx.out.status(&format!(
        "wishing for {} ({}s)…",
        sent.iter()
            .map(|w| format!("'{w}'"))
            .collect::<Vec<_>>()
            .join(", "),
        ctx.search_timeout.as_secs()
    ));
    Client::collect_for(ctx.search_timeout, None);

    let mut found_any = false;
    for wish in sent {
        let results = client.get_search_results(wish);
        let ranked =
            transfer::ranked(&results, &how.filter, how.sort, how.limit, wish);
        let Ok(found) = ranked else {
            ctx.out.status(&format!("nothing for '{wish}' this time"));
            continue;
        };
        found_any = true;
        for candidate in &found {
            ctx.out.emit(candidate);
        }
    }
    found_any
}

/// Which wishes a run covers. `--only` has to name a stored wish: letting a
/// typo through would quietly search for something the user never asked to
/// keep, and report it as a wishlist hit.
///
/// Matching goes through [`FileConfig::wish`], so a query that `wish add` would
/// call a duplicate is one `--only` can select.
fn selected(config: &FileConfig, only: Option<&str>) -> CliResult<Vec<String>> {
    let Some(only) = only else {
        let wishes = config.wishes();
        if wishes.is_empty() {
            return Err(CliError::no_results(
                "the wishlist is empty — add one with `wish add QUERY`",
            ));
        }
        return Ok(wishes);
    };

    config
        .wish(only)
        .map(|wish| vec![wish.clone()])
        .ok_or_else(|| {
            CliError::usage(format!(
                "'{}' is not on the wishlist; `wish list` shows what is",
                only.trim()
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Exit;

    fn stored() -> FileConfig {
        let mut config = FileConfig::default();
        config.add_wish("Aphex Twin");
        config.add_wish("Boards of Canada");
        config
    }

    #[test]
    fn without_only_every_stored_wish_runs() {
        assert_eq!(selected(&stored(), None).unwrap(), stored().wishes());
    }

    #[test]
    fn an_empty_wishlist_is_the_no_results_verdict() {
        let error =
            selected(&FileConfig::default(), None).expect_err("nothing to run");
        assert_eq!(error.exit, Exit::NoResults);
    }

    #[test]
    fn only_selects_one_stored_wish_ignoring_case_and_space() {
        assert_eq!(
            selected(&stored(), Some("  aphex twin ")).unwrap(),
            ["Aphex Twin"],
            "the stored spelling is what gets searched"
        );
    }

    #[test]
    fn only_naming_something_unstored_is_a_usage_error() {
        let error = selected(&stored(), Some("plaid"))
            .expect_err("a typo must not become an ad-hoc search");
        assert_eq!(error.exit, Exit::Usage);
        assert!(error.message.contains("wish list"), "{}", error.message);
    }

    #[test]
    fn a_sweeper_needs_something_to_look_for() {
        assert!(Sweeper::new(Vec::new()).is_none());
        assert!(Sweeper::new(stored().wishes()).is_some());
    }
}
