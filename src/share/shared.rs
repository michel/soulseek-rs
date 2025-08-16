use crate::types::File;
use crate::{info, warn};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::matches::matches_query;

#[derive(Clone)]
pub struct SharedFile {
    pub key: String,
    pub value: File,
}

pub struct Shared {
    files: Arc<Mutex<Vec<SharedFile>>>,
    folders: Arc<Mutex<Vec<String>>>,
}

impl Shared {
    pub fn new() -> Self {
        Self {
            files: Arc::new(Mutex::new(Vec::new())),
            folders: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn scan_folder(&self, folder: &str) {
        let path = Path::new(folder);
        if !path.exists() {
            warn!("Folder {} does not exist", folder);
            return;
        }

        if !path.is_dir() {
            warn!("{} is not a directory", folder);
            return;
        }

        self.folders.lock().unwrap().push(folder.to_string());

        let mut file_count = 0;
        self.scan_recursive(path, &mut file_count);

        info!(
            "Scan folder {} completed, {} files shared",
            folder, file_count
        );
    }

    fn scan_recursive(&self, path: &Path, file_count: &mut usize) {
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();

                if path.is_file() {
                    if let Ok(metadata) = entry.metadata() {
                        let file_path = path.to_string_lossy().to_string();

                        // Create a key from the last 2 path components
                        let components: Vec<&str> = file_path
                            .split(std::path::MAIN_SEPARATOR)
                            .collect();
                        let key = if components.len() >= 2 {
                            components[components.len().saturating_sub(2)..]
                                .join(&std::path::MAIN_SEPARATOR.to_string())
                        } else {
                            file_path.clone()
                        };

                        let shared_file = SharedFile {
                            key,
                            value: File {
                                username: String::new(), // Will be filled when sending results
                                name: file_path,
                                size: metadata.len(),
                                attribs: HashMap::new(),
                            },
                        };

                        self.files.lock().unwrap().push(shared_file);
                        *file_count += 1;
                    }
                } else if path.is_dir() {
                    self.scan_recursive(&path, file_count);
                }
            }
        }
    }

    pub fn search(&self, query: &str) -> Vec<SharedFile> {
        let files = self.files.lock().unwrap();
        files
            .iter()
            .filter(|file| matches_query(&file.key, query))
            .cloned()
            .collect()
    }

    pub fn get_file_count(&self) -> usize {
        self.files.lock().unwrap().len()
    }

    pub fn get_folder_count(&self) -> usize {
        self.folders.lock().unwrap().len()
    }
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}
