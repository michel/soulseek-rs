// Core modules
pub mod actor;
pub mod client;
pub mod dispatcher;
pub mod error;
pub mod message;
pub mod peer;
pub mod types;
#[macro_use]
pub mod utils;

// Prelude module for commonly used items
pub mod prelude {
    pub use crate::actor::server_actor::PeerAddress;
    pub use crate::types::{
        DownloadResult, DownloadStatus, File, FileSearchResult, Transfer,
    };
    pub use crate::{debug, error, info, trace, warn};
}

// Re-export commonly used types
pub use actor::server_actor::PeerAddress;
pub use client::Client;
pub use error::{Result, SoulseekRs};
pub use types::{
    DownloadResult, DownloadStatus, File, FileSearchResult, Transfer,
};
