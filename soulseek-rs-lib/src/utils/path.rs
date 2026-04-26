use std::path::PathBuf;

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix('~')
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(stripped.trim_start_matches('/'));
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_tilde_returned_as_is() {
        assert_eq!(expand_tilde("/abs/path"), PathBuf::from("/abs/path"));
        assert_eq!(expand_tilde("rel/path"), PathBuf::from("rel/path"));
    }

    #[test]
    fn tilde_expanded_when_home_set() {
        // SAFETY: tests run single-threaded by default in this crate; if you
        // ever run with --test-threads >1 this needs serialization.
        unsafe {
            std::env::set_var("HOME", "/home/test");
        }
        assert_eq!(expand_tilde("~"), PathBuf::from("/home/test"));
        assert_eq!(
            expand_tilde("~/Downloads"),
            PathBuf::from("/home/test/Downloads")
        );
    }
}
