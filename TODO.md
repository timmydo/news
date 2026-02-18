# TODO

## Core
- [ ] Implement RSS/Atom XML feed parser in `feed.rs`
- [ ] Wire up backend thread to TUI in `main.rs`
- [ ] Implement periodic background feed refresh (sync_interval_secs)

## TUI
- [ ] Raw terminal setup and ANSI drawing (`tui/screen.rs`)
- [ ] Key and mouse input parser (`tui/input.rs`)
- [ ] Event loop and terminal management (`tui/mod.rs`)
- [ ] Feed list view with unread counts
- [ ] Article list view with search
- [ ] Article view with plain text rendering (html2text)
- [ ] Help view with keybinding reference
- [ ] Mouse support (click, wheel scrolling)

## Features
- [ ] Open article link in `$BROWSER`
- [ ] Mark read/unread per article and per feed
- [ ] Offline mode (browse cached articles only)
- [ ] Logging infrastructure (`--log`)

## Future
- [ ] Generate summary/digest file from cached articles (similar to `rss_digest.py` but reading from redb and producing a standalone HTML or text summary)
- [ ] CLI mode (NDJSON protocol, like tmc's `--cli`)
- [ ] Feed-specific refresh intervals
- [ ] OPML import/export
