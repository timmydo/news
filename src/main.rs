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

    let offline = args.iter().any(|a| a == "--offline");

    let (cmd_tx, resp_rx) = backend::spawn(&config);

    if !offline {
        cmd_tx.send(backend::BackendCommand::FetchAllFeeds).unwrap();
    }

    // Process backend responses until all feeds are fetched
    let feed_count = config.feeds.len();
    let mut done = 0;
    while done < feed_count {
        match resp_rx.recv() {
            Ok(backend::BackendResponse::FeedArticles {
                feed_name,
                total,
                unread,
                ..
            }) => {
                eprintln!("{}: {} articles ({} unread)", feed_name, total, unread);
                done += 1;
            }
            Ok(backend::BackendResponse::FetchError { feed_url, error }) => {
                eprintln!("Error fetching {}: {}", feed_url, error);
                done += 1;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }

    // TODO: start TUI event loop here; for now just exit
    eprintln!("TUI not yet implemented. Feeds fetched successfully.");

    let _ = cmd_tx.send(backend::BackendCommand::Shutdown);
}
