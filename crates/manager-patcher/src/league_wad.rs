//! Reader for *real* League of Legends `.wad.client` archives (WAD v3.x).
//!
//! This is distinct from the project's own `wad` module (the `RW 3.0` teaching
//! format). Here we parse the actual on-disk layout the game ships so we can read
//! a champion archive, locate entries by path hash, and decode their payloads.
//!
//! ## v3 layout
//! ```text
//! header (272 bytes):
//!   magic        u8[2]   = "RW"
//!   version      u8,u8   = major, minor   (3.x)
//!   signature    u8[256] = ECDSA signature (opaque; copied verbatim)
//!   headerChecksum u64
//!   chunkCount   u32
//! then chunkCount entries (32 bytes each):
//!   pathHash         u64   XXH64 of the lowercased in-WAD path
//!   dataOffset       u32   absolute offset of the payload in the file
//!   compressedSize   u32   payload size on disk
//!   uncompressedSize u32
//!   typeByte         u8    low nibble = compression type, high nibble = subchunk count
//!   isDuplicated     u8
//!   subchunkStart    u16
//!   checksum         u64   XXH3-64 of the on-disk payload
//! ```
//! Compression types: 0 Raw, 1 Gzip, 2 Link, 3 Zstd, 4 ZstdChunked.

use std::collections::HashMap;
use std::io::{Read, Write};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use thiserror::Error;
use xxhash_rust::xxh3::xxh3_64;
use xxhash_rust::xxh64::xxh64;

pub const MAGIC: [u8; 2] = *b"RW";
pub const HEADER_LEN: usize = 2 + 2 + 256 + 8 + 4;
pub const ENTRY_LEN: usize = 32;
const SIGNATURE_LEN: usize = 256;

#[derive(Debug, Error)]
pub enum WadV3Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a WAD archive: bad magic")]
    BadMagic,
    #[error("unsupported WAD major version {0} (expected 3)")]
    UnsupportedVersion(u8),
    #[error("archive is truncated or malformed")]
    Truncated,
    #[error("unknown chunk compression type {0}")]
    UnknownCompression(u8),
    #[error("link/satellite chunks are not supported")]
    UnsupportedLink,
    #[error("zstd decode failed: {0}")]
    Zstd(String),
    #[error("no chunk found for path hash {0:#018x}")]
    ChunkNotFound(u64),
}

/// How a chunk's payload is stored on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkType {
    Raw,
    Gzip,
    Link,
    Zstd,
    ZstdChunked,
}

impl ChunkType {
    fn from_code(code: u8) -> Result<Self, WadV3Error> {
        match code {
            0 => Ok(ChunkType::Raw),
            1 => Ok(ChunkType::Gzip),
            2 => Ok(ChunkType::Link),
            3 => Ok(ChunkType::Zstd),
            4 => Ok(ChunkType::ZstdChunked),
            other => Err(WadV3Error::UnknownCompression(other)),
        }
    }
}

/// A single table-of-contents entry. Keeps the raw `type_byte`, `checksum`, etc.
/// so unmodified chunks can later be copied into a new archive verbatim.
#[derive(Debug, Clone)]
pub struct WadChunk {
    pub path_hash: u64,
    pub data_offset: u32,
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub chunk_type: ChunkType,
    pub subchunk_count: u8,
    pub is_duplicated: bool,
    pub subchunk_start: u16,
    pub checksum: u64,
}

/// Parsed table of contents of a real WAD v3 archive (payloads stay in the
/// caller's byte buffer; decode them on demand with [`decode_chunk`]).
#[derive(Debug, Clone)]
pub struct WadV3 {
    pub version_major: u8,
    pub version_minor: u8,
    pub signature: [u8; SIGNATURE_LEN],
    pub header_checksum: u64,
    pub chunks: Vec<WadChunk>,
}

/// Hash an in-WAD path the way League keys entries: XXH64 of the lowercased path.
pub fn path_hash(path: &str) -> u64 {
    xxh64(path.to_ascii_lowercase().as_bytes(), 0)
}

impl WadV3 {
    /// Parse the header and table of contents from a full archive buffer.
    pub fn parse(bytes: &[u8]) -> Result<Self, WadV3Error> {
        if bytes.len() < HEADER_LEN {
            return Err(WadV3Error::Truncated);
        }
        if bytes[0..2] != MAGIC {
            return Err(WadV3Error::BadMagic);
        }
        let version_major = bytes[2];
        let version_minor = bytes[3];
        if version_major != 3 {
            return Err(WadV3Error::UnsupportedVersion(version_major));
        }

        let mut signature = [0u8; SIGNATURE_LEN];
        signature.copy_from_slice(&bytes[4..4 + SIGNATURE_LEN]);
        let header_checksum = read_u64(bytes, 4 + SIGNATURE_LEN);
        let count = read_u32(bytes, 4 + SIGNATURE_LEN + 8) as usize;

        let toc_end = HEADER_LEN
            .checked_add(count * ENTRY_LEN)
            .ok_or(WadV3Error::Truncated)?;
        if bytes.len() < toc_end {
            return Err(WadV3Error::Truncated);
        }

        let mut chunks = Vec::with_capacity(count);
        for index in 0..count {
            let base = HEADER_LEN + index * ENTRY_LEN;
            let type_byte = bytes[base + 20];
            chunks.push(WadChunk {
                path_hash: read_u64(bytes, base),
                data_offset: read_u32(bytes, base + 8),
                compressed_size: read_u32(bytes, base + 12),
                uncompressed_size: read_u32(bytes, base + 16),
                chunk_type: ChunkType::from_code(type_byte & 0x0F)?,
                subchunk_count: type_byte >> 4,
                is_duplicated: bytes[base + 21] != 0,
                subchunk_start: read_u16(bytes, base + 22),
                checksum: read_u64(bytes, base + 24),
            });
        }

        Ok(WadV3 {
            version_major,
            version_minor,
            signature,
            header_checksum,
            chunks,
        })
    }

    /// Find a chunk by its precomputed path hash.
    pub fn find(&self, path_hash: u64) -> Option<&WadChunk> {
        self.chunks.iter().find(|chunk| chunk.path_hash == path_hash)
    }

    /// Find a chunk by its in-WAD path (hashed with [`path_hash`]).
    pub fn find_path(&self, path: &str) -> Option<&WadChunk> {
        self.find(path_hash(path))
    }
}

/// The on-disk payload slice for a chunk, within the full archive buffer.
pub fn chunk_bytes<'a>(file: &'a [u8], chunk: &WadChunk) -> Result<&'a [u8], WadV3Error> {
    let start = chunk.data_offset as usize;
    let end = start
        .checked_add(chunk.compressed_size as usize)
        .ok_or(WadV3Error::Truncated)?;
    file.get(start..end).ok_or(WadV3Error::Truncated)
}

/// Decode a chunk's payload to its uncompressed bytes.
///
/// `ZstdChunked` payloads are concatenated independent zstd frames; the streaming
/// decoder consumes them all, so they decode the same as plain `Zstd`.
pub fn decode_chunk(file: &[u8], chunk: &WadChunk) -> Result<Vec<u8>, WadV3Error> {
    let raw = chunk_bytes(file, chunk)?;
    match chunk.chunk_type {
        ChunkType::Raw => Ok(raw.to_vec()),
        ChunkType::Gzip => {
            let mut out = Vec::with_capacity(chunk.uncompressed_size as usize);
            GzDecoder::new(raw).read_to_end(&mut out)?;
            Ok(out)
        }
        ChunkType::Zstd | ChunkType::ZstdChunked => {
            zstd::stream::decode_all(raw).map_err(|e| WadV3Error::Zstd(e.to_string()))
        }
        ChunkType::Link => Err(WadV3Error::UnsupportedLink),
    }
}

/// Produce a patched copy of a real WAD by overriding specific chunks with new
/// raw payloads, identified by path hash.
///
/// Strategy: *append-and-repoint*. The whole original archive is kept
/// byte-for-byte — so every untouched chunk, including the subchunk metadata of
/// `ZstdChunked` entries, stays valid — each override's payload is appended at
/// the end of the file as a `Raw` chunk, and only that chunk's 32-byte table
/// entry is rewritten to point at it. The original payload bytes of overridden
/// chunks are left in place as harmless dead space.
///
/// Returns the new archive bytes. Errors if any override targets a path hash the
/// archive does not contain.
pub fn patch_wad(original: &[u8], overrides: &[(u64, &[u8])]) -> Result<Vec<u8>, WadV3Error> {
    let wad = WadV3::parse(original)?;
    let mut out = original.to_vec();
    let mut cursor = out.len() as u64;
    let mut appended: Vec<u8> = Vec::new();

    for (path_hash, content) in overrides {
        let index = wad
            .chunks
            .iter()
            .position(|chunk| chunk.path_hash == *path_hash)
            .ok_or(WadV3Error::ChunkNotFound(*path_hash))?;

        let size = u32::try_from(content.len()).map_err(|_| WadV3Error::Truncated)?;
        let offset = u32::try_from(cursor).map_err(|_| WadV3Error::Truncated)?;

        let base = HEADER_LEN + index * ENTRY_LEN;
        out[base + 8..base + 12].copy_from_slice(&offset.to_le_bytes()); // dataOffset
        out[base + 12..base + 16].copy_from_slice(&size.to_le_bytes()); // compressedSize
        out[base + 16..base + 20].copy_from_slice(&size.to_le_bytes()); // uncompressedSize
        out[base + 20] = 0; // compression type Raw, 0 subchunks
        out[base + 21] = 0; // not duplicated
        out[base + 22..base + 24].copy_from_slice(&0u16.to_le_bytes()); // subchunkStart
        out[base + 24..base + 32].copy_from_slice(&xxh3_64(content).to_le_bytes()); // checksum

        appended.extend_from_slice(content);
        cursor += content.len() as u64;
    }

    out.extend_from_slice(&appended);
    Ok(out)
}

/// Produce a patched copy of a real WAD that both *overrides* existing chunks and
/// *adds* brand-new ones, keyed by path hash.
///
/// This is the superset of [`patch_wad`]: a path hash already present in the
/// archive is overridden in place (append-and-repoint), while a path hash the
/// archive does not contain is appended as a new `Raw` entry with a fresh table
/// slot. Real skin mods routinely ship new files (extra textures, animations a
/// rewritten `.bin` references), so rejecting unknown hashes — as `patch_wad`
/// does — would silently drop those assets and leave the skin broken in game.
///
/// The grown table of contents shifts every original payload right by
/// `additions * ENTRY_LEN`; each untouched entry keeps its bytes verbatim with
/// only its absolute `dataOffset` re-based, so `ZstdChunked` subchunk metadata
/// and every other field stay valid. The header's signature and checksum bytes
/// are preserved verbatim, matching [`patch_wad`].
pub fn patch_or_add_wad(original: &[u8], entries: &[(u64, &[u8])]) -> Result<Vec<u8>, WadV3Error> {
    let wad = WadV3::parse(original)?;
    let original_count = wad.chunks.len();
    let original_toc_end = HEADER_LEN + original_count * ENTRY_LEN;

    // Split requested entries into overrides (hash already present) and additions
    // (hash absent). De-duplicate additions so a hash is only added once.
    let mut overrides: HashMap<u64, &[u8]> = HashMap::new();
    let mut additions: Vec<(u64, &[u8])> = Vec::new();
    for (hash, content) in entries {
        if wad.chunks.iter().any(|chunk| chunk.path_hash == *hash) {
            overrides.insert(*hash, *content);
        } else if !additions.iter().any(|(existing, _)| existing == hash) {
            additions.push((*hash, *content));
        }
    }

    let new_count = original_count + additions.len();
    let new_toc_end = HEADER_LEN + new_count * ENTRY_LEN;
    // How far the table of contents grew; every original payload offset shifts by this.
    let delta = u32::try_from(new_toc_end - original_toc_end).map_err(|_| WadV3Error::Truncated)?;

    let original_payload = &original[original_toc_end..];

    // Header verbatim, then a (zeroed) grown TOC, then the shifted original payload.
    let mut out = Vec::with_capacity(original.len() + delta as usize);
    out.extend_from_slice(&original[..HEADER_LEN]);
    let count_at = 4 + SIGNATURE_LEN + 8;
    out[count_at..count_at + 4].copy_from_slice(&(new_count as u32).to_le_bytes());
    out.resize(new_toc_end, 0);
    out.extend_from_slice(original_payload);

    // The appended region begins right after the shifted original payload. Lay out
    // every overridden and added payload there, Gzip-compressing whatever shrinks,
    // and remember each entry's on-disk layout.
    let mut appended: Vec<u8> = Vec::new();
    let mut cursor = out.len() as u64;
    let mut override_slot: HashMap<u64, EntrySlot> = HashMap::new();
    let mut addition_slot: Vec<(u64, EntrySlot)> = Vec::new();

    for chunk in &wad.chunks {
        if let Some(content) = overrides.get(&chunk.path_hash) {
            let slot = encode_into(&mut appended, &mut cursor, content)?;
            override_slot.insert(chunk.path_hash, slot);
        }
    }
    for (hash, content) in &additions {
        let slot = encode_into(&mut appended, &mut cursor, content)?;
        addition_slot.push((*hash, slot));
    }

    // Rewrite the original entries: overridden ones repoint to the appended payload;
    // untouched ones copy verbatim with their offset re-based by `delta`.
    for (index, chunk) in wad.chunks.iter().enumerate() {
        let base = HEADER_LEN + index * ENTRY_LEN;
        if let Some(slot) = override_slot.get(&chunk.path_hash) {
            write_entry(&mut out, base, chunk.path_hash, slot);
        } else {
            let src = HEADER_LEN + index * ENTRY_LEN;
            out[base..base + ENTRY_LEN].copy_from_slice(&original[src..src + ENTRY_LEN]);
            let rebased = chunk.data_offset.checked_add(delta).ok_or(WadV3Error::Truncated)?;
            out[base + 8..base + 12].copy_from_slice(&rebased.to_le_bytes());
        }
    }
    // Append the new entries' table slots after the original ones.
    for (slot_index, (hash, slot)) in addition_slot.iter().enumerate() {
        let base = HEADER_LEN + (original_count + slot_index) * ENTRY_LEN;
        write_entry(&mut out, base, *hash, slot);
    }

    out.extend_from_slice(&appended);
    Ok(out)
}

/// On-disk layout of one appended payload.
struct EntrySlot {
    offset: u32,
    compressed_size: u32,
    uncompressed_size: u32,
    chunk_type: u8,
    checksum: u64,
}

/// Encode `content` into the appended region (Gzip when it shrinks, else Raw),
/// advance `cursor`, and return the resulting slot. The checksum is XXH3-64 of
/// the on-disk bytes, matching how the format keys chunk integrity.
fn encode_into(appended: &mut Vec<u8>, cursor: &mut u64, content: &[u8]) -> Result<EntrySlot, WadV3Error> {
    let (chunk_type, on_disk) = encode_payload(content);
    let offset = u32::try_from(*cursor).map_err(|_| WadV3Error::Truncated)?;
    let compressed_size = u32::try_from(on_disk.len()).map_err(|_| WadV3Error::Truncated)?;
    let uncompressed_size = u32::try_from(content.len()).map_err(|_| WadV3Error::Truncated)?;
    let checksum = xxh3_64(&on_disk);
    appended.extend_from_slice(&on_disk);
    *cursor += on_disk.len() as u64;
    Ok(EntrySlot {
        offset,
        compressed_size,
        uncompressed_size,
        chunk_type,
        checksum,
    })
}

/// Gzip-compress `content`, returning `(ChunkType::Gzip, bytes)` only when the
/// result is smaller than the input; otherwise store it `(Raw, bytes)`. Both
/// types are ones the game decodes natively.
fn encode_payload(content: &[u8]) -> (u8, Vec<u8>) {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if encoder.write_all(content).is_ok() {
        if let Ok(compressed) = encoder.finish() {
            if compressed.len() < content.len() {
                return (1, compressed); // ChunkType::Gzip
            }
        }
    }
    (0, content.to_vec()) // ChunkType::Raw
}

/// Write a 32-byte table entry for a single-chunk (non-subchunked) payload.
fn write_entry(out: &mut [u8], base: usize, path_hash: u64, slot: &EntrySlot) {
    out[base..base + 8].copy_from_slice(&path_hash.to_le_bytes());
    out[base + 8..base + 12].copy_from_slice(&slot.offset.to_le_bytes()); // dataOffset
    out[base + 12..base + 16].copy_from_slice(&slot.compressed_size.to_le_bytes());
    out[base + 16..base + 20].copy_from_slice(&slot.uncompressed_size.to_le_bytes());
    out[base + 20] = slot.chunk_type; // low nibble = compression, high nibble = 0 subchunks
    out[base + 21] = 0; // not duplicated
    out[base + 22..base + 24].copy_from_slice(&0u16.to_le_bytes()); // subchunkStart
    out[base + 24..base + 32].copy_from_slice(&slot.checksum.to_le_bytes()); // checksum
}

fn read_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn read_u64(bytes: &[u8], at: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_hash_is_lowercased_xxh64() {
        // XXH64("data/test.bin", seed 0) — case-insensitive.
        assert_eq!(path_hash("DATA/Test.bin"), path_hash("data/test.bin"));
        assert_ne!(path_hash("a.bin"), path_hash("b.bin"));
    }

    #[test]
    fn rejects_foreign_and_truncated_data() {
        assert!(matches!(WadV3::parse(b"not a wad"), Err(WadV3Error::Truncated)));
        let mut buf = vec![0u8; HEADER_LEN];
        buf[0] = b'X';
        assert!(matches!(WadV3::parse(&buf), Err(WadV3Error::BadMagic)));
    }

    /// Build a valid Raw-only v3 archive from (path, data) pairs, for tests.
    fn build_v3(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let count = entries.len();
        let toc_end = HEADER_LEN + count * ENTRY_LEN;
        let mut buf = vec![0u8; toc_end];
        buf[0..2].copy_from_slice(&MAGIC);
        buf[2] = 3;
        buf[3] = 4;
        buf[4 + SIGNATURE_LEN + 8..4 + SIGNATURE_LEN + 12]
            .copy_from_slice(&(count as u32).to_le_bytes());

        let mut cursor = toc_end as u64;
        for (i, (path, data)) in entries.iter().enumerate() {
            let base = HEADER_LEN + i * ENTRY_LEN;
            buf[base..base + 8].copy_from_slice(&path_hash(path).to_le_bytes());
            buf[base + 8..base + 12].copy_from_slice(&(cursor as u32).to_le_bytes());
            buf[base + 12..base + 16].copy_from_slice(&(data.len() as u32).to_le_bytes());
            buf[base + 16..base + 20].copy_from_slice(&(data.len() as u32).to_le_bytes());
            buf[base + 20] = 0; // Raw
            buf.extend_from_slice(data);
            cursor += data.len() as u64;
        }
        buf
    }

    #[test]
    fn patch_overrides_one_chunk_and_leaves_others_intact() {
        let original = build_v3(&[
            ("data/keep.bin", b"original-keep"),
            ("data/swap.bin", b"original-swap"),
        ]);

        let new_payload = b"the brand new and longer payload";
        let patched = patch_wad(&original, &[(path_hash("data/swap.bin"), new_payload)]).unwrap();

        let wad = WadV3::parse(&patched).unwrap();
        let swapped = wad.find_path("data/swap.bin").unwrap();
        assert_eq!(swapped.chunk_type, ChunkType::Raw);
        assert_eq!(decode_chunk(&patched, swapped).unwrap(), new_payload);
        assert_eq!(swapped.checksum, xxh3_64(new_payload));

        // The untouched chunk still resolves to its original bytes.
        let kept = wad.find_path("data/keep.bin").unwrap();
        assert_eq!(decode_chunk(&patched, kept).unwrap(), b"original-keep");
    }

    #[test]
    fn patch_errors_on_unknown_path() {
        let original = build_v3(&[("data/only.bin", b"x")]);
        let err = patch_wad(&original, &[(path_hash("data/missing.bin"), b"y")]).unwrap_err();
        assert!(matches!(err, WadV3Error::ChunkNotFound(_)));
    }

    #[test]
    fn patch_or_add_overrides_existing_and_appends_new() {
        let original = build_v3(&[
            ("data/keep.bin", b"original-keep"),
            ("data/swap.bin", b"original-swap"),
        ]);

        let swapped_payload = b"a much longer replacement payload than before";
        let added_payload = b"brand new asset bytes";
        let patched = patch_or_add_wad(
            &original,
            &[
                (path_hash("data/swap.bin"), swapped_payload),
                (path_hash("data/added.bin"), added_payload),
            ],
        )
        .unwrap();

        let wad = WadV3::parse(&patched).unwrap();
        assert_eq!(wad.chunks.len(), 3);

        // Untouched chunk still decodes to its original bytes after the TOC grew.
        let kept = wad.find_path("data/keep.bin").unwrap();
        assert_eq!(decode_chunk(&patched, kept).unwrap(), b"original-keep");

        // Overridden chunk now resolves to the new payload.
        let swapped = wad.find_path("data/swap.bin").unwrap();
        assert_eq!(swapped.chunk_type, ChunkType::Raw);
        assert_eq!(decode_chunk(&patched, swapped).unwrap(), swapped_payload);

        // The brand-new chunk is present and decodes correctly.
        let added = wad.find_path("data/added.bin").unwrap();
        assert_eq!(added.chunk_type, ChunkType::Raw);
        assert_eq!(decode_chunk(&patched, added).unwrap(), added_payload);
        assert_eq!(added.checksum, xxh3_64(added_payload));
    }

    #[test]
    fn patch_or_add_compresses_large_payloads_and_round_trips() {
        let original = build_v3(&[("data/swap.bin", b"x")]);
        // Highly compressible payload, well above the gzip break-even point.
        let payload = vec![b'A'; 8192];
        let patched =
            patch_or_add_wad(&original, &[(path_hash("data/swap.bin"), &payload)]).unwrap();

        let wad = WadV3::parse(&patched).unwrap();
        let chunk = wad.find_path("data/swap.bin").unwrap();
        assert_eq!(chunk.chunk_type, ChunkType::Gzip);
        assert!((chunk.compressed_size as usize) < payload.len());
        assert_eq!(chunk.uncompressed_size as usize, payload.len());
        assert_eq!(decode_chunk(&patched, chunk).unwrap(), payload);
    }

    #[test]
    fn patch_or_add_with_only_overrides_matches_patch_wad_semantics() {
        let original = build_v3(&[("data/a.bin", b"aaaa"), ("data/b.bin", b"bbbb")]);
        let patched = patch_or_add_wad(&original, &[(path_hash("data/b.bin"), b"BBBB")]).unwrap();

        let wad = WadV3::parse(&patched).unwrap();
        assert_eq!(wad.chunks.len(), 2);
        assert_eq!(decode_chunk(&patched, wad.find_path("data/a.bin").unwrap()).unwrap(), b"aaaa");
        assert_eq!(decode_chunk(&patched, wad.find_path("data/b.bin").unwrap()).unwrap(), b"BBBB");
    }

    #[test]
    fn parses_a_minimal_v3_header() {
        // Hand-build a header with one Raw chunk.
        let mut buf = vec![0u8; HEADER_LEN + ENTRY_LEN + 5];
        buf[0..2].copy_from_slice(&MAGIC);
        buf[2] = 3;
        buf[3] = 4;
        buf[4 + SIGNATURE_LEN + 8..4 + SIGNATURE_LEN + 12].copy_from_slice(&1u32.to_le_bytes());
        let base = HEADER_LEN;
        buf[base..base + 8].copy_from_slice(&path_hash("data/x.bin").to_le_bytes());
        buf[base + 8..base + 12].copy_from_slice(&((HEADER_LEN + ENTRY_LEN) as u32).to_le_bytes());
        buf[base + 12..base + 16].copy_from_slice(&5u32.to_le_bytes());
        buf[base + 16..base + 20].copy_from_slice(&5u32.to_le_bytes());
        buf[base + 20] = 0; // Raw, 0 subchunks
        let payload_at = HEADER_LEN + ENTRY_LEN;
        buf[payload_at..payload_at + 5].copy_from_slice(b"hello");

        let wad = WadV3::parse(&buf).unwrap();
        assert_eq!((wad.version_major, wad.version_minor), (3, 4));
        assert_eq!(wad.chunks.len(), 1);
        let chunk = wad.find_path("data/x.bin").unwrap();
        assert_eq!(chunk.chunk_type, ChunkType::Raw);
        assert_eq!(decode_chunk(&buf, chunk).unwrap(), b"hello");
    }
}
