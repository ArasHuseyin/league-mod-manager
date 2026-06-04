//! Proof harness for the real WAD merge.
//!
//! Usage:
//!   cargo run -p manager-patcher --example patch_wad -- \
//!       <original.wad.client> <output.wad.client> \
//!       <in-wad-path> <replacement-file> [<in-wad-path> <replacement-file> ...]
//!
//! Reads the original archive, overrides each given in-WAD path with the bytes of
//! the corresponding replacement file, writes the patched archive, then re-parses
//! the output and verifies every overridden chunk now decodes to the new bytes.

use std::process::ExitCode;

use manager_patcher::league_wad::{decode_chunk, patch_wad, path_hash, WadV3};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 || (args.len() - 2) % 2 != 0 {
        eprintln!(
            "usage: patch_wad <original> <output> <in-wad-path> <replacement-file> [...pairs]"
        );
        return ExitCode::FAILURE;
    }

    let original_path = &args[0];
    let output_path = &args[1];

    // Collect (in-wad-path, replacement-bytes) pairs.
    let mut replacements: Vec<(String, Vec<u8>)> = Vec::new();
    for pair in args[2..].chunks(2) {
        let in_wad = pair[0].clone();
        match std::fs::read(&pair[1]) {
            Ok(bytes) => replacements.push((in_wad, bytes)),
            Err(e) => {
                eprintln!("failed to read replacement {}: {e}", pair[1]);
                return ExitCode::FAILURE;
            }
        }
    }

    let original = match std::fs::read(original_path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("failed to read {original_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("original: {original_path} ({} bytes)", original.len());

    let overrides: Vec<(u64, &[u8])> = replacements
        .iter()
        .map(|(path, bytes)| {
            println!("  override {path}  ({} bytes)  hash={:#018x}", bytes.len(), path_hash(path));
            (path_hash(path), bytes.as_slice())
        })
        .collect();

    let patched = match patch_wad(&original, &overrides) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("patch failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = std::fs::write(output_path, &patched) {
        eprintln!("failed to write {output_path}: {e}");
        return ExitCode::FAILURE;
    }
    println!("patched : {output_path} ({} bytes, +{})", patched.len(), patched.len() - original.len());

    // Verify: re-parse the output and confirm each override decodes to its new bytes.
    let wad = match WadV3::parse(&patched) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("re-parse failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut failures = 0;
    for (path, bytes) in &replacements {
        match wad.find(path_hash(path)) {
            Some(chunk) => {
                let decoded = decode_chunk(&patched, chunk).unwrap_or_default();
                let ok = &decoded == bytes;
                println!(
                    "verify  {path}: {} (decoded {} bytes)",
                    if ok { "OK" } else { "MISMATCH" },
                    decoded.len()
                );
                if !ok {
                    failures += 1;
                }
            }
            None => {
                println!("verify  {path}: NOT FOUND");
                failures += 1;
            }
        }
    }

    if failures > 0 {
        eprintln!("\n{failures} verification(s) failed");
        ExitCode::FAILURE
    } else {
        println!("\npatched WAD verified — overridden chunks now carry the mod bytes");
        ExitCode::SUCCESS
    }
}
