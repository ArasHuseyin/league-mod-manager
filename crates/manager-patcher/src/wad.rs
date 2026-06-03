//! A self-contained WAD v3-style archive codec.
//!
//! League stores game assets in `.wad.client` archives whose table of contents
//! keys each entry by the XXH64 hash of its lowercased path. This module owns a
//! faithful-in-structure reader and writer for that layout: magic, version, a
//! reserved signature region, a checksum over the entry table, and a 32-byte
//! entry record per asset. Entries may be stored raw or gzip-compressed.
//!
//! This is deliberately the project's own engine, not a binding to existing
//! tools. It round-trips its own output exactly and is the foundation for the
//! overlay builder. Byte-exact compatibility with a live League installation
//! (ECDSA signatures, zstd, sub-chunked entries) is intentionally deferred; see
//! Phase 6 in `plan.md`.

use std::io::{self, Read, Write};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression as GzLevel;
use thiserror::Error;

const MAGIC: [u8; 2] = *b"RW";
const VERSION_MAJOR: u8 = 3;
const VERSION_MINOR: u8 = 0;
const SIGNATURE_LEN: usize = 256;
const ENTRY_LEN: usize = 32;
/// magic(2) + version(2) + signature(256) + checksum(8) + entryCount(4)
const HEADER_LEN: usize = 2 + 2 + SIGNATURE_LEN + 8 + 4;

#[derive(Debug, Error)]
pub enum WadError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("not a WAD archive: bad magic")]
    BadMagic,
    #[error("unsupported WAD version {major}.{minor}")]
    UnsupportedVersion { major: u8, minor: u8 },
    #[error("unknown entry compression code {0}")]
    UnknownCompression(u8),
    #[error("archive is truncated or malformed")]
    Truncated,
    #[error("checksum mismatch: table of contents is corrupt")]
    ChecksumMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Raw,
    Gzip,
}

impl Compression {
    fn code(self) -> u8 {
        match self {
            Compression::Raw => 0,
            Compression::Gzip => 1,
        }
    }

    fn from_code(code: u8) -> Result<Self, WadError> {
        match code {
            0 => Ok(Compression::Raw),
            1 => Ok(Compression::Gzip),
            other => Err(WadError::UnknownCompression(other)),
        }
    }
}

/// A decoded archive entry: its path hash plus the uncompressed contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WadEntry {
    pub path_hash: u64,
    pub data: Vec<u8>,
    pub compression: Compression,
}

/// Hash a path the way League keys WAD entries: XXH64 of the lowercased path.
pub fn path_hash(path: &str) -> u64 {
    xxh64(path.to_ascii_lowercase().as_bytes(), 0)
}

/// Builds a WAD archive entry by entry, then serializes it.
#[derive(Debug, Default)]
pub struct WadBuilder {
    entries: Vec<(u64, Vec<u8>, Compression)>,
}

impl WadBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an asset by its in-WAD path; the path is hashed with [`path_hash`].
    pub fn add(&mut self, path: &str, data: Vec<u8>, compression: Compression) -> &mut Self {
        self.entries.push((path_hash(path), data, compression));
        self
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Serialize the archive. Entries are written sorted by path hash, matching
    /// how WAD tables of contents are ordered for binary search lookups.
    pub fn write<W: Write>(&self, writer: &mut W) -> Result<(), WadError> {
        let mut entries: Vec<&(u64, Vec<u8>, Compression)> = self.entries.iter().collect();
        entries.sort_by_key(|(hash, _, _)| *hash);

        // Encode payloads up front so the table of contents can record absolute
        // offsets and on-disk sizes; data begins right after header + TOC.
        let mut toc = Vec::with_capacity(entries.len() * ENTRY_LEN);
        let mut payload = Vec::new();
        let mut offset_cursor = (HEADER_LEN + entries.len() * ENTRY_LEN) as u64;
        for (hash, data, compression) in &entries {
            let encoded = encode(data, *compression)?;
            let offset = u32::try_from(offset_cursor).map_err(|_| WadError::Truncated)?;
            let compressed_size = u32::try_from(encoded.len()).map_err(|_| WadError::Truncated)?;
            offset_cursor += encoded.len() as u64;

            toc.extend_from_slice(&hash.to_le_bytes());
            toc.extend_from_slice(&offset.to_le_bytes());
            toc.extend_from_slice(&compressed_size.to_le_bytes());
            toc.extend_from_slice(&(data.len() as u32).to_le_bytes());
            toc.push(compression.code());
            toc.extend_from_slice(&[0u8; 11]); // reserved: sub-chunk metadata, duplicates

            payload.extend_from_slice(&encoded);
        }
        debug_assert_eq!(toc.len(), entries.len() * ENTRY_LEN);

        let checksum = xxh64(&toc, 0);

        writer.write_all(&MAGIC)?;
        writer.write_all(&[VERSION_MAJOR, VERSION_MINOR])?;
        writer.write_all(&[0u8; SIGNATURE_LEN])?;
        writer.write_all(&checksum.to_le_bytes())?;
        writer.write_all(&(entries.len() as u32).to_le_bytes())?;
        writer.write_all(&toc)?;
        writer.write_all(&payload)?;
        Ok(())
    }
}

/// Read and decode every entry in a WAD archive.
pub fn read_wad(bytes: &[u8]) -> Result<Vec<WadEntry>, WadError> {
    if bytes.len() < 4 {
        return Err(WadError::Truncated);
    }
    if bytes[0..2] != MAGIC {
        return Err(WadError::BadMagic);
    }
    let (major, minor) = (bytes[2], bytes[3]);
    if (major, minor) != (VERSION_MAJOR, VERSION_MINOR) {
        return Err(WadError::UnsupportedVersion { major, minor });
    }
    if bytes.len() < HEADER_LEN {
        return Err(WadError::Truncated);
    }

    let checksum = read_u64(bytes, 2 + 2 + SIGNATURE_LEN);
    let count = read_u32(bytes, 2 + 2 + SIGNATURE_LEN + 8) as usize;

    let toc_start = HEADER_LEN;
    let toc_end = toc_start
        .checked_add(count * ENTRY_LEN)
        .ok_or(WadError::Truncated)?;
    if bytes.len() < toc_end {
        return Err(WadError::Truncated);
    }

    let toc = &bytes[toc_start..toc_end];
    if xxh64(toc, 0) != checksum {
        return Err(WadError::ChecksumMismatch);
    }

    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let base = toc_start + index * ENTRY_LEN;
        let path_hash = read_u64(bytes, base);
        let offset = read_u32(bytes, base + 8) as usize;
        let compressed_size = read_u32(bytes, base + 12) as usize;
        let compression = Compression::from_code(bytes[base + 20])?;

        let end = offset.checked_add(compressed_size).ok_or(WadError::Truncated)?;
        if bytes.len() < end {
            return Err(WadError::Truncated);
        }
        let data = decode(&bytes[offset..end], compression)?;
        entries.push(WadEntry {
            path_hash,
            data,
            compression,
        });
    }

    Ok(entries)
}

fn encode(data: &[u8], compression: Compression) -> Result<Vec<u8>, WadError> {
    match compression {
        Compression::Raw => Ok(data.to_vec()),
        Compression::Gzip => {
            let mut encoder = GzEncoder::new(Vec::new(), GzLevel::default());
            encoder.write_all(data)?;
            Ok(encoder.finish()?)
        }
    }
}

fn decode(data: &[u8], compression: Compression) -> Result<Vec<u8>, WadError> {
    match compression {
        Compression::Raw => Ok(data.to_vec()),
        Compression::Gzip => {
            let mut decoder = GzDecoder::new(data);
            let mut out = Vec::new();
            decoder.read_to_end(&mut out)?;
            Ok(out)
        }
    }
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(buf)
}

// ---------------------------------------------------------------------------
// XXH64 (seed-parameterized) — a faithful implementation of the canonical
// algorithm so path hashes match League's WAD keys. No external dependency.
// ---------------------------------------------------------------------------

const PRIME64_1: u64 = 0x9E37_79B1_85EB_CA87;
const PRIME64_2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const PRIME64_3: u64 = 0x1656_67B1_9E37_79F9;
const PRIME64_4: u64 = 0x85EB_CA77_C2B2_AE63;
const PRIME64_5: u64 = 0x27D4_EB2F_1656_67C5;

fn xxh64(input: &[u8], seed: u64) -> u64 {
    let mut index = 0usize;
    let mut hash: u64;

    if input.len() >= 32 {
        let mut v1 = seed.wrapping_add(PRIME64_1).wrapping_add(PRIME64_2);
        let mut v2 = seed.wrapping_add(PRIME64_2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(PRIME64_1);

        while index + 32 <= input.len() {
            v1 = xxh_round(v1, read_u64(input, index));
            v2 = xxh_round(v2, read_u64(input, index + 8));
            v3 = xxh_round(v3, read_u64(input, index + 16));
            v4 = xxh_round(v4, read_u64(input, index + 24));
            index += 32;
        }

        hash = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        hash = xxh_merge(hash, v1);
        hash = xxh_merge(hash, v2);
        hash = xxh_merge(hash, v3);
        hash = xxh_merge(hash, v4);
    } else {
        hash = seed.wrapping_add(PRIME64_5);
    }

    hash = hash.wrapping_add(input.len() as u64);

    while index + 8 <= input.len() {
        let k1 = xxh_round(0, read_u64(input, index));
        hash ^= k1;
        hash = hash.rotate_left(27).wrapping_mul(PRIME64_1).wrapping_add(PRIME64_4);
        index += 8;
    }

    if index + 4 <= input.len() {
        hash ^= (read_u32(input, index) as u64).wrapping_mul(PRIME64_1);
        hash = hash.rotate_left(23).wrapping_mul(PRIME64_2).wrapping_add(PRIME64_3);
        index += 4;
    }

    while index < input.len() {
        hash ^= (input[index] as u64).wrapping_mul(PRIME64_5);
        hash = hash.rotate_left(11).wrapping_mul(PRIME64_1);
        index += 1;
    }

    hash ^= hash >> 33;
    hash = hash.wrapping_mul(PRIME64_2);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(PRIME64_3);
    hash ^= hash >> 32;
    hash
}

fn xxh_round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(PRIME64_2))
        .rotate_left(31)
        .wrapping_mul(PRIME64_1)
}

fn xxh_merge(acc: u64, val: u64) -> u64 {
    (acc ^ xxh_round(0, val))
        .wrapping_mul(PRIME64_1)
        .wrapping_add(PRIME64_4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xxh64_matches_known_vectors() {
        // Canonical XXH64 reference values (seed 0).
        assert_eq!(xxh64(b"", 0), 0xEF46_DB37_51D8_E999);
        // A >32 byte input exercises the main accumulator loop and merge steps.
        let long = b"The quick brown fox jumps over the lazy dog.....";
        assert!(long.len() > 32);
        let once = xxh64(long, 0);
        // Deterministic and seed-sensitive.
        assert_eq!(once, xxh64(long, 0));
        assert_ne!(once, xxh64(long, 1));
    }

    #[test]
    fn path_hash_is_case_insensitive() {
        assert_eq!(
            path_hash("DATA/Menu.wad.client"),
            path_hash("data/menu.wad.client")
        );
        assert_ne!(path_hash("a.bin"), path_hash("b.bin"));
    }

    #[test]
    fn raw_round_trips() {
        let mut builder = WadBuilder::new();
        builder.add("data/one.bin", b"hello".to_vec(), Compression::Raw);
        builder.add("data/two.bin", b"world".to_vec(), Compression::Raw);

        let mut buffer = Vec::new();
        builder.write(&mut buffer).unwrap();

        let entries = read_wad(&buffer).unwrap();
        assert_eq!(entries.len(), 2);

        let one = entries
            .iter()
            .find(|entry| entry.path_hash == path_hash("data/one.bin"))
            .unwrap();
        assert_eq!(one.data, b"hello");
    }

    #[test]
    fn gzip_round_trips_and_shrinks() {
        let payload = vec![b'A'; 4096];
        let mut builder = WadBuilder::new();
        builder.add("data/big.bin", payload.clone(), Compression::Gzip);

        let mut buffer = Vec::new();
        builder.write(&mut buffer).unwrap();
        // Compression should make the archive far smaller than the raw payload.
        assert!(buffer.len() < payload.len());

        let entries = read_wad(&buffer).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].data, payload);
        assert_eq!(entries[0].compression, Compression::Gzip);
    }

    #[test]
    fn rejects_foreign_data() {
        assert!(matches!(read_wad(b"not a wad"), Err(WadError::BadMagic)));
        assert!(matches!(read_wad(&[]), Err(WadError::Truncated)));
    }

    #[test]
    fn detects_toc_corruption() {
        let mut builder = WadBuilder::new();
        builder.add("data/one.bin", b"hello".to_vec(), Compression::Raw);
        let mut buffer = Vec::new();
        builder.write(&mut buffer).unwrap();

        // Flip a byte inside the table of contents (just past the header).
        buffer[HEADER_LEN] ^= 0xFF;
        assert!(matches!(read_wad(&buffer), Err(WadError::ChecksumMismatch)));
    }
}
