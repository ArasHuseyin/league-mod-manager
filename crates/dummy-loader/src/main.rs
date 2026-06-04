use std::fs;
use std::thread;
use std::time::Duration;
use std::process;

fn main() {
    let pid = process::id();
    println!("=== Dummy Loader Running ===");
    println!("Process ID (PID): {}", pid);
    println!("Reading 'test.txt' every 2 seconds...");
    println!("Please inject 'manager_hook_dll.dll' into this process.");
    println!("============================\n");

    loop {
        match fs::read_to_string("test.txt") {
            Ok(content) => {
                println!("[Reading test.txt] Content: '{}'", content.trim());
            }
            Err(e) => {
                println!("[Reading test.txt] Error reading file: {}", e);
            }
        }
        thread::sleep(Duration::from_secs(2));
    }
}
