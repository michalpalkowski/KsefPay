use crate::domain::batch::BatchArchive;
use crate::error::KSeFError;

/// Port: build KSeF batch archive payload from invoice files.
pub trait BatchArchiveBuilder: Send + Sync {
    fn build_archive(&self, files: &[(String, Vec<u8>)]) -> Result<BatchArchive, KSeFError>;
}
