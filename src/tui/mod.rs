mod input;
mod screen;
pub mod views;

use std::sync::mpsc;
use std::time::Duration;

use crate::backend::{BackendCommand, BackendResponse};
use crate::cache::Cache;
use crate::config::Config;
use crate::feed::{datetime_sort_key, normalize_datetime_to_local, Article};
use crate::keybindings;

use input::{InputEvent, Key, MouseEvent};
use screen::Terminal;

#[derive(Clone)]
struct FeedRow {
    name: String,
    url: String,
    total: usize,
    unread: usize,
    last_updated: Option<String>,
    last_error: Option<String>,
}

enum View {
    FeedList,
    ArticleList,
    ArticleView,
    Log,
    Help,
}

#[derive(Clone)]
enum FeedScope {
    All,
    Unread,
    Feed(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LogTab {
    News,
    Debug,
}

#[derive(Clone, Copy, Default)]
struct UiTheme {
    selection_bg: Option<(u8, u8, u8)>,
    selection_fg: Option<(u8, u8, u8)>,
    status_bg: Option<(u8, u8, u8)>,
    status_fg: Option<(u8, u8, u8)>,
    header_fg: Option<(u8, u8, u8)>,
    bold_fg: Option<(u8, u8, u8)>,
}

pub fn run(
    config: &Config,
    cache: &Cache,
    cmd_tx: &mpsc::Sender<BackendCommand>,
    resp_rx: &mpsc::Receiver<BackendResponse>,
    offline: bool,
) -> Result<(), String> {
    let terminal = Terminal::enter(config.ui.mouse, &config.theme)?;
    let (input_tx, input_rx) = mpsc::channel::<InputEvent>();
    input::spawn_input_thread(input_tx);

    let mut app = App::new(config, cache, offline);
    if !offline {
        let _ = cmd_tx.send(BackendCommand::FetchAllFeeds);
    }

    loop {
        while let Ok(resp) = resp_rx.try_recv() {
            app.handle_backend(resp, cache);
        }

        app.draw(&terminal)?;

        match input_rx.recv_timeout(Duration::from_millis(120)) {
            Ok(input) => {
                if app.handle_input(input, cache, cmd_tx) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

struct App {
    view: View,
    feeds: Vec<FeedRow>,
    selected_feed: usize,
    selected_article: usize,
    article_scroll: usize,
    selected_feed_scope: Option<FeedScope>,
    articles: Vec<Article>,
    search: String,
    search_mode: bool,
    status: String,
    last_updated: Option<String>,
    last_size: (usize, usize),
    page_size: usize,
    theme: UiTheme,
    log_tab: LogTab,
    log_scroll: usize,
    quitting: bool,
    pending_redraw: bool,
}

impl UiTheme {
    fn from_config(theme: &crate::config::Theme) -> Self {
        Self {
            selection_bg: parse_color_opt(&theme.selection_bg),
            selection_fg: parse_color_opt(&theme.selection_fg),
            status_bg: parse_color_opt(&theme.status_bg),
            status_fg: parse_color_opt(&theme.status_fg),
            header_fg: parse_color_opt(&theme.header_fg),
            bold_fg: parse_color_opt(&theme.bold_fg),
        }
    }
}

impl App {
    fn new(config: &Config, cache: &Cache, offline: bool) -> Self {
        let mut feeds = Vec::with_capacity(config.feeds.len());
        let mut last_updated = None;
        let mut last_updated_ts = None;
        for feed in &config.feeds {
            let hashes = cache.get_feed_index(&feed.url).unwrap_or_default();
            let unread = hashes
                .iter()
                .filter(|hash| cache.get_article(hash).map(|a| !a.read).unwrap_or(false))
                .count();
            if let Some(meta) = cache.get_feed_meta(&feed.url) {
                if let Some(ts) = datetime_sort_key(Some(&meta.last_fetched)) {
                    if last_updated_ts.map(|current| ts > current).unwrap_or(true) {
                        last_updated_ts = Some(ts);
                        last_updated = normalize_datetime_to_local(&meta.last_fetched);
                    }
                }
            }
            feeds.push(FeedRow {
                name: feed.name.clone(),
                url: feed.url.clone(),
                total: hashes.len(),
                unread,
                last_updated: cache
                    .get_feed_meta(&feed.url)
                    .and_then(|m| normalize_datetime_to_local(&m.last_fetched)),
                last_error: None,
            });
        }

        Self {
            view: View::FeedList,
            feeds,
            selected_feed: 0,
            selected_article: 0,
            article_scroll: 0,
            selected_feed_scope: None,
            articles: Vec::new(),
            search: String::new(),
            search_mode: false,
            status: if offline {
                "Offline mode: browsing cache".to_string()
            } else {
                "Ready".to_string()
            },
            last_updated,
            last_size: (80, 24),
            page_size: config.ui.page_size.max(1),
            theme: UiTheme::from_config(&config.theme),
            log_tab: LogTab::News,
            log_scroll: 0,
            quitting: false,
            pending_redraw: true,
        }
    }

    fn handle_backend(&mut self, msg: BackendResponse, cache: &Cache) {
        match msg {
            BackendResponse::FeedArticles {
                feed_url,
                feed_name,
                fetched_at,
                total,
                unread,
                articles,
            } => {
                if let Some(row) = self.feeds.iter_mut().find(|f| f.url == feed_url) {
                    row.total = total;
                    row.unread = unread;
                    row.last_updated = normalize_datetime_to_local(&fetched_at);
                    row.last_error = None;
                }
                if matches!(
                    self.selected_feed_scope.as_ref(),
                    Some(FeedScope::Feed(url)) if url == &feed_url
                ) || matches!(
                    self.selected_feed_scope.as_ref(),
                    Some(FeedScope::All | FeedScope::Unread)
                ) {
                    self.reload_articles(cache);
                }
                self.last_updated = Some(fetched_at);
                self.status = if articles.is_empty() {
                    format!("{}: no new items", feed_name)
                } else {
                    format!(
                        "{}: {} new item(s), {} total ({} unread)",
                        feed_name,
                        articles.len(),
                        total,
                        unread
                    )
                };
                self.pending_redraw = true;
            }
            BackendResponse::FetchError { feed_url, error } => {
                if let Some(row) = self.feeds.iter_mut().find(|f| f.url == feed_url) {
                    row.last_error = Some(error.clone());
                }
                self.status = format!("Fetch error: {}", error);
                self.pending_redraw = true;
            }
            BackendResponse::ArticleMutation { hash, read } => {
                if let Some(article) = self.articles.iter_mut().find(|a| a.hash == hash) {
                    article.read = read;
                }
                self.recount_current_feed_unread();
                self.pending_redraw = true;
            }
            BackendResponse::FeedMarkedRead { feed_url } => {
                if let Some(row) = self.feeds.iter_mut().find(|f| f.url == feed_url) {
                    row.unread = 0;
                }
                if let Some(scope) = self.selected_feed_scope.as_ref() {
                    match scope {
                        FeedScope::Feed(url) if url == &feed_url => {
                            for article in &mut self.articles {
                                article.read = true;
                            }
                        }
                        FeedScope::All | FeedScope::Unread => self.reload_articles(cache),
                        _ => {}
                    }
                }
                self.status = "Marked feed as read".to_string();
                self.pending_redraw = true;
            }
        }
    }

    fn handle_input(
        &mut self,
        input: InputEvent,
        cache: &Cache,
        cmd_tx: &mpsc::Sender<BackendCommand>,
    ) -> bool {
        if self.quitting {
            return true;
        }

        if let InputEvent::Key(Key::Char('?')) = input {
            if !matches!(self.view, View::Help) {
                self.view = View::Help;
                self.pending_redraw = true;
                return false;
            }
        }

        if matches!(self.view, View::Help) {
            if let InputEvent::Key(Key::Char('q')) = input {
                self.view = if self.selected_feed_scope.is_some() {
                    View::ArticleList
                } else {
                    View::FeedList
                };
                self.pending_redraw = true;
            }
            return false;
        }

        if self.search_mode {
            if let InputEvent::Key(key) = input {
                match key {
                    Key::Enter => {
                        self.search_mode = false;
                        self.selected_article = 0;
                        self.pending_redraw = true;
                    }
                    Key::Backspace => {
                        self.search.pop();
                        self.selected_article = 0;
                        self.pending_redraw = true;
                    }
                    Key::Char(c) if !c.is_control() => {
                        self.search.push(c);
                        self.selected_article = 0;
                        self.pending_redraw = true;
                    }
                    _ => {}
                }
            }
            return false;
        }

        match self.view {
            View::FeedList => self.handle_feed_keys(input, cache, cmd_tx),
            View::ArticleList => self.handle_article_list_keys(input, cache, cmd_tx),
            View::ArticleView => self.handle_article_view_keys(input, cache, cmd_tx),
            View::Log => self.handle_log_keys(input),
            View::Help => {}
        }

        self.quitting
    }

    fn handle_feed_keys(
        &mut self,
        input: InputEvent,
        cache: &Cache,
        cmd_tx: &mpsc::Sender<BackendCommand>,
    ) {
        match input {
            InputEvent::Key(Key::Char('q')) => {
                self.quitting = true;
            }
            InputEvent::Key(Key::Down)
            | InputEvent::Key(Key::Char('j'))
            | InputEvent::Key(Key::Char('n')) => {
                if self.selected_feed + 1 < self.feed_row_count() {
                    self.selected_feed += 1;
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::Up)
            | InputEvent::Key(Key::Char('k'))
            | InputEvent::Key(Key::Char('p')) => {
                if self.selected_feed > 0 {
                    self.selected_feed -= 1;
                    self.pending_redraw = true;
                }
            }
            InputEvent::Mouse(MouseEvent::LeftClick { row }) => {
                if row >= 2 {
                    let list_rows = self.feed_list_rows();
                    let start = self
                        .selected_feed
                        .saturating_sub(list_rows.saturating_sub(1));
                    let idx = start + (row - 2);
                    if idx < self.feed_row_count() {
                        self.selected_feed = idx;
                        self.pending_redraw = true;
                    }
                }
            }
            InputEvent::Key(Key::Enter) => {
                if self.selected_feed == self.log_row_index() {
                    self.view = View::Log;
                    self.log_scroll = 0;
                } else {
                    self.selected_feed_scope = Some(self.scope_for_selected_feed());
                    self.selected_article = 0;
                    self.article_scroll = 0;
                    self.search.clear();
                    self.reload_articles(cache);
                    self.view = View::ArticleList;
                }
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Char('g')) => {
                let _ = cmd_tx.send(BackendCommand::FetchAllFeeds);
                self.status = "Refreshing feeds...".to_string();
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Char('u')) => {
                if self.selected_feed != self.log_row_index() {
                    match self.scope_for_selected_feed() {
                        FeedScope::Feed(url) => {
                            if let Some(feed) = self.feeds.iter().find(|f| f.url == url) {
                                let _ = cmd_tx.send(BackendCommand::MarkFeedRead {
                                    feed_url: feed.url.clone(),
                                });
                                self.status = format!("Marking {} read...", feed.name);
                                self.pending_redraw = true;
                            }
                        }
                        FeedScope::All | FeedScope::Unread => {
                            for feed in &self.feeds {
                                let _ = cmd_tx.send(BackendCommand::MarkFeedRead {
                                    feed_url: feed.url.clone(),
                                });
                            }
                            self.status = "Marking all feeds read...".to_string();
                            self.pending_redraw = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn handle_article_list_keys(
        &mut self,
        input: InputEvent,
        cache: &Cache,
        cmd_tx: &mpsc::Sender<BackendCommand>,
    ) {
        match input {
            InputEvent::Key(Key::Char('q')) => {
                self.view = View::FeedList;
                self.selected_feed_scope = None;
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Down)
            | InputEvent::Key(Key::Char('j'))
            | InputEvent::Key(Key::Char('n')) => {
                let visible = self.filtered_article_indices();
                if self.selected_article + 1 < visible.len() {
                    self.selected_article += 1;
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::Up)
            | InputEvent::Key(Key::Char('k'))
            | InputEvent::Key(Key::Char('p')) => {
                if self.selected_article > 0 {
                    self.selected_article -= 1;
                    self.pending_redraw = true;
                }
            }
            InputEvent::Mouse(MouseEvent::LeftClick { row }) => {
                if row >= 3 {
                    let list_rows = self.article_list_rows();
                    let start = self
                        .selected_article
                        .saturating_sub(list_rows.saturating_sub(1));
                    let idx = start + (row - 3);
                    let visible = self.filtered_article_indices();
                    if idx < visible.len() {
                        self.selected_article = idx;
                        self.article_scroll = 0;
                        self.view = View::ArticleView;
                        self.mark_current_article_read(cmd_tx);
                        self.pending_redraw = true;
                    }
                }
            }
            InputEvent::Mouse(MouseEvent::ScrollDown) => {
                let visible = self.filtered_article_indices();
                if self.selected_article + 1 < visible.len() {
                    self.selected_article += 1;
                    self.pending_redraw = true;
                }
            }
            InputEvent::Mouse(MouseEvent::ScrollUp) => {
                if self.selected_article > 0 {
                    self.selected_article -= 1;
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::Enter) => {
                if !self.filtered_article_indices().is_empty() {
                    self.article_scroll = 0;
                    self.view = View::ArticleView;
                    self.mark_current_article_read(cmd_tx);
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::PageDown) => {
                let visible = self.filtered_article_indices();
                if !visible.is_empty() {
                    let step = self.article_list_rows().max(1);
                    self.selected_article = (self.selected_article + step).min(visible.len() - 1);
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::PageUp) => {
                let visible = self.filtered_article_indices();
                if !visible.is_empty() {
                    let step = self.article_list_rows().max(1);
                    self.selected_article = self.selected_article.saturating_sub(step);
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::Home) => {
                if !self.filtered_article_indices().is_empty() {
                    self.selected_article = 0;
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::End) => {
                let visible = self.filtered_article_indices();
                if !visible.is_empty() {
                    self.selected_article = visible.len() - 1;
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::Char('/')) => {
                self.search_mode = true;
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Char('g')) => {
                if let Some(scope) = self.selected_feed_scope.as_ref() {
                    match scope {
                        FeedScope::Feed(url) => {
                            let _ = cmd_tx.send(BackendCommand::FetchFeed { url: url.clone() });
                            self.status = "Refreshing feed...".to_string();
                        }
                        FeedScope::All | FeedScope::Unread => {
                            let _ = cmd_tx.send(BackendCommand::FetchAllFeeds);
                            self.status = "Refreshing feeds...".to_string();
                        }
                    }
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::Char('u')) => self.toggle_current_read(cache, cmd_tx),
            InputEvent::Key(Key::Char('o')) => self.open_current_article(),
            _ => {}
        }
    }

    fn handle_article_view_keys(
        &mut self,
        input: InputEvent,
        cache: &Cache,
        cmd_tx: &mpsc::Sender<BackendCommand>,
    ) {
        match input {
            InputEvent::Key(Key::Char('q')) => {
                self.view = View::ArticleList;
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Down) | InputEvent::Key(Key::Char('j')) => {
                self.article_scroll = self.article_scroll.saturating_add(1);
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Up) | InputEvent::Key(Key::Char('k')) => {
                self.article_scroll = self.article_scroll.saturating_sub(1);
                self.pending_redraw = true;
            }
            InputEvent::Mouse(MouseEvent::ScrollDown)
            | InputEvent::Key(Key::PageDown)
            | InputEvent::Key(Key::Char(' ')) => {
                self.article_scroll = self.article_scroll.saturating_add(15);
                self.pending_redraw = true;
            }
            InputEvent::Mouse(MouseEvent::ScrollUp) | InputEvent::Key(Key::PageUp) => {
                self.article_scroll = self.article_scroll.saturating_sub(15);
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Char('n')) => {
                let visible = self.filtered_article_indices();
                if self.selected_article + 1 < visible.len() {
                    self.selected_article += 1;
                    self.article_scroll = 0;
                    self.mark_current_article_read(cmd_tx);
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::Char('p')) => {
                if self.selected_article > 0 {
                    self.selected_article -= 1;
                    self.article_scroll = 0;
                    self.mark_current_article_read(cmd_tx);
                    self.pending_redraw = true;
                }
            }
            InputEvent::Key(Key::Char('u')) => self.toggle_current_read(cache, cmd_tx),
            InputEvent::Key(Key::Char('o')) => self.open_current_article(),
            _ => {}
        }
    }

    fn handle_log_keys(&mut self, input: InputEvent) {
        match input {
            InputEvent::Key(Key::Char('q')) => {
                self.view = View::FeedList;
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Down) | InputEvent::Key(Key::Char('j')) => {
                self.log_scroll = self.log_scroll.saturating_add(1);
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Up) | InputEvent::Key(Key::Char('k')) => {
                self.log_scroll = self.log_scroll.saturating_sub(1);
                self.pending_redraw = true;
            }
            InputEvent::Mouse(MouseEvent::ScrollDown) | InputEvent::Key(Key::PageDown) => {
                self.log_scroll = self.log_scroll.saturating_add(15);
                self.pending_redraw = true;
            }
            InputEvent::Mouse(MouseEvent::ScrollUp) | InputEvent::Key(Key::PageUp) => {
                self.log_scroll = self.log_scroll.saturating_sub(15);
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Home) => {
                self.log_scroll = 0;
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::End) => {
                self.log_scroll = usize::MAX;
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Char('n')) => {
                self.log_tab = LogTab::News;
                self.log_scroll = 0;
                self.pending_redraw = true;
            }
            InputEvent::Key(Key::Char('d')) => {
                self.log_tab = LogTab::Debug;
                self.log_scroll = 0;
                self.pending_redraw = true;
            }
            _ => {}
        }
    }

    fn toggle_current_read(&mut self, _cache: &Cache, cmd_tx: &mpsc::Sender<BackendCommand>) {
        if let Some(article) = self.current_article().cloned() {
            let _ = cmd_tx.send(BackendCommand::MarkRead {
                hash: article.hash.clone(),
                read: !article.read,
            });
            if let Some(local) = self.articles.iter_mut().find(|a| a.hash == article.hash) {
                local.read = !article.read;
            }
            self.recount_current_feed_unread();
            self.pending_redraw = true;
        }
    }

    fn open_current_article(&mut self) {
        if let Some(article) = self.current_article() {
            self.status = match open_in_browser(&article.link) {
                Ok(()) => "Opened in browser".to_string(),
                Err(e) => format!("Failed to open browser: {}", e),
            };
            self.pending_redraw = true;
        }
    }

    fn mark_current_article_read(&mut self, cmd_tx: &mpsc::Sender<BackendCommand>) {
        if let Some(article) = self.current_article().cloned() {
            if article.read {
                return;
            }
            let _ = cmd_tx.send(BackendCommand::MarkRead {
                hash: article.hash.clone(),
                read: true,
            });
            if let Some(local) = self.articles.iter_mut().find(|a| a.hash == article.hash) {
                local.read = true;
            }
            self.recount_current_feed_unread();
        }
    }

    fn draw(&mut self, terminal: &Terminal) -> Result<(), String> {
        if !self.pending_redraw {
            return Ok(());
        }

        let (width, height) = terminal.size();
        self.last_size = (width, height);
        let mut lines = Vec::with_capacity(height);
        lines.push(self.style_header(views::truncate(
            &format!(
                "Timmy's News Console - Last Updated {}",
                self.last_updated.as_deref().unwrap_or("never")
            ),
            width,
        )));
        let content_height = height.saturating_sub(1);
        let mut content_lines = Vec::with_capacity(content_height);

        match self.view {
            View::FeedList => self.render_feed_list(width, content_height, &mut content_lines),
            View::ArticleList => {
                self.render_article_list(width, content_height, &mut content_lines)
            }
            View::ArticleView => {
                self.render_article_view(width, content_height, &mut content_lines)
            }
            View::Log => self.render_log_view(width, content_height, &mut content_lines),
            View::Help => self.render_help(width, content_height, &mut content_lines),
        }
        lines.extend(content_lines);

        terminal.draw(&lines)?;
        self.pending_redraw = false;
        Ok(())
    }

    fn render_feed_list(&self, width: usize, height: usize, lines: &mut Vec<String>) {
        lines.push(self.style_header(views::truncate(
            "Feeds (Enter open feed/log, g refresh, u mark read, ? help, q quit)",
            width,
        )));

        let list_rows = height.saturating_sub(2);
        let start = self
            .selected_feed
            .saturating_sub(list_rows.saturating_sub(1));
        for idx in start..self.feed_row_count().min(start + list_rows) {
            let (name, updated, total, unread, has_error) = if idx == 0 {
                (
                    "[All]".to_string(),
                    self.last_updated.clone().unwrap_or_else(|| "-".to_string()),
                    self.total_articles(),
                    self.total_unread(),
                    false,
                )
            } else if idx == 1 {
                (
                    "[Unread]".to_string(),
                    self.last_updated.clone().unwrap_or_else(|| "-".to_string()),
                    self.total_unread(),
                    self.total_unread(),
                    false,
                )
            } else if idx == self.log_row_index() {
                ("[Log]".to_string(), "-".to_string(), 0, 0, false)
            } else {
                let feed = &self.feeds[idx - 2];
                (
                    feed.name.clone(),
                    feed.last_updated.clone().unwrap_or_else(|| "-".to_string()),
                    feed.total,
                    feed.unread,
                    feed.last_error.is_some(),
                )
            };
            let marker = if idx == self.selected_feed { ">" } else { " " };
            let err = if has_error { " !" } else { "" };
            let line = format!(
                "{} {:<22} {:<19} {:>5} total {:>5} unread{}",
                marker, name, updated, total, unread, err
            );
            let line = views::truncate(&line, width);
            if idx == self.selected_feed {
                lines.push(self.style_selection(line, false));
            } else {
                lines.push(line);
            }
        }

        while lines.len() + 1 < height {
            lines.push(String::new());
        }
        lines.push(self.style_status(views::truncate(&self.status, width)));
    }

    fn render_article_list(&self, width: usize, height: usize, lines: &mut Vec<String>) {
        let feed_name = match self.selected_feed_scope.as_ref() {
            Some(FeedScope::All) => "[All]",
            Some(FeedScope::Unread) => "[Unread]",
            Some(FeedScope::Feed(url)) => self
                .feeds
                .iter()
                .find(|f| &f.url == url)
                .map(|f| f.name.as_str())
                .unwrap_or("Articles"),
            None => "Articles",
        };
        let mut header = format!(
            "{} (Enter open, / search, u toggle read, o open, q back)",
            feed_name
        );
        if !self.search.is_empty() {
            header.push_str(&format!(" [search: {}]", self.search));
        }
        if self.search_mode {
            header.push_str(" (typing search)");
        }
        lines.push(self.style_header(views::truncate(&header, width)));

        let visible = self.filtered_article_indices();
        let show_feed_source = matches!(
            self.selected_feed_scope.as_ref(),
            Some(FeedScope::All | FeedScope::Unread)
        );
        let list_rows = height.saturating_sub(2);
        let start = self
            .selected_article
            .saturating_sub(list_rows.saturating_sub(1));
        for (list_idx, article_idx) in visible
            .iter()
            .copied()
            .enumerate()
            .skip(start)
            .take(list_rows)
        {
            let article = &self.articles[article_idx];
            let marker = if list_idx == self.selected_article {
                ">"
            } else {
                " "
            };
            let unread = if article.read { " " } else { "*" };
            let published = article
                .published
                .as_deref()
                .and_then(normalize_datetime_to_local)
                .unwrap_or_else(|| "No date".to_string());
            let title = views::strip_newlines(&article.title);
            let line = if show_feed_source {
                format!(
                    "{}{} [{}] [{}] {}",
                    marker,
                    unread,
                    published,
                    views::strip_newlines(&article.feed_name),
                    title
                )
            } else {
                format!("{}{} [{}] {}", marker, unread, published, title)
            };
            let line = views::truncate(&line, width);
            if list_idx == self.selected_article {
                lines.push(self.style_selection(line, !article.read));
            } else if !article.read {
                lines.push(self.style_unread(line));
            } else {
                lines.push(line);
            }
        }

        while lines.len() + 1 < height {
            lines.push(String::new());
        }
        lines.push(if visible.is_empty() {
            self.style_status(views::truncate("No matching articles", width))
        } else {
            self.style_status(views::truncate(&self.status, width))
        });
    }

    fn render_article_view(&self, width: usize, height: usize, lines: &mut Vec<String>) {
        let Some(article) = self.current_article() else {
            lines.push(views::truncate("No article selected", width));
            return;
        };

        let header = format!(
            "{} [{}] (q back, j/k scroll, PgUp/PgDn, n/p next/prev, u toggle, o open)",
            views::strip_newlines(&article.title),
            if article.read { "read" } else { "unread" }
        );
        lines.push(self.style_header(views::truncate(&header, width)));

        let html = if article.content.trim().is_empty() {
            article.description.as_str()
        } else {
            article.content.as_str()
        };
        let text = html2text::from_read(html.as_bytes(), width.max(20))
            .unwrap_or_else(|_| views::strip_newlines(html));
        let rendered: Vec<String> = text
            .lines()
            .map(|line| views::truncate(line, width))
            .collect();

        let body_rows = height.saturating_sub(2);
        let start = self.article_scroll.min(rendered.len());
        for line in rendered.iter().skip(start).take(body_rows) {
            lines.push(line.clone());
        }
        while lines.len() + 1 < height {
            lines.push(String::new());
        }

        let footer = match article.published.as_ref() {
            Some(published) => format!(
                "{} | {}",
                article.link,
                normalize_datetime_to_local(published).unwrap_or_else(|| "No date".to_string())
            ),
            None => article.link.clone(),
        };
        lines.push(self.style_status(views::truncate(&footer, width)));
    }

    fn render_help(&self, width: usize, height: usize, lines: &mut Vec<String>) {
        lines.push(self.style_header(views::truncate("Help (q to close)", width)));
        for item in keybindings::GLOBAL {
            lines.push(views::truncate(&format!("Global: {}", item), width));
        }
        for item in keybindings::FEED_LIST {
            lines.push(views::truncate(&format!("Feed list: {}", item), width));
        }
        for item in keybindings::ARTICLE_LIST {
            lines.push(views::truncate(&format!("Article list: {}", item), width));
        }
        for item in keybindings::ARTICLE_VIEW {
            lines.push(views::truncate(&format!("Article view: {}", item), width));
        }
        for item in keybindings::LOG_VIEW {
            lines.push(views::truncate(&format!("Log view: {}", item), width));
        }
        lines.push(views::truncate(
            "Mouse: feed click selects, article click opens, wheel scrolls lists/view",
            width,
        ));

        while lines.len() + 1 < height {
            lines.push(String::new());
        }
        lines.push(self.style_status(views::truncate(&self.status, width)));
    }

    fn render_log_view(&self, width: usize, height: usize, lines: &mut Vec<String>) {
        let (label, path) = match self.log_tab {
            LogTab::News => ("[News Log]", crate::log::news_log_path()),
            LogTab::Debug => ("[Debug Log]", crate::log::debug_log_path()),
        };
        lines.push(self.style_header(views::truncate(
            &format!("{} (n news, d debug, j/k scroll, PgUp/PgDn, q back)", label),
            width,
        )));

        let log_text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| format!("Could not read {} ({})", path.display(), e));
        let rendered: Vec<String> = if log_text.is_empty() {
            vec!["Log is empty".to_string()]
        } else {
            log_text
                .lines()
                .map(|line| views::truncate(line, width))
                .collect()
        };

        let body_rows = height.saturating_sub(2);
        let start = self
            .log_scroll
            .min(rendered.len().saturating_sub(body_rows.max(1)));
        for line in rendered.iter().skip(start).take(body_rows) {
            lines.push(line.clone());
        }
        while lines.len() + 1 < height {
            lines.push(String::new());
        }
        lines.push(self.style_status(views::truncate(
            &format!("{} lines | {}", rendered.len(), path.display()),
            width,
        )));
    }

    fn style_header(&self, s: String) -> String {
        style_line(&s, self.theme.header_fg, None, true, false)
    }

    fn style_status(&self, s: String) -> String {
        style_line(&s, self.theme.status_fg, self.theme.status_bg, false, true)
    }

    fn style_selection(&self, s: String, bold: bool) -> String {
        style_line(
            &s,
            self.theme.selection_fg,
            self.theme.selection_bg,
            bold,
            true,
        )
    }

    fn style_unread(&self, s: String) -> String {
        style_line(&s, self.theme.bold_fg, None, true, false)
    }

    fn reload_articles(&mut self, cache: &Cache) {
        self.articles.clear();
        if let Some(scope) = self.selected_feed_scope.as_ref() {
            match scope {
                FeedScope::Feed(url) => {
                    let hashes = cache.get_feed_index(url).unwrap_or_default();
                    self.articles = hashes
                        .iter()
                        .filter_map(|hash| cache.get_article(hash))
                        .collect();
                }
                FeedScope::All | FeedScope::Unread => {
                    let per_feed_articles: Vec<Vec<Article>> = self
                        .feeds
                        .iter()
                        .map(|feed| {
                            cache
                                .get_feed_index(&feed.url)
                                .unwrap_or_default()
                                .iter()
                                .filter_map(|hash| cache.get_article(hash))
                                .filter(|a| matches!(scope, FeedScope::All) || !a.read)
                                .collect::<Vec<_>>()
                        })
                        .collect();
                    let mut positions = vec![0usize; per_feed_articles.len()];
                    loop {
                        let mut progressed = false;
                        for (feed_idx, feed_articles) in per_feed_articles.iter().enumerate() {
                            let pos = positions[feed_idx];
                            if let Some(article) = feed_articles.get(pos).cloned() {
                                self.articles.push(article);
                                positions[feed_idx] = pos + 1;
                                progressed = true;
                            }
                        }
                        if !progressed {
                            break;
                        }
                    }
                }
            }
        }
        self.articles.sort_by(|a, b| {
            let a_ts = datetime_sort_key(a.published.as_deref()).unwrap_or(i64::MIN);
            let b_ts = datetime_sort_key(b.published.as_deref()).unwrap_or(i64::MIN);
            b_ts.cmp(&a_ts).then_with(|| a.title.cmp(&b.title))
        });
        let visible_len = self.filtered_article_indices().len();
        if self.selected_article >= visible_len {
            self.selected_article = visible_len.saturating_sub(1);
        }
    }

    fn recount_current_feed_unread(&mut self) {
        let Some(url) = self.selected_feed_scope.as_ref() else {
            return;
        };
        if let FeedScope::Feed(url) = url {
            let unread = self.articles.iter().filter(|a| !a.read).count();
            if let Some(row) = self.feeds.iter_mut().find(|f| &f.url == url) {
                row.unread = unread;
                row.total = self.articles.len();
            }
        }
    }

    fn filtered_article_indices(&self) -> Vec<usize> {
        if self.search.is_empty() {
            return (0..self.articles.len()).collect();
        }
        let query = self.search.to_lowercase();
        self.articles
            .iter()
            .enumerate()
            .filter_map(|(idx, article)| {
                let hay = format!(
                    "{} {} {}",
                    article.title.to_lowercase(),
                    article.description.to_lowercase(),
                    article.content.to_lowercase()
                );
                if hay.contains(&query) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    fn current_article(&self) -> Option<&Article> {
        let visible = self.filtered_article_indices();
        let idx = *visible.get(self.selected_article)?;
        self.articles.get(idx)
    }

    fn feed_row_count(&self) -> usize {
        self.feeds.len() + 3
    }

    fn log_row_index(&self) -> usize {
        self.feeds.len() + 2
    }

    fn scope_for_selected_feed(&self) -> FeedScope {
        if self.selected_feed == 0 {
            FeedScope::All
        } else if self.selected_feed == 1 {
            FeedScope::Unread
        } else if self.selected_feed >= self.log_row_index() {
            FeedScope::All
        } else {
            FeedScope::Feed(self.feeds[self.selected_feed - 2].url.clone())
        }
    }

    fn total_articles(&self) -> usize {
        self.feeds.iter().map(|f| f.total).sum()
    }

    fn total_unread(&self) -> usize {
        self.feeds.iter().map(|f| f.unread).sum()
    }

    fn feed_list_rows(&self) -> usize {
        self.last_size.1.saturating_sub(3).min(self.page_size)
    }

    fn article_list_rows(&self) -> usize {
        self.last_size.1.saturating_sub(3).min(self.page_size)
    }
}

fn open_in_browser(url: &str) -> Result<(), String> {
    if let Ok(browser) = std::env::var("BROWSER") {
        let status = std::process::Command::new(browser)
            .arg(url)
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            return Ok(());
        }
        return Err(format!("non-zero exit status: {}", status));
    }

    for opener in ["xdg-open", "open"] {
        match std::process::Command::new(opener).arg(url).status() {
            Ok(status) if status.success() => return Ok(()),
            _ => continue,
        }
    }

    Err("no browser opener available".to_string())
}

fn parse_color_opt(v: &Option<String>) -> Option<(u8, u8, u8)> {
    v.as_deref()
        .and_then(|hex| crate::config::Theme::parse_color(hex).ok())
}

fn style_line(
    text: &str,
    fg: Option<(u8, u8, u8)>,
    bg: Option<(u8, u8, u8)>,
    bold: bool,
    reverse_fallback: bool,
) -> String {
    let mut seq = String::new();
    let has_colors = fg.is_some() || bg.is_some();
    if bold {
        seq.push_str("\x1b[1m");
    }
    if let Some((r, g, b)) = fg {
        seq.push_str(&format!("\x1b[38;2;{};{};{}m", r, g, b));
    }
    if let Some((r, g, b)) = bg {
        seq.push_str(&format!("\x1b[48;2;{};{};{}m", r, g, b));
    }
    if reverse_fallback && !has_colors {
        seq.push_str("\x1b[7m");
    }

    if seq.is_empty() {
        text.to_string()
    } else {
        format!("{}{}", seq, text)
    }
}
