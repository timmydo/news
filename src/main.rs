mod backend;
mod cache;
mod config;
mod feed;
mod log;

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

    let config_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--config="))
        .map(|s| s.to_string());

    let config = match config::Config::load(config_path.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error loading config: {}", e);
            std::process::exit(1);
        }
    };

    if args.iter().any(|a| a == "--clear-cache") {
        cache::Cache::clear();
        eprintln!("Cache cleared.");
        return;
    }

    let _offline = args.iter().any(|a| a == "--offline");

    eprintln!("Loaded {} feeds from config.", config.feeds.len());
    for f in &config.feeds {
        eprintln!("  - {} ({})", f.name, f.url);
    }

    // TODO: spawn backend, start TUI
    eprintln!("TUI not yet implemented.");
}
