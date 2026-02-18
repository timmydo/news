mod backend;
mod cache;
mod config;
mod feed;
mod keybindings;
mod log;
mod tui;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        eprintln!("Usage: news [OPTIONS]");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --help          Show this help message");
        eprintln!("  --log           View log file");
        eprintln!("  --clear-cache   Delete all cache files");
        eprintln!("  --offline       Browse cached articles only");
        eprintln!("  --config=PATH   Use a custom config file");
        return;
    }

    if args.iter().any(|a| a == "--log") {
        let path = log::log_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                print!("{}", contents);
            }
            Err(e) => {
                eprintln!("No log at {} ({})", path.display(), e);
            }
        }
        return;
    }

    if let Err(e) = log::init() {
        eprintln!("Failed to initialize logging: {}", e);
    } else {
        log::info("news starting");
    }

    let config_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--config="))
        .map(|s| s.to_string());

    let config = match config::Config::load(config_path.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            log::error(format!("config load failed: {}", e));
            eprintln!("Error loading config: {}", e);
            std::process::exit(1);
        }
    };

    if args.iter().any(|a| a == "--clear-cache") {
        cache::Cache::clear();
        log::info("cache cleared");
        eprintln!("Cache cleared.");
        return;
    }

    let offline = args.iter().any(|a| a == "--offline");
    let cache = match cache::Cache::open() {
        Ok(c) => c,
        Err(e) => {
            log::error(format!("failed to open cache: {}", e));
            eprintln!("Failed to open cache: {}", e);
            std::process::exit(1);
        }
    };

    let (cmd_tx, resp_rx) = backend::spawn(&config);
    if let Err(e) = tui::run(&config, &cache, &cmd_tx, &resp_rx, offline) {
        log::error(format!("tui error: {}", e));
        eprintln!("TUI error: {}", e);
    }

    let _ = cmd_tx.send(backend::BackendCommand::Shutdown);
    log::info("news shutdown");
}
