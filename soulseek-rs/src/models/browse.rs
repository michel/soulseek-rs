//! Model for browsing another user's shared files as a collapsible tree.
//!
//! The library returns a *flat* `Vec<SharedDirectory>` where each directory
//! carries its full backslash-separated path and only directories that directly
//! contain files are present. This module synthesizes the intermediate folders
//! into a real nested tree, and provides a flattened, expansion-aware row view
//! for rendering and navigation.

use soulseek_rs::SharedDirectory;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// A node in the browse tree: a folder (with children) or a file leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseNode {
    Folder {
        /// Last path segment (display name).
        name: String,
        /// Full backslash path (expansion key + display).
        path: String,
        children: Vec<Self>,
    },
    File {
        /// Basename (display name).
        name: String,
        /// Full backslash virtual path — the exact key used to download it.
        path: String,
        size: u64,
    },
}

impl BrowseNode {
    fn sort_key(&self) -> String {
        match self {
            Self::Folder { name, .. } | Self::File { name, .. } => {
                name.to_lowercase()
            }
        }
    }
}

/// The synthesized tree plus counts for the title bar.
#[derive(Debug, Default)]
pub struct BuiltTree {
    pub nodes: Vec<BrowseNode>,
    pub file_count: usize,
    pub folder_count: usize,
}

/// Intermediate builder keyed by path segment while synthesizing folders.
#[derive(Default)]
struct Builder {
    full_path: String,
    folders: HashMap<String, Self>,
    /// `(basename, size, full_virtual_path)`.
    files: Vec<(String, u64, String)>,
}

impl Builder {
    fn into_nodes(
        self,
        file_count: &mut usize,
        folder_count: &mut usize,
    ) -> Vec<BrowseNode> {
        let mut folders: Vec<BrowseNode> = self
            .folders
            .into_iter()
            .map(|(name, child)| {
                *folder_count += 1;
                let path = child.full_path.clone();
                let children = child.into_nodes(file_count, folder_count);
                BrowseNode::Folder {
                    name,
                    path,
                    children,
                }
            })
            .collect();
        folders.sort_by_key(BrowseNode::sort_key);

        let mut files: Vec<BrowseNode> = self
            .files
            .into_iter()
            .map(|(name, size, path)| {
                *file_count += 1;
                BrowseNode::File { name, path, size }
            })
            .collect();
        files.sort_by_key(BrowseNode::sort_key);

        folders.extend(files);
        folders
    }
}

/// Build a nested tree from the peer's flat directory listing, synthesizing
/// intermediate folders. File download paths are taken from the *original*
/// directory name so they are byte-exact for the download request.
#[must_use]
pub fn build_browse_tree(directories: &[SharedDirectory]) -> BuiltTree {
    let mut root = Builder::default();

    for dir in directories {
        // Walk/synthesize the folder chain, tolerating leading/trailing/doubled
        // backslashes by dropping empty segments.
        let mut node = &mut root;
        let mut parent_path = String::new();
        for segment in dir.name.split('\\').filter(|s| !s.is_empty()) {
            let child_path = if parent_path.is_empty() {
                segment.to_string()
            } else {
                format!("{parent_path}\\{segment}")
            };
            node =
                node.folders.entry(segment.to_string()).or_insert_with(|| {
                    Builder {
                        full_path: child_path.clone(),
                        ..Builder::default()
                    }
                });
            parent_path = child_path;
        }

        // File paths come from the original name so downloads match exactly.
        let dir_path = dir.name.trim_end_matches('\\');
        for (basename, size) in &dir.files {
            let file_path = if dir_path.is_empty() {
                basename.clone()
            } else {
                format!("{dir_path}\\{basename}")
            };
            node.files.push((basename.clone(), *size, file_path));
        }
    }

    let mut file_count = 0;
    let mut folder_count = 0;
    let nodes = root.into_nodes(&mut file_count, &mut folder_count);
    BuiltTree {
        nodes,
        file_count,
        folder_count,
    }
}

/// One rendered/navigable row of the flattened tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseRow {
    pub depth: usize,
    pub is_folder: bool,
    pub expanded: bool,
    pub name: String,
    pub path: String,
    pub size: Option<u64>,
}

/// Flatten the tree into visible rows, descending only into expanded folders.
#[must_use]
pub fn flatten_browse(
    nodes: &[BrowseNode],
    expanded: &HashSet<String>,
) -> Vec<BrowseRow> {
    let mut rows = Vec::new();
    flatten_into(nodes, 0, expanded, &mut rows);
    rows
}

fn flatten_into(
    nodes: &[BrowseNode],
    depth: usize,
    expanded: &HashSet<String>,
    rows: &mut Vec<BrowseRow>,
) {
    for node in nodes {
        match node {
            BrowseNode::Folder {
                name,
                path,
                children,
            } => {
                let is_expanded = expanded.contains(path);
                rows.push(BrowseRow {
                    depth,
                    is_folder: true,
                    expanded: is_expanded,
                    name: name.clone(),
                    path: path.clone(),
                    size: None,
                });
                if is_expanded {
                    flatten_into(children, depth + 1, expanded, rows);
                }
            }
            BrowseNode::File { name, path, size } => {
                rows.push(BrowseRow {
                    depth,
                    is_folder: false,
                    expanded: false,
                    name: name.clone(),
                    path: path.clone(),
                    size: Some(*size),
                });
            }
        }
    }
}

/// Find a node by its full path.
#[must_use]
pub fn find_node<'a>(
    nodes: &'a [BrowseNode],
    path: &str,
) -> Option<&'a BrowseNode> {
    for node in nodes {
        match node {
            BrowseNode::Folder {
                path: p, children, ..
            } => {
                if p == path {
                    return Some(node);
                }
                if let Some(found) = find_node(children, path) {
                    return Some(found);
                }
            }
            BrowseNode::File { path: p, .. } => {
                if p == path {
                    return Some(node);
                }
            }
        }
    }
    None
}

/// All file leaves under `node` as `(full_path, size)`, descending the tree
/// structurally (never string-prefix matching).
#[must_use]
pub fn files_under(node: &BrowseNode) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    collect_files(node, &mut out);
    out
}

fn collect_files(node: &BrowseNode, out: &mut Vec<(String, u64)>) {
    match node {
        BrowseNode::File { path, size, .. } => out.push((path.clone(), *size)),
        BrowseNode::Folder { children, .. } => {
            for child in children {
                collect_files(child, out);
            }
        }
    }
}

/// Loading state of an in-flight browse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseStatus {
    Loading,
    Loaded,
    Empty,
    TimedOut,
}

/// UI state for browsing one user.
pub struct BrowseState {
    pub username: String,
    pub status: BrowseStatus,
    pub tree: Vec<BrowseNode>,
    pub expanded: HashSet<String>,
    pub selected_row: usize,
    pub file_count: usize,
    pub folder_count: usize,
    pub requested_at: Instant,
}

impl BrowseState {
    #[must_use]
    pub fn loading(username: String) -> Self {
        Self {
            username,
            status: BrowseStatus::Loading,
            tree: Vec::new(),
            expanded: HashSet::new(),
            selected_row: 0,
            file_count: 0,
            folder_count: 0,
            requested_at: Instant::now(),
        }
    }

    /// Populate from a received listing: build the tree, auto-expand the top
    /// level so structure is visible, and set Loaded/Empty.
    pub fn load(&mut self, directories: &[SharedDirectory]) {
        let built = build_browse_tree(directories);
        self.expanded = built
            .nodes
            .iter()
            .filter_map(|n| match n {
                BrowseNode::Folder { path, .. } => Some(path.clone()),
                BrowseNode::File { .. } => None,
            })
            .collect();
        self.status = if built.nodes.is_empty() {
            BrowseStatus::Empty
        } else {
            BrowseStatus::Loaded
        };
        self.tree = built.nodes;
        self.file_count = built.file_count;
        self.folder_count = built.folder_count;
        self.selected_row = 0;
    }

    /// The flattened visible rows for the current expansion state.
    #[must_use]
    pub fn rows(&self) -> Vec<BrowseRow> {
        flatten_browse(&self.tree, &self.expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str, files: &[(&str, u64)]) -> SharedDirectory {
        SharedDirectory {
            name: name.to_string(),
            files: files.iter().map(|(n, s)| ((*n).to_string(), *s)).collect(),
        }
    }

    #[test]
    fn intermediate_folders_are_synthesized() {
        let tree =
            build_browse_tree(&[dir("share\\music\\album", &[("a.mp3", 10)])]);
        // share -> music -> album -> a.mp3
        assert_eq!(tree.file_count, 1);
        assert_eq!(tree.folder_count, 3);
        let BrowseNode::Folder {
            name,
            path,
            children,
        } = &tree.nodes[0]
        else {
            panic!("expected a folder");
        };
        assert_eq!(name, "share");
        assert_eq!(path, "share");
        assert!(
            matches!(&children[0], BrowseNode::Folder { name, .. } if name == "music")
        );
    }

    #[test]
    fn same_named_dirs_merge_regardless_of_order() {
        let tree = build_browse_tree(&[
            dir("share\\music\\album", &[("a.mp3", 1)]),
            dir("share\\music", &[("b.mp3", 2)]),
        ]);
        // A single "share" -> single "music" holding album/ and b.mp3.
        assert_eq!(tree.nodes.len(), 1);
        let BrowseNode::Folder {
            children: share_kids,
            ..
        } = &tree.nodes[0]
        else {
            panic!();
        };
        assert_eq!(share_kids.len(), 1); // just "music"
        let BrowseNode::Folder {
            children: music_kids,
            ..
        } = &share_kids[0]
        else {
            panic!();
        };
        // folders first (album), then files (b.mp3)
        assert_eq!(music_kids.len(), 2);
        assert!(
            matches!(&music_kids[0], BrowseNode::Folder { name, .. } if name == "album")
        );
        assert!(
            matches!(&music_kids[1], BrowseNode::File { name, .. } if name == "b.mp3")
        );
    }

    #[test]
    fn file_paths_are_byte_exact_from_original_name() {
        // Trailing backslash and a root-level (empty name) directory.
        let tree = build_browse_tree(&[
            dir("share\\music\\", &[("song.flac", 5)]),
            dir("", &[("root.txt", 3)]),
        ]);
        let files = collect_all(&tree.nodes);
        assert!(files.contains(&("share\\music\\song.flac".to_string(), 5)));
        assert!(files.contains(&("root.txt".to_string(), 3)));
    }

    fn collect_all(nodes: &[BrowseNode]) -> Vec<(String, u64)> {
        let mut out = Vec::new();
        for n in nodes {
            out.extend(files_under(n));
        }
        out
    }

    #[test]
    fn flatten_only_descends_into_expanded_folders() {
        let tree = build_browse_tree(&[dir("a\\b", &[("f.mp3", 1)])]);
        let mut expanded = HashSet::new();
        // Nothing expanded: only the top folder "a" shows.
        assert_eq!(flatten_browse(&tree.nodes, &expanded).len(), 1);
        expanded.insert("a".to_string());
        assert_eq!(flatten_browse(&tree.nodes, &expanded).len(), 2); // a, b
        expanded.insert("a\\b".to_string());
        assert_eq!(flatten_browse(&tree.nodes, &expanded).len(), 3); // a, b, f
    }

    #[test]
    fn files_under_collects_all_descendant_leaves() {
        let tree = build_browse_tree(&[
            dir("a\\b", &[("x.mp3", 1)]),
            dir("a\\c", &[("y.mp3", 2), ("z.mp3", 3)]),
        ]);
        let all = files_under(&tree.nodes[0]); // folder "a"
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn browse_state_load_auto_expands_top_level() {
        let mut state = BrowseState::loading("bob".to_string());
        state.load(&[dir("music\\album", &[("a.mp3", 1)])]);
        assert_eq!(state.status, BrowseStatus::Loaded);
        // Top-level "music" auto-expanded → music and album rows visible.
        assert!(state.rows().len() >= 2);

        let mut empty = BrowseState::loading("nobody".to_string());
        empty.load(&[]);
        assert_eq!(empty.status, BrowseStatus::Empty);
    }
}
