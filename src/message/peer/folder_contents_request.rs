use crate::message::{Message, MessageHandler};
use crate::peer::PeerOperation;
use crate::{debug, trace};
use std::sync::mpsc::Sender;

pub struct FolderContentsRequestHandler;

impl MessageHandler<PeerOperation> for FolderContentsRequestHandler {
    fn get_code(&self) -> u32 {
        36
    }

    fn handle(&self, message: &mut Message, _sender: Sender<PeerOperation>) {
        let num_folders = message.read_int32();

        // For now, we'll just log the request
        // In a full implementation, we would read the folder paths and respond with the contents
        debug!("Received FolderContentsRequest for {} folders", num_folders);

        for _ in 0..num_folders {
            let folder_path = message.read_string();
            trace!("Folder requested: {}", folder_path);
        }

        // TODO: Implement actual folder contents response
    }
}
