use std::collections::HashMap;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct File {
    pub username: String,
    pub name: String,
    pub size: i32,
    pub attribs: HashMap<i32, i32>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct FileSearchResult {
    pub token: String,
    pub files: Vec<File>,
    pub slots: i8,
    pub speed: i32,
}

