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
        eprintln!("Usage: tn [OPTIONS]");
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --help          Show this help message");
        eprintln!("  --help-config   Show configuration file documentation");
        eprintln!("  --log           View debug log file");
        eprintln!("  --news-log      View news log file");
        eprintln!("  --clear-cache   Delete all cache files");
        eprintln!("  --offline       Browse cached articles only");
        eprintln!("  --config=PATH   Use a custom config file");
        return;
    }

    if args.iter().any(|a| a == "--help-config") {
        print_help_config();
        return;
    }

    if args.iter().any(|a| a == "--log") {
        let path = log::debug_log_path();
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

    if args.iter().any(|a| a == "--news-log") {
        let path = log::news_log_path();
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
        log::info("tn starting");
        log::news("tn session started");
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

    let (cmd_tx, resp_rx) = backend::spawn(&config, cache.clone());
    if let Err(e) = tui::run(&config, &cache, &cmd_tx, &resp_rx, offline) {
        log::error(format!("tui error: {}", e));
        eprintln!("TUI error: {}", e);
    }

    let _ = cmd_tx.send(backend::BackendCommand::Shutdown);
    log::info("tn shutdown");
    log::news("tn session ended");
}

fn print_help_config() {
    let xdg = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap_or_default();
        format!("{}/.config", home)
    });
    eprintln!("Configuration file: {}/tn/config.toml", xdg);
    eprintln!();
    eprintln!("[ui]");
    eprintln!("  page_size = 100              # max articles per page (default: 100)");
    eprintln!("  mouse = true                 # enable mouse support (default: true)");
    eprintln!("  sync_interval_secs = 300     # auto-refresh interval in seconds (default: 300)");
    eprintln!(
        "  browser = \"command {{url}}\"     # browser command; {{url}} is replaced with the URL"
    );
    eprintln!("                               # if no {{url}}, URL is appended as argument");
    eprintln!(
        "                               # executed via sh -c; falls back to $BROWSER, xdg-open"
    );
    eprintln!();
    eprintln!("[theme]");
    eprintln!("  bg = \"#002b36\"               # background color (#RRGGBB)");
    eprintln!("  fg = \"#839496\"               # foreground color");
    eprintln!("  bold_fg = \"#93a1a1\"           # bold/unread text color");
    eprintln!("  selection_bg = \"#073642\"      # selected row background");
    eprintln!("  selection_fg = \"#eee8d5\"      # selected row foreground");
    eprintln!("  status_bg = \"#586e75\"         # status bar background");
    eprintln!("  status_fg = \"#eee8d5\"         # status bar foreground");
    eprintln!("  header_fg = \"#268bd2\"         # header text color");
    eprintln!();
    eprintln!("[[feed]]");
    eprintln!("  name = \"Feed Name\"            # display name for the feed");
    eprintln!("  url = \"https://example/rss\"   # RSS or Atom feed URL");
}
