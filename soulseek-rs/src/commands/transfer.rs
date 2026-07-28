//! Finding files and fetching them: `search`, `download`, and `get`.

use super::{Ctx, Session, social};
use crate::api::SessionApi;
use crate::cli::{
    DownloadArgs, FilterArgs, GetArgs, Pick, SearchArgs, SortKey,
};
use crate::output::{
    CliError, CliResult, DownloadRecord, Exit, Out, SearchRecord,
};
use soulseek_rs::utils::path::expand_tilde;
use soulseek_rs::{DownloadStatus, SearchResult};
use std::collections::{HashMap, VecDeque};
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Soulseek file attribute codes the network agrees on.
const ATTR_BITRATE: u32 = 0;
const ATTR_DURATION: u32 = 1;

/// What to keep out of the raw result set. Every field is a further
/// restriction: a candidate has to satisfy all of them.
///
/// `exclude` and `extensions` are stored already lowercased and without leading
/// dots — see [`Filter::from`] — so matching a result costs no allocation. A
/// search returns thousands of files and every one of them is tested.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub min_bitrate: Option<u32>,
    pub free_slots_only: bool,
    /// Terms that must not appear anywhere in the remote path, lowercased.
    pub exclude: Vec<String>,
    /// File extensions to keep, lowercased and dotless.
    pub extensions: Vec<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
}

impl Filter {
    fn keeps(&self, candidate: &SearchRecord) -> bool {
        if self.free_slots_only && !candidate.free_slot {
            return false;
        }
        if self.min_size.is_some_and(|min| candidate.size < min) {
            return false;
        }
        if self.max_size.is_some_and(|max| candidate.size > max) {
            return false;
        }
        // An unknown bitrate cannot satisfy a bitrate floor.
        if self
            .min_bitrate
            .is_some_and(|min| candidate.bitrate.is_none_or(|have| have < min))
        {
            return false;
        }
        if self.exclude.is_empty() && self.extensions.is_empty() {
            return true;
        }
        let path = candidate.path.to_lowercase();
        !self.exclude.iter().any(|term| path.contains(term.as_str()))
            && (self.extensions.is_empty() || self.matches_extension(&path))
    }

    fn matches_extension(&self, lowercase_path: &str) -> bool {
        let name = lowercase_path
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default();
        let Some((_, actual)) = name.rsplit_once('.') else {
            return false;
        };
        self.extensions.iter().any(|wanted| wanted == actual)
    }
}

/// Lowercase and strip a leading dot, so the comparison in [`Filter::keeps`] is
/// a plain equality however the flag was spelled.
fn normalized(terms: &[String]) -> Vec<String> {
    terms
        .iter()
        .map(|term| term.trim_start_matches('.').to_lowercase())
        .collect()
}

/// Flatten per-peer results into individual files, dropping what `filter`
/// rejects.
#[must_use]
pub fn candidates(
    results: &[SearchResult],
    filter: &Filter,
) -> Vec<SearchRecord> {
    let mut out = Vec::new();
    for result in results {
        for file in &result.files {
            let candidate = SearchRecord {
                user: result.username.clone(),
                path: file.name.clone(),
                size: file.size,
                bitrate: file.attribs.get(&ATTR_BITRATE).copied(),
                duration: file.attribs.get(&ATTR_DURATION).copied(),
                free_slot: result.slots > 0,
                slots: result.slots,
                speed: result.speed,
            };
            if filter.keeps(&candidate) {
                out.push(candidate);
            }
        }
    }
    out
}

/// Order results. Every mode ends with the same user/path tiebreak so the
/// output of two identical runs is identical.
pub fn rank(items: &mut [SearchRecord], sort: SortKey) {
    items.sort_by(|a, b| {
        let primary = match sort {
            SortKey::Best => b
                .free_slot
                .cmp(&a.free_slot)
                .then_with(|| {
                    b.bitrate.unwrap_or(0).cmp(&a.bitrate.unwrap_or(0))
                })
                .then_with(|| b.speed.cmp(&a.speed))
                .then_with(|| b.size.cmp(&a.size)),
            SortKey::Size => b.size.cmp(&a.size),
            SortKey::Bitrate => {
                b.bitrate.unwrap_or(0).cmp(&a.bitrate.unwrap_or(0))
            }
            SortKey::Speed => b.speed.cmp(&a.speed),
        };
        primary
            .then_with(|| a.user.cmp(&b.user))
            .then_with(|| a.path.cmp(&b.path))
    });
}

/// Where a remote file lands locally — the same rule the library applies, so
/// the path we report is the path that gets written.
#[must_use]
pub fn local_path(download_dir: &str, remote_path: &str) -> PathBuf {
    let name = remote_path
        .split(['/', '\\'])
        .next_back()
        .unwrap_or(remote_path);
    expand_tilde(download_dir).join(name)
}

/// One file to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub user: String,
    pub path: String,
    pub size: u64,
}

impl From<&FilterArgs> for Filter {
    fn from(args: &FilterArgs) -> Self {
        Self {
            min_bitrate: args.min_bitrate,
            free_slots_only: args.free_slots,
            exclude: normalized(&args.exclude),
            extensions: normalized(&args.extension),
            min_size: args.min_size,
            max_size: args.max_size,
        }
    }
}

impl From<&SearchRecord> for Request {
    fn from(candidate: &SearchRecord) -> Self {
        Self {
            user: candidate.user.clone(),
            path: candidate.path.clone(),
            size: candidate.size,
        }
    }
}

/// Filter, rank, and cap a raw result set, failing when nothing survives.
pub(super) fn ranked(
    results: &[SearchResult],
    filter: &Filter,
    sort: SortKey,
    limit: usize,
    query: &str,
) -> CliResult<Vec<SearchRecord>> {
    let mut found = candidates(results, filter);
    rank(&mut found, sort);
    if limit > 0 {
        found.truncate(limit);
    }
    if found.is_empty() {
        return Err(CliError::no_results(format!("no results for '{query}'")));
    }
    Ok(found)
}

pub fn search(ctx: &Ctx, args: &SearchArgs) -> CliResult {
    let session = Session::open(ctx)?;
    let results = collect(ctx, &session.client, &args.query, false)?;
    let found = ranked(
        &results,
        &Filter::from(&args.filter),
        args.sort,
        args.limit,
        &args.query,
    )?;

    for candidate in &found {
        ctx.out.emit(candidate);
    }
    Ok(())
}

pub fn download(ctx: &Ctx, args: &DownloadArgs) -> CliResult {
    let requests = if args.stdin {
        let stdin = std::io::stdin();
        read_requests(&mut stdin.lock(), &ctx.out)?
    } else {
        let (Some(user), Some(path)) = (args.user.clone(), args.path.clone())
        else {
            return Err(CliError::usage(
                "download needs USER and PATH, or --stdin",
            ));
        };
        vec![Request {
            user,
            path,
            size: args.size.unwrap_or(0),
        }]
    };

    if requests.is_empty() {
        return Err(CliError::no_results("no files to download"));
    }

    ensure_download_dir(ctx)?;
    let session = Session::open(ctx)?;
    let requests = resolve_sizes(ctx, session.client.as_ref(), requests)?;
    download_all(
        ctx,
        &session.client,
        requests,
        Duration::from_secs(args.timeout),
    )
}

pub fn get(ctx: &Ctx, args: &GetArgs) -> CliResult {
    ensure_download_dir(ctx)?;
    let session = Session::open(ctx)?;
    let results =
        collect(ctx, &session.client, &args.query, args.pick == Pick::First)?;

    let limit = match args.pick {
        Pick::Best | Pick::First => 1,
        Pick::All => args.limit,
    };
    let found = ranked(
        &results,
        &Filter::from(&args.filter),
        SortKey::Best,
        limit,
        &args.query,
    )?;

    download_all(
        ctx,
        &session.client,
        found.iter().map(Request::from).collect(),
        Duration::from_secs(args.timeout),
    )
}

/// Run a search and hand back the raw per-peer results.
///
/// `stop_early` returns as soon as anything arrives, which is what
/// `get --pick first` wants; otherwise the search runs its full span because
/// late responders are often the better sources.
fn collect(
    ctx: &Ctx,
    client: &Arc<dyn SessionApi>,
    query: &str,
    stop_early: bool,
) -> CliResult<Vec<SearchResult>> {
    let timeout = ctx.search_timeout;
    ctx.out.status(&format!(
        "searching for '{query}' ({}s)…",
        timeout.as_secs()
    ));

    if !stop_early {
        return client
            .search(query, timeout)
            .map_err(|e| CliError::connection(format!("search failed: {e}")));
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let worker = {
        let client = Arc::clone(client);
        let query = query.to_string();
        let cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            client.search_with_cancel(&query, timeout, Some(cancel))
        })
    };

    super::poll_until(timeout, Duration::from_millis(100), || {
        (client.get_search_results_count(query) > 0).then_some(())
    });
    cancel.store(true, Ordering::Relaxed);
    let _ = worker.join();
    Ok(client.get_search_results(query))
}

/// Drop requests that would collide with one already queued.
///
/// Two collisions are possible and both lose data silently: the library keys
/// downloads by a hash of the remote path, so the same path from two peers
/// shadows itself, and two different paths sharing a basename would be written
/// to the same local file. Keep the first (best-ranked) of each and say what
/// was dropped rather than transferring less than asked.
fn dedupe(
    requests: impl IntoIterator<Item = Request>,
    download_dir: &str,
    out: &Out,
) -> Vec<Request> {
    let mut targets: HashMap<PathBuf, Request> = HashMap::new();
    let mut kept = Vec::new();
    for request in requests {
        let target = local_path(download_dir, &request.path);
        if let Some(queued) = targets.get(&target) {
            let collision = if queued.path == request.path {
                format!("same remote path already queued from {}", queued.user)
            } else {
                format!(
                    "would overwrite the file already queued from {}",
                    queued.user
                )
            };
            out.warn(&format!(
                "skipping {} from {}: {collision}",
                request.path, request.user
            ));
            continue;
        }
        targets.insert(target, request.clone());
        kept.push(request);
    }
    kept
}

/// A transfer whose size we do not know would be finalized against zero bytes
/// and saved empty, so look it up in the peer's share listing first.
fn resolve_sizes(
    ctx: &Ctx,
    client: &dyn SessionApi,
    requests: Vec<Request>,
) -> CliResult<Vec<Request>> {
    if requests.iter().all(|request| request.size > 0) {
        return Ok(requests);
    }

    let mut sizes: HashMap<String, HashMap<String, u64>> = HashMap::new();
    let mut resolved = Vec::with_capacity(requests.len());
    for mut request in requests {
        if request.size > 0 {
            resolved.push(request);
            continue;
        }
        if !sizes.contains_key(&request.user) {
            ctx.out.status(&format!(
                "looking up the size of {} in {}'s shares…",
                request.path, request.user
            ));
            let listing = social::fetch_listing(
                client,
                &request.user,
                Duration::from_secs(20),
            )?;
            sizes.insert(
                request.user.clone(),
                listing
                    .iter()
                    .flat_map(|directory| {
                        directory.files.iter().map(|(basename, size)| {
                            (
                                social::full_path(&directory.name, basename),
                                *size,
                            )
                        })
                    })
                    .collect(),
            );
        }
        request.size = sizes
            .get(&request.user)
            .and_then(|files| files.get(&request.path))
            .copied()
            .ok_or_else(|| {
                CliError::no_results(format!(
                    "{} does not share '{}' — pass --size to download it \
                     anyway",
                    request.user, request.path
                ))
            })?;
        resolved.push(request);
    }
    Ok(resolved)
}

/// Make sure the bytes have somewhere to land.
///
/// A daemon writes to its own filesystem and has already done this; creating
/// the same path here would leave a stray directory on the client machine and,
/// for a daemon on another host, could fail outright over a path that is none
/// of this machine's business.
fn ensure_download_dir(ctx: &Ctx) -> CliResult {
    if ctx.daemon.is_some() {
        return Ok(());
    }
    let path = expand_tilde(&ctx.download_dir);
    std::fs::create_dir_all(&path).map_err(|e| {
        CliError::usage(format!(
            "cannot use download directory {}: {e}",
            path.display()
        ))
    })
}

/// Fetch every request, at most `max_concurrent_downloads` at a time, emitting
/// each result as it lands. Any failure fails the command.
fn download_all(
    ctx: &Ctx,
    client: &Arc<dyn SessionApi>,
    requests: Vec<Request>,
    timeout: Duration,
) -> CliResult {
    let requests = dedupe(requests, &ctx.download_dir, &ctx.out);
    let workers = ctx.max_concurrent_downloads.clamp(1, requests.len().max(1));
    let queue = Mutex::new(VecDeque::from(requests));
    let (sender, receiver) = channel();

    let mut failures = std::thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let queue = &queue;
            scope.spawn(move || {
                while let Some(request) =
                    queue.lock().ok().and_then(|mut q| q.pop_front())
                {
                    let outcome = download_one(
                        client.as_ref(),
                        &ctx.out,
                        &request,
                        &ctx.download_dir,
                        timeout,
                    );
                    if sender.send((request, outcome)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);

        let mut failures = Vec::new();
        for (request, outcome) in receiver {
            match outcome {
                Ok(record) => ctx.out.emit(&record),
                Err(error) => {
                    ctx.out.error(&format!("{}: {error}", request.path));
                    failures.push(error);
                }
            }
        }
        failures
    });

    if failures.is_empty() {
        return Ok(());
    }
    if failures.len() == 1 {
        return Err(failures.remove(0));
    }
    let n = failures.len();
    if failures.iter().all(|f| f.exit == Exit::Timeout) {
        return Err(CliError::new(
            Exit::Timeout,
            format!("{n} downloads timed out"),
        ));
    }
    Err(CliError::transfer(format!("{n} downloads failed")))
}

/// Queue one transfer and watch it to a verdict. The library reports failure
/// on the status channel rather than through the call that starts the
/// transfer, so the channel is the only thing worth believing.
fn download_one(
    client: &dyn SessionApi,
    out: &Out,
    request: &Request,
    download_dir: &str,
    timeout: Duration,
) -> CliResult<DownloadRecord> {
    // Clear any earlier attempt: entries are keyed by a hash of the remote
    // path, so a stale one would shadow this transfer.
    let _ = client.remove_download(&request.user, &request.path);

    let (_download, updates) = client
        .download(
            request.path.clone(),
            request.user.clone(),
            request.size,
            download_dir.to_string(),
        )
        .map_err(|e| CliError::transfer(format!("cannot start: {e}")))?;

    out.status(&format!(
        "requesting {} from {} ({} bytes)…",
        request.path, request.user, request.size
    ));

    let deadline = Instant::now() + timeout;
    let mut last_report = Instant::now();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = client.remove_download(&request.user, &request.path);
            return Err(CliError::timeout(format!(
                "gave up after {}s",
                timeout.as_secs()
            )));
        }
        match updates.recv_timeout(remaining.min(Duration::from_millis(250))) {
            Ok(DownloadStatus::Completed) => {
                let file = local_path(download_dir, &request.path);
                return Ok(DownloadRecord {
                    user: request.user.clone(),
                    path: request.path.clone(),
                    size: request.size,
                    file: file.display().to_string(),
                });
            }
            Ok(DownloadStatus::Failed(reason)) => {
                let _ = client.remove_download(&request.user, &request.path);
                return Err(CliError::transfer(
                    reason.unwrap_or_else(|| "the transfer failed".to_string()),
                ));
            }
            Ok(DownloadStatus::Queued) => {
                out.status(&format!(
                    "queued behind {}'s uploads",
                    request.user
                ));
            }
            Ok(DownloadStatus::InProgress {
                bytes_downloaded,
                total_bytes,
                speed_bytes_per_sec,
            }) => {
                if last_report.elapsed() >= Duration::from_secs(1) {
                    last_report = Instant::now();
                    out.status(&format!(
                        "{}: {bytes_downloaded}/{total_bytes} bytes ({:.0} KB/s)",
                        request.path,
                        speed_bytes_per_sec / 1024.0
                    ));
                }
            }
            Ok(DownloadStatus::Paused { .. } | DownloadStatus::TimedOut)
            | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let _ = client.remove_download(&request.user, &request.path);
                return Err(CliError::transfer(
                    "the transfer ended without reporting a result",
                ));
            }
        }
    }
}

/// Parse `search`/`browse` output back into download requests.
///
/// Both shapes this program emits are accepted: a JSON object per line, or
/// tab-separated text whose first field is the user and last field is the
/// remote path, with the size in between when there is one.
fn read_requests(
    input: &mut impl BufRead,
    out: &Out,
) -> CliResult<Vec<Request>> {
    let mut requests = Vec::new();
    for (number, line) in input.lines().enumerate() {
        let line = line
            .map_err(|e| CliError::usage(format!("cannot read stdin: {e}")))?;
        match parse_request(&line) {
            Ok(Some(request)) => requests.push(request),
            Ok(None) => {}
            Err(reason) => {
                out.warn(&format!("stdin line {}: {reason}", number + 1));
            }
        }
    }
    Ok(requests)
}

/// `Ok(None)` for a blank line, `Err` for something unusable.
// aislop-ignore-next-line complexity/function-too-long -- scanner bug, not a long function: the `'{'` char literal below reads as an opening brace, so aislop's counter never closes and measures to EOF. It is 33 lines.
fn parse_request(line: &str) -> Result<Option<Request>, String> {
    if line.trim().is_empty() {
        return Ok(None);
    }

    let (user, path, size) = if line.trim_start().starts_with('{') {
        let record: JsonRequest = serde_json::from_str(line.trim())
            .map_err(|e| format!("invalid JSON record: {e}"))?;
        (record.user, record.path, record.size)
    } else {
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 2 {
            return Err(
                "expected tab-separated USER…PATH or a JSON record".to_string()
            );
        }
        let size = if fields.len() >= 3 {
            fields[1].trim().parse().unwrap_or(0)
        } else {
            0
        };
        (
            fields[0].trim().to_string(),
            fields[fields.len() - 1].trim().to_string(),
            size,
        )
    };

    if user.is_empty() || path.is_empty() {
        return Err("user and path must not be empty".to_string());
    }
    Ok(Some(Request { user, path, size }))
}

/// The subset of a `search`/`browse` JSON record a download needs. Extra
/// fields are ignored, so a record enriched by `jq` still parses.
#[derive(serde::Deserialize)]
struct JsonRequest {
    user: String,
    path: String,
    #[serde(default)]
    size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use soulseek_rs::File;
    use std::collections::HashMap;

    fn file(name: &str, size: u64, bitrate: Option<u32>) -> File {
        let mut attribs = HashMap::new();
        if let Some(bitrate) = bitrate {
            attribs.insert(ATTR_BITRATE, bitrate);
            attribs.insert(ATTR_DURATION, 200);
        }
        File {
            username: String::new(),
            name: name.to_string(),
            size,
            attribs,
        }
    }

    fn result(
        user: &str,
        slots: u8,
        speed: u32,
        files: Vec<File>,
    ) -> SearchResult {
        SearchResult {
            token: 1,
            files,
            slots,
            speed,
            username: user.to_string(),
        }
    }

    fn paths_of(found: &[SearchRecord]) -> Vec<String> {
        found.iter().map(|c| c.path.clone()).collect()
    }

    fn sample() -> Vec<SearchResult> {
        vec![
            result("slow", 1, 100, vec![file("a\\low.mp3", 10, Some(128))]),
            result("busy", 0, 900, vec![file("b\\best.mp3", 90, Some(320))]),
            result(
                "fast",
                1,
                500,
                vec![
                    file("c\\good.mp3", 50, Some(256)),
                    file("c\\unknown.mp3", 70, None),
                ],
            ),
        ]
    }

    #[test]
    fn candidates_flatten_every_file_of_every_peer() {
        let found = candidates(&sample(), &Filter::default());
        assert_eq!(found.len(), 4);
        assert!(found.iter().any(|c| c.user == "fast" && c.size == 70));
    }

    #[test]
    fn candidates_carry_bitrate_duration_and_slot_state() {
        let found = candidates(&sample(), &Filter::default());
        let best = found.iter().find(|c| c.path == "b\\best.mp3").unwrap();
        assert_eq!(best.bitrate, Some(320));
        assert_eq!(best.duration, Some(200));
        assert!(!best.free_slot);
        assert_eq!(best.speed, 900);
    }

    /// Build a filter the way a command does, so the tests cover the flag
    /// normalisation (case, leading dots) rather than assuming it.
    fn filter(build: impl FnOnce(&mut FilterArgs)) -> Filter {
        let mut args = FilterArgs::default();
        build(&mut args);
        Filter::from(&args)
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn min_bitrate_drops_lower_and_unknown_bitrates() {
        let filter = filter(|a| a.min_bitrate = Some(256));
        assert_eq!(
            paths_of(&candidates(&sample(), &filter)),
            ["b\\best.mp3", "c\\good.mp3"]
        );
    }

    #[test]
    fn free_slots_only_keeps_peers_advertising_a_slot() {
        let filter = filter(|a| a.free_slots = true);
        let found = candidates(&sample(), &filter);
        assert!(found.iter().all(|c| c.free_slot));
        assert!(found.iter().all(|c| c.user != "busy"));
    }

    #[test]
    fn exclude_drops_a_path_containing_the_term_whatever_its_case() {
        let filter = filter(|a| a.exclude = strings(&["LOW"]));
        let found = candidates(&sample(), &filter);
        assert!(found.iter().all(|c| c.path != "a\\low.mp3"));
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn exclude_matches_any_term_not_all_of_them() {
        let filter = filter(|a| a.exclude = strings(&["low", "unknown"]));
        assert_eq!(
            paths_of(&candidates(&sample(), &filter)),
            ["b\\best.mp3", "c\\good.mp3"]
        );
    }

    #[test]
    fn extension_keeps_only_that_type_however_it_is_written() {
        let mixed = vec![result(
            "peer",
            1,
            1,
            vec![
                file("a.mp3", 1, None),
                file("b.FLAC", 2, None),
                file("c.ogg", 3, None),
            ],
        )];
        for spelling in [".flac", "flac", "FLAC", ".FLAC"] {
            let filter = filter(|a| a.extension = strings(&[spelling]));
            assert_eq!(
                paths_of(&candidates(&mixed, &filter)),
                ["b.FLAC"],
                "--extension {spelling} should match b.FLAC"
            );
        }
    }

    #[test]
    fn several_extensions_are_a_union() {
        let mixed = vec![result(
            "peer",
            1,
            1,
            vec![
                file("a.mp3", 1, None),
                file("b.flac", 2, None),
                file("c.ogg", 3, None),
            ],
        )];
        let filter = filter(|a| a.extension = strings(&["mp3", "ogg"]));
        assert_eq!(paths_of(&candidates(&mixed, &filter)), ["a.mp3", "c.ogg"]);
    }

    #[test]
    fn an_extensionless_file_never_satisfies_an_extension_filter() {
        let odd = vec![result("peer", 1, 1, vec![file("README", 1, None)])];
        let filter = filter(|a| a.extension = strings(&["mp3"]));
        assert!(candidates(&odd, &filter).is_empty());
    }

    #[test]
    fn a_dot_in_a_folder_name_is_not_the_files_extension() {
        // Only the last path segment decides the extension, or every file under
        // "Album (2004).v0" would look like a .v0 file.
        let nested = vec![result(
            "peer",
            1,
            1,
            vec![file("Album (2004).v0\\track.mp3", 1, None)],
        )];
        assert!(
            candidates(&nested, &filter(|a| a.extension = strings(&["v0"])))
                .is_empty(),
            "the folder's .v0 suffix must not count as the file's extension"
        );
        assert_eq!(
            paths_of(&candidates(
                &nested,
                &filter(|a| a.extension = strings(&["mp3"]))
            )),
            ["Album (2004).v0\\track.mp3"]
        );
    }

    #[test]
    fn size_bounds_are_inclusive_on_both_ends() {
        let filter = filter(|a| {
            a.min_size = Some(50);
            a.max_size = Some(70);
        });
        assert_eq!(
            paths_of(&candidates(&sample(), &filter)),
            ["c\\good.mp3", "c\\unknown.mp3"]
        );
    }

    #[test]
    fn each_size_bound_works_without_the_other() {
        let floor = filter(|a| a.min_size = Some(70));
        assert_eq!(
            paths_of(&candidates(&sample(), &floor)),
            ["b\\best.mp3", "c\\unknown.mp3"]
        );
        let ceiling = filter(|a| a.max_size = Some(10));
        assert_eq!(paths_of(&candidates(&sample(), &ceiling)), ["a\\low.mp3"]);
    }

    #[test]
    fn filters_combine_as_an_and() {
        let filter = filter(|a| {
            a.free_slots = true;
            a.min_bitrate = Some(200);
            a.min_size = Some(20);
            a.exclude = strings(&["bad"]);
            a.extension = strings(&["mp3"]);
        });
        assert_eq!(paths_of(&candidates(&sample(), &filter)), ["c\\good.mp3"]);
    }

    #[test]
    fn a_filter_that_rejects_everything_is_the_no_results_verdict() {
        let filter = filter(|a| a.min_size = Some(u64::MAX));
        let error = ranked(&sample(), &filter, SortKey::Best, 0, "q")
            .expect_err("nothing survives the filter");
        assert_eq!(error.exit, Exit::NoResults);
    }

    #[test]
    fn best_prefers_a_free_slot_over_a_higher_bitrate() {
        let mut found = candidates(&sample(), &Filter::default());
        rank(&mut found, SortKey::Best);
        assert_eq!(found[0].path, "c\\good.mp3");
        assert_eq!(found[0].user, "fast");
    }

    #[test]
    fn sort_modes_lead_with_their_own_key() {
        let mut found = candidates(&sample(), &Filter::default());
        rank(&mut found, SortKey::Size);
        assert_eq!(found[0].size, 90);
        rank(&mut found, SortKey::Bitrate);
        assert_eq!(found[0].bitrate, Some(320));
        rank(&mut found, SortKey::Speed);
        assert_eq!(found[0].speed, 900);
    }

    #[test]
    fn ranking_is_deterministic_for_equivalent_results() {
        let tie = vec![
            result("zed", 1, 1, vec![file("x.mp3", 1, Some(128))]),
            result("amy", 1, 1, vec![file("x.mp3", 1, Some(128))]),
        ];
        let mut found = candidates(&tie, &Filter::default());
        rank(&mut found, SortKey::Best);
        assert_eq!(found[0].user, "amy");
    }

    #[test]
    fn local_path_uses_the_last_segment_of_a_windows_style_path() {
        assert_eq!(
            local_path("/music", "@@abc\\Albums\\01 - Track.mp3"),
            PathBuf::from("/music/01 - Track.mp3")
        );
        assert_eq!(
            local_path("/music", "plain.mp3"),
            PathBuf::from("/music/plain.mp3")
        );
        assert_eq!(
            local_path("/music", "unix/style/file.mp3"),
            PathBuf::from("/music/file.mp3")
        );
    }

    #[test]
    fn stdin_accepts_the_json_records_search_emits() {
        let request = parse_request(
            r#"{"user":"bob","path":"a\\b.mp3","size":42,"bitrate":320}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(request.user, "bob");
        assert_eq!(request.path, "a\\b.mp3");
        assert_eq!(request.size, 42);
    }

    #[test]
    fn stdin_accepts_the_text_records_search_emits() {
        let request = parse_request("bob\t4096\t320\t@@x\\track.mp3")
            .unwrap()
            .unwrap();
        assert_eq!(request.user, "bob");
        assert_eq!(request.size, 4096);
        assert_eq!(request.path, "@@x\\track.mp3");
    }

    #[test]
    fn stdin_accepts_a_bare_user_and_path_pair() {
        let request = parse_request("bob\t@@x\\track.mp3").unwrap().unwrap();
        assert_eq!(request.user, "bob");
        assert_eq!(request.path, "@@x\\track.mp3");
        assert_eq!(request.size, 0);
    }

    #[test]
    fn stdin_skips_blank_lines_and_rejects_junk() {
        assert_eq!(parse_request("   ").unwrap(), None);
        assert!(parse_request("nonsense").is_err());
        assert!(parse_request("{\"path\":\"a\"}").is_err());
        assert!(parse_request("\tpath-only").is_err());
    }

    #[test]
    fn a_bad_line_does_not_discard_the_good_ones() {
        let input = "bob\t10\t320\ta.mp3\nnonsense\n\nsue\tb.mp3\n";
        let out = Out::new(false, true);
        let requests = read_requests(&mut input.as_bytes(), &out).unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[1].user, "sue");
    }

    fn request(user: &str, path: &str) -> Request {
        Request {
            user: user.into(),
            path: path.into(),
            size: 1,
        }
    }

    #[test]
    fn duplicate_remote_paths_are_dropped_because_the_store_keys_by_path() {
        let out = Out::new(false, true);
        let kept = dedupe(
            vec![
                request("a", "same.mp3"),
                request("b", "same.mp3"),
                request("c", "other.mp3"),
            ],
            "/music",
            &out,
        );
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].user, "a");
        assert_eq!(kept[1].path, "other.mp3");
    }

    #[test]
    fn requests_that_would_overwrite_the_same_local_file_are_dropped() {
        // Different remote paths, same basename: both would be written to
        // /music/track.mp3 and the slower transfer would clobber the other.
        let out = Out::new(false, true);
        let kept = dedupe(
            vec![
                request("a", "@@one\\album\\track.mp3"),
                request("b", "@@two\\other\\track.mp3"),
            ],
            "/music",
            &out,
        );
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].user, "a");
    }

    #[test]
    fn stdin_rejects_a_json_record_with_empty_fields() {
        assert!(parse_request(r#"{"user":"","path":"a.mp3"}"#).is_err());
        assert!(parse_request(r#"{"user":"bob","path":""}"#).is_err());
    }

    #[test]
    fn stdin_ignores_extra_json_fields_a_filter_may_have_added() {
        let request =
            parse_request(r#"{"user":"bob","path":"a.mp3","size":7,"tag":1}"#)
                .unwrap()
                .unwrap();
        assert_eq!(request.size, 7);
    }
}
