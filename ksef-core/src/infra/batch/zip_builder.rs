use std::io::{Cursor, Write};

use base64::Engine;
use sha2::{Digest, Sha256};
use zip::write::SimpleFileOptions;

use crate::domain::batch::{BatchArchive, BatchFileInfo, BatchFilePartInfo};
use crate::error::KSeFError;
use crate::ports::batch_archive_builder::BatchArchiveBuilder;

pub struct BatchFileBuilder {
    max_part_size_bytes: usize,
}

impl Default for BatchFileBuilder {
    fn default() -> Self {
        Self {
            max_part_size_bytes: 5 * 1024 * 1024,
        }
    }
}

impl BatchFileBuilder {
    #[must_use]
    pub fn new(max_part_size_bytes: usize) -> Self {
        Self {
            max_part_size_bytes,
        }
    }

    pub fn build(&self, files: &[(String, Vec<u8>)]) -> Result<BatchArchive, KSeFError> {
        if files.is_empty() {
            return Err(KSeFError::InvoiceSubmissionFailed(
                "batch archive cannot be empty".to_string(),
            ));
        }
        if self.max_part_size_bytes == 0 {
            return Err(KSeFError::InvoiceSubmissionFailed(
                "max_part_size_bytes must be greater than zero".to_string(),
            ));
        }

        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        for (name, data) in files {
            if name.trim().is_empty() {
                return Err(KSeFError::InvoiceSubmissionFailed(
                    "batch file name cannot be empty".to_string(),
                ));
            }
            writer.start_file(name, options).map_err(|e| {
                KSeFError::InvoiceSubmissionFailed(format!(
                    "failed to add '{name}' to zip archive: {e}"
                ))
            })?;
            writer.write_all(data).map_err(|e| {
                KSeFError::InvoiceSubmissionFailed(format!(
                    "failed to write '{name}' to zip archive: {e}"
                ))
            })?;
        }

        writer.finish().map_err(|e| {
            KSeFError::InvoiceSubmissionFailed(format!("failed to finalize zip archive: {e}"))
        })?;
        let zip_bytes = cursor.into_inner();
        let file_hash = sha256_base64(&zip_bytes);
        let file_size_bytes = u64::try_from(zip_bytes.len())
            .map_err(|_| KSeFError::InvoiceSubmissionFailed("zip archive too large".to_string()))?;

        let file_info = BatchFileInfo {
            file_name: "batch.zip".to_string(),
            file_size_bytes,
            file_hash_sha256_base64: file_hash,
        };
        let parts = split_parts(&zip_bytes, self.max_part_size_bytes)?;

        Ok(BatchArchive {
            zip_bytes,
            file_info,
            parts,
        })
    }
}

impl BatchArchiveBuilder for BatchFileBuilder {
    fn build_archive(&self, files: &[(String, Vec<u8>)]) -> Result<BatchArchive, KSeFError> {
        self.build(files)
    }
}

fn sha256_base64(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    base64::engine::general_purpose::STANDARD.encode(digest)
}

fn split_parts(
    data: &[u8],
    max_part_size_bytes: usize,
) -> Result<Vec<BatchFilePartInfo>, KSeFError> {
    if data.is_empty() {
        return Err(KSeFError::InvoiceSubmissionFailed(
            "zip data cannot be empty".to_string(),
        ));
    }

    let mut parts = Vec::new();
    let mut offset = 0usize;
    let mut part_number = 1u32;

    while offset < data.len() {
        let remaining = data.len() - offset;
        let size = remaining.min(max_part_size_bytes);
        let chunk = &data[offset..offset + size];
        let offset_bytes = u64::try_from(offset)
            .map_err(|_| KSeFError::InvoiceSubmissionFailed("offset exceeds u64".to_string()))?;
        let size_bytes = u64::try_from(size)
            .map_err(|_| KSeFError::InvoiceSubmissionFailed("part size exceeds u64".to_string()))?;

        parts.push(BatchFilePartInfo {
            part_number,
            offset_bytes,
            size_bytes,
            hash_sha256_base64: sha256_base64(chunk),
        });
        offset += size;
        part_number += 1;
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn empty_input_returns_error() {
        let builder = BatchFileBuilder::default();
        let err = builder.build(&[]).unwrap_err();
        assert!(matches!(err, KSeFError::InvoiceSubmissionFailed(_)));
    }

    #[test]
    fn single_file_archive_is_valid_zip() {
        let builder = BatchFileBuilder::new(1024 * 1024);
        let archive = builder
            .build(&[("invoice1.xml".to_string(), b"<Faktura>1</Faktura>".to_vec())])
            .unwrap();

        let cursor = Cursor::new(archive.zip_bytes);
        let mut zip = zip::ZipArchive::new(cursor).unwrap();
        assert_eq!(zip.len(), 1);
        let mut file = zip.by_name("invoice1.xml").unwrap();
        let mut body = String::new();
        file.read_to_string(&mut body).unwrap();
        assert_eq!(body, "<Faktura>1</Faktura>");
    }

    #[test]
    fn multi_file_archive_contains_all_entries() {
        let builder = BatchFileBuilder::new(1024 * 1024);
        let archive = builder
            .build(&[
                ("a.xml".to_string(), b"a".to_vec()),
                ("b.xml".to_string(), b"b".to_vec()),
                ("c.xml".to_string(), b"c".to_vec()),
            ])
            .unwrap();

        let cursor = Cursor::new(archive.zip_bytes);
        let mut zip = zip::ZipArchive::new(cursor).unwrap();
        assert_eq!(zip.len(), 3);
        assert!(zip.by_name("a.xml").is_ok());
        assert!(zip.by_name("b.xml").is_ok());
        assert!(zip.by_name("c.xml").is_ok());
    }

    #[test]
    fn split_parts_respects_max_part_size() {
        let builder = BatchFileBuilder::new(128);
        let files = (0..80)
            .map(|idx| {
                let content = format!("invoice-{idx}-{}", "0123456789abcdef".repeat(8));
                (format!("invoice-{idx}.xml"), content.into_bytes())
            })
            .collect::<Vec<_>>();
        let archive = builder.build(&files).unwrap();
        assert!(archive.parts.len() > 1);
        assert!(
            archive
                .parts
                .iter()
                .all(|part| usize::try_from(part.size_bytes).unwrap() <= 128)
        );
    }

    #[test]
    fn part_hashes_match_data_chunks() {
        let builder = BatchFileBuilder::new(1024);
        let archive = builder
            .build(&[
                ("one.xml".to_string(), vec![b'a'; 2000]),
                ("two.xml".to_string(), vec![b'b'; 2000]),
            ])
            .unwrap();

        for part in &archive.parts {
            let offset = usize::try_from(part.offset_bytes).unwrap();
            let size = usize::try_from(part.size_bytes).unwrap();
            let chunk = &archive.zip_bytes[offset..offset + size];
            assert_eq!(part.hash_sha256_base64, super::sha256_base64(chunk));
        }
    }
}
