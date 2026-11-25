pub mod generator;
pub mod settings;
pub mod vault;
pub mod autotype;
mod input;

#[cfg(test)]
pub(crate) mod test_util {
    use std::path::{Path, PathBuf};

    /// Temporary directory removed recursively on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let path = std::env::temp_dir().join(format!("ourlogins-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            TempDir(path)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
