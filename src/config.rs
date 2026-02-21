use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default, rename = "feed")]
    pub feeds: Vec<FeedConfig>,
}

#[derive(Debug, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    #[serde(default = "default_scrolloff")]
    pub scrolloff: usize,
    #[serde(default = "default_true")]
    pub mouse: bool,
    #[serde(default = "default_sync_interval")]
    pub sync_interval_secs: u64,
    pub browser: Option<String>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            page_size: default_page_size(),
            scrolloff: default_scrolloff(),
            mouse: true,
            sync_interval_secs: default_sync_interval(),
            browser: None,
        }
    }
}

fn default_page_size() -> usize {
    100
}

fn default_scrolloff() -> usize {
    0
}

fn default_true() -> bool {
    true
}

fn default_sync_interval() -> u64 {
    300
}

#[derive(Debug, Default, Deserialize)]
pub struct Theme {
    pub bg: Option<String>,
    pub fg: Option<String>,
    pub bold_fg: Option<String>,
    pub selection_bg: Option<String>,
    pub selection_fg: Option<String>,
    pub status_bg: Option<String>,
    pub status_fg: Option<String>,
    pub header_fg: Option<String>,
}

impl Theme {
    pub fn parse_color(hex: &str) -> Result<(u8, u8, u8), String> {
        if hex.len() != 7 || !hex.starts_with('#') {
            return Err(format!("invalid color '{}': expected #RRGGBB", hex));
        }
        let r =
            u8::from_str_radix(&hex[1..3], 16).map_err(|_| format!("invalid color '{}'", hex))?;
        let g =
            u8::from_str_radix(&hex[3..5], 16).map_err(|_| format!("invalid color '{}'", hex))?;
        let b =
            u8::from_str_radix(&hex[5..7], 16).map_err(|_| format!("invalid color '{}'", hex))?;
        Ok((r, g, b))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct FeedConfig {
    pub name: String,
    pub url: String,
}

impl Config {
    pub fn load(path: Option<&str>) -> Result<Config, String> {
        let config_path = match path {
            Some(p) => PathBuf::from(p),
            None => {
                let xdg = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
                    let home = std::env::var("HOME").unwrap_or_default();
                    format!("{}/.config", home)
                });
                PathBuf::from(xdg).join("tn").join("config.toml")
            }
        };

        let contents = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("failed to read {}: {}", config_path.display(), e))?;

        let config: Config =
            toml::from_str(&contents).map_err(|e| format!("config parse error: {}", e))?;

        if config.feeds.is_empty() {
            return Err("no [[feed]] entries in config".to_string());
        }

        // Validate theme colors
        for (name, val) in [
            ("bg", &config.theme.bg),
            ("fg", &config.theme.fg),
            ("bold_fg", &config.theme.bold_fg),
            ("selection_bg", &config.theme.selection_bg),
            ("selection_fg", &config.theme.selection_fg),
            ("status_bg", &config.theme.status_bg),
            ("status_fg", &config.theme.status_fg),
            ("header_fg", &config.theme.header_fg),
        ] {
            if let Some(hex) = val {
                Theme::parse_color(hex).map_err(|e| format!("theme.{}: {}", name, e))?;
            }
        }

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to build "#RRGGBB" strings without tripping the Rust 2021 lexer.
    fn hex(s: &str) -> String {
        format!("#{}", s)
    }

    #[test]
    fn parse_minimal_config() {
        let toml_str = "[[feed]]\nname = \"Test\"\nurl = \"https://example.com/rss\"\n";
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.feeds.len(), 1);
        assert_eq!(config.feeds[0].name, "Test");
        assert_eq!(config.ui.page_size, 100);
        assert_eq!(config.ui.scrolloff, 0);
        assert_eq!(config.ui.sync_interval_secs, 300);
    }

    #[test]
    fn parse_full_config() {
        let toml_str = concat!(
            "[ui]\n",
            "page_size = 50\n",
            "scrolloff = 3\n",
            "mouse = false\n",
            "sync_interval_secs = 600\n",
            "\n",
            "[theme]\n",
            "bg = \"#002b36\"\n",
            "fg = \"#839496\"\n",
            "\n",
            "[[feed]]\n",
            "name = \"HN\"\n",
            "url = \"https://news.ycombinator.com/rss\"\n",
            "\n",
            "[[feed]]\n",
            "name = \"LWN\"\n",
            "url = \"https://lwn.net/headlines/rss\"\n",
        );
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.feeds.len(), 2);
        assert_eq!(config.ui.page_size, 50);
        assert_eq!(config.ui.scrolloff, 3);
        assert!(!config.ui.mouse);
        assert_eq!(config.theme.bg.as_deref(), Some(hex("002b36")).as_deref());
    }

    #[test]
    fn parse_color_valid() {
        assert_eq!(Theme::parse_color(&hex("002b36")), Ok((0, 43, 54)));
        assert_eq!(Theme::parse_color(&hex("ffffff")), Ok((255, 255, 255)));
    }

    #[test]
    fn parse_color_invalid() {
        assert!(Theme::parse_color("002b36").is_err());
        assert!(Theme::parse_color(&hex("gggggg")).is_err());
        assert!(Theme::parse_color(&hex("abc")).is_err());
    }
}
