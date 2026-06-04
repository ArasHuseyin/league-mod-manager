use std::env;
use std::path::PathBuf;
use std::time::Duration;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: manager-injector.exe <process_name> <path_to_dll>");
        std::process::exit(1);
    }

    let process_name = &args[1];
    let dll_path = PathBuf::from(&args[2]);

    if !dll_path.exists() {
        eprintln!("Error: DLL path does not exist: {}", dll_path.display());
        std::process::exit(1);
    }

    match manager_injector::monitor_and_inject(process_name, &dll_path, Duration::from_millis(500)) {
        Ok(_) => println!("Injection completed successfully."),
        Err(err) => {
            eprintln!("Injection error: {}", err);
            std::process::exit(1);
        }
    }
}
