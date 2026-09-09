// The README is the crate's front page, and `cargo test --doc` compiles every
// example in it, so a signature change cannot leave the advertised usage stale.
#![doc = include_str!("../README.md")]

// The public surface is the client, its types, the wire codec and the share
// index. The actors, dispatcher, peer plumbing and download store are how the
// client is built, not what a host programs against, so they stay inside the
// crate and a change to them is not a change to the library's API.
pub(crate) mod actor;
pub mod client;
pub(crate) mod dispatcher;
pub(crate) mod download_store;
pub mod error;
pub mod message;
pub(crate) mod peer;
pub mod shares;
pub mod types;
#[macro_use]
pub mod utils;

// Prelude module for commonly used items
pub mod prelude {
    pub use crate::actor::server_actor::PeerAddress;
    pub use crate::types::{
        ClientVersion, DownloadStatus, File, Recommendation, RoomEvent,
        RoomInfo, RoomTicker, Search, SearchResult, SimilarUser, Transfer,
        UploadInfo, UploadStatus, UserInfo, UserInterests, UserPresence,
        UserStats, UserStatus,
    };
    pub use crate::{debug, error, info, trace, warn};
}

// Re-export commonly used types
pub use actor::server_actor::{PeerAddress, UserMessage};
pub use client::{Client, ClientSettings};
pub use error::{Result, SoulseekRs};
pub use message::peer::{SharedDirectory, SharedFileEntry};
pub use peer::{ConnectionType, ParseConnectionTypeError};
pub use types::{
    ClientVersion, DownloadStatus, File, Recommendation, RoomEvent, RoomInfo,
    RoomTicker, Search, SearchResult, SessionLoss, SimilarUser, Transfer,
    UploadInfo, UploadStatus, UserInfo, UserInterests, UserPresence, UserStats,
    UserStatus,
};
