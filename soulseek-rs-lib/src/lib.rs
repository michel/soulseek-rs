// The README is the crate's front page, and `cargo test --doc` compiles every
// example in it, so a signature change cannot leave the advertised usage stale.
#![doc = include_str!("../README.md")]

// Core modules
pub mod actor;
pub mod client;
pub mod dispatcher;
pub mod download_store;
pub mod error;
pub mod message;
pub mod peer;
pub mod shares;
pub mod types;
#[macro_use]
pub mod utils;

// Prelude module for commonly used items
pub mod prelude {
    pub use crate::actor::server_actor::PeerAddress;
    pub use crate::types::{
        ClientVersion, DownloadStatus, File, RoomEvent, RoomInfo, Search,
        SearchResult, Transfer, UploadInfo, UploadStatus, UserInfo,
        UserPresence, UserStats, UserStatus,
    };
    pub use crate::{debug, error, info, trace, warn};
}

// Re-export commonly used types
pub use actor::server_actor::{PeerAddress, UserMessage};
pub use client::{Client, ClientSettings};
pub use error::{Result, SoulseekRs};
pub use message::peer::SharedDirectory;
pub use types::{
    ClientVersion, DownloadStatus, File, RoomEvent, RoomInfo, Search,
    SearchResult, SessionLoss, Transfer, UploadInfo, UploadStatus, UserInfo,
    UserPresence, UserStats, UserStatus,
};
