pub struct TempDir(tempfile::TempDir);
impl TempDir {
    pub fn new() -> Self {
        Self(tempfile::tempdir().unwrap())
    }
    pub fn path(&self) -> &std::path::Path {
        self.0.path()
    }
}
