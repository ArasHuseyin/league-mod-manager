//! Offline hook-proof target.
//!
//! Repeatedly opens a real `.wad.client` via `std::fs::read` (which calls
//! `CreateFileW` on Windows) and reports one chunk's storage type, decoded size
//! and XXH3. Run it, then inject the CreateFileW redirect hook into this process:
//! the reported chunk should flip from the original archive's entry to the
//! patched overlay's entry, with no change to this program.
//!
//! Usage:
//!   wad_probe <real.wad.client> <in-wad-path>

use std::thread::sleep;
use std::time::Duration;

use manager_patcher::league_wad::{decode_chunk, path_hash, WadV3};
use xxhash_rust::xxh3::xxh3_64;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: wad_probe <real.wad.client> <in-wad-path>");
        std::process::exit(1);
    }
    let wad_path = &args[0];
    let in_wad = &args[1];
    let hash = path_hash(in_wad);

    println!("wad_probe pid={}", std::process::id());
    println!("  file : {wad_path}");
    println!("  chunk: {in_wad}  (hash {hash:#018x})");
    println!("  (inject the hook now; watch the chunk flip)\n");

    loop {
        match std::fs::read(wad_path) {
            Ok(bytes) => match WadV3::parse(&bytes) {
                Ok(wad) => match wad.find(hash) {
                    Some(chunk) => match decode_chunk(&bytes, chunk) {
                        Ok(data) => println!(
                            "[read] file={:>10} bytes | chunk type={:?} decoded={} xxh3={:#018x}",
                            bytes.len(),
                            chunk.chunk_type,
                            data.len(),
                            xxh3_64(&data)
                        ),
                        Err(e) => println!("[read] decode error: {e}"),
                    },
                    None => println!("[read] chunk not found in archive"),
                },
                Err(e) => println!("[read] parse error: {e}"),
            },
            Err(e) => println!("[read] open error: {e}"),
        }
        sleep(Duration::from_secs(3));
    }
}
