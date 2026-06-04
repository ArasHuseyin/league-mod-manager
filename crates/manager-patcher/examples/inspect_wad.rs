//! Proof harness for the real WAD v3 reader.
//!
//! Usage:
//!   cargo run -p manager-patcher --example inspect_wad -- <real.wad.client> [in-wad-path ...]
//!
//! Prints the archive header, a compression-type tally, and — for each in-WAD
//! path given — resolves it by hash, decodes the payload, and verifies the
//! decoded length matches the recorded uncompressed size.

use std::collections::BTreeMap;
use std::process::ExitCode;

use manager_patcher::league_wad::{decode_chunk, path_hash, ChunkType, WadV3};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(wad_path) = args.next() else {
        eprintln!("usage: inspect_wad <real.wad.client> [in-wad-path ...]");
        return ExitCode::FAILURE;
    };
    let lookups: Vec<String> = args.collect();

    let bytes = match std::fs::read(&wad_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to read {wad_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("file   : {wad_path} ({} bytes)", bytes.len());

    let wad = match WadV3::parse(&bytes) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("parse error: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("version: {}.{}", wad.version_major, wad.version_minor);
    println!("chunks : {}", wad.chunks.len());

    let mut tally: BTreeMap<&str, usize> = BTreeMap::new();
    for chunk in &wad.chunks {
        let name = match chunk.chunk_type {
            ChunkType::Raw => "raw",
            ChunkType::Gzip => "gzip",
            ChunkType::Link => "link",
            ChunkType::Zstd => "zstd",
            ChunkType::ZstdChunked => "zstd-chunked",
        };
        *tally.entry(name).or_default() += 1;
    }
    println!("types  : {tally:?}");

    let mut failures = 0;
    for path in &lookups {
        let hash = path_hash(path);
        print!("\nlookup : {path}\n  hash : {hash:#018x}  ");
        match wad.find(hash) {
            Some(chunk) => {
                println!(
                    "FOUND  type={:?} comp={} uncomp={}",
                    chunk.chunk_type, chunk.compressed_size, chunk.uncompressed_size
                );
                match decode_chunk(&bytes, chunk) {
                    Ok(data) => {
                        let ok = data.len() as u32 == chunk.uncompressed_size;
                        println!(
                            "  decode: {} bytes ({})",
                            data.len(),
                            if ok { "OK — matches uncompressedSize" } else { "MISMATCH!" }
                        );
                        if !ok {
                            failures += 1;
                        }
                    }
                    Err(e) => {
                        println!("  decode: ERROR {e}");
                        failures += 1;
                    }
                }
            }
            None => {
                println!("NOT FOUND");
                failures += 1;
            }
        }
    }

    if failures > 0 {
        eprintln!("\n{failures} lookup(s) failed");
        ExitCode::FAILURE
    } else {
        println!("\nall lookups ok");
        ExitCode::SUCCESS
    }
}
