// Core modules
pub mod actor;
pub mod client;
pub mod dispatcher;
pub mod error;
pub mod message;
pub mod peer;
pub mod server;
pub mod types;
#[macro_use]
pub mod utils;
// Prelude module for commonly used items
pub mod prelude {
    pub use crate::types::{
        DownloadResult, DownloadStatus, File, FileSearchResult, Transfer,
    };
    pub use crate::{debug, error, info, trace, warn};
    pub use crate::{Client, PeerAddress};
}

// Re-export commonly used types
pub use client::Client;
pub use error::{Result, SoulseekRs};
pub use server::PeerAddress;
pub use types::{
    DownloadResult, DownloadStatus, File, FileSearchResult, Transfer,
};
