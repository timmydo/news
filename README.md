# Timmy's News

`tn` (Timmy's News) is a Rust terminal news reader for RSS/Atom feeds. It fetches feeds,
caches articles in `redb`, and provides a keyboard/mouse-first TUI.

## Build / Run

This project needs `CC=gcc` (the `ring` crate needs a C compiler and `cc` is
not on `PATH` in this environment):

```bash
CC=gcc cargo build
CC=gcc cargo run
CC=gcc cargo test
CC=gcc cargo clippy
cargo fmt -- --check
```

## Configuration

Default config path:

- `$XDG_CONFIG_HOME/news/config.toml`
- or `~/.config/news/config.toml`

Example:

```toml
[ui]
page_size = 100
mouse = true
sync_interval_secs = 300

[theme]
bg = "#002b36"
fg = "#839496"
bold_fg = "#93a1a1"
selection_bg = "#073642"
selection_fg = "#eee8d5"
status_bg = "#586e75"
status_fg = "#eee8d5"
header_fg = "#268bd2"

[[feed]]
name = "Hacker News"
url = "https://news.ycombinator.com/rss"
```

## Usage

- CLI executable: `tn`

- Main feed list includes virtual views:
- `[All]` for all feeds combined
- `[Unread]` for unread-only combined
- Articles are sorted newest-first by date.
- Datetimes are normalized to local timezone and displayed as:
- `YYYY-MM-DD HH:MM:SS` (no timezone offset)

### Keybindings

- Global: `?` help, `q` back/quit, `g` refresh
- Feed list: `j/k/n/p` or arrows, `Enter`, `u`
- Article list: `j/k/n/p` or arrows, `PgUp/PgDn`, `Home/End`, `Enter`, `u`, `o`, `/`
- Article view: `j/k` or arrows, `Space`, `PgUp/PgDn`, `n/p`, `u`, `o`
- Mouse:
- Feed list click selects feed
- Article list click opens article
- Mouse wheel scrolls article list and article view

## Cache

- Database: `~/.cache/news/news.redb`
