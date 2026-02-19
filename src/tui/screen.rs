use std::io::{self, Write};
use std::mem;

use crate::config::Theme;

pub struct Terminal {
    original: libc::termios,
    mouse_enabled: bool,
    base_seq: String,
}

impl Terminal {
    pub fn enter(mouse: bool, theme: &Theme) -> Result<Self, String> {
        let fd = libc::STDIN_FILENO;
        let mut termios = unsafe { mem::zeroed::<libc::termios>() };
        if unsafe { libc::tcgetattr(fd, &mut termios) } != 0 {
            return Err("tcgetattr failed".to_string());
        }

        let original = termios;

        termios.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG);
        termios.c_iflag &= !(libc::IXON | libc::ICRNL);
        termios.c_oflag &= !(libc::OPOST);
        termios.c_cc[libc::VMIN] = 1;
        termios.c_cc[libc::VTIME] = 0;

        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &termios) } != 0 {
            return Err("tcsetattr failed".to_string());
        }

        let base_seq = base_theme_sequence(theme);
        print!("\x1b[?1049h\x1b[?25l");
        if mouse {
            print!("\x1b[?1000h\x1b[?1006h");
        }
        print!("\x1b[2J\x1b[H{}", base_seq);
        io::stdout().flush().map_err(|e| e.to_string())?;

        Ok(Self {
            original,
            mouse_enabled: mouse,
            base_seq,
        })
    }

    pub fn set_mouse(&mut self, enabled: bool) {
        if enabled && !self.mouse_enabled {
            print!("\x1b[?1000h\x1b[?1006h");
            let _ = io::stdout().flush();
            self.mouse_enabled = true;
        } else if !enabled && self.mouse_enabled {
            print!("\x1b[?1006l\x1b[?1000l");
            let _ = io::stdout().flush();
            // Keep mouse_enabled true so Drop still cleans up
        }
    }

    pub fn size(&self) -> (usize, usize) {
        let mut ws = unsafe { mem::zeroed::<libc::winsize>() };
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) } == 0 {
            (ws.ws_col.max(1) as usize, ws.ws_row.max(1) as usize)
        } else {
            (80, 24)
        }
    }

    pub fn draw(&self, lines: &[String]) -> Result<(), String> {
        let mut out = io::stdout();
        write!(out, "\x1b[H\x1b[2J").map_err(|e| e.to_string())?;
        for line in lines {
            write!(out, "\x1b[2K{}{}\x1b[K\x1b[0m\r\n", self.base_seq, line)
                .map_err(|e| e.to_string())?;
        }
        out.flush().map_err(|e| e.to_string())
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original) };
        if self.mouse_enabled {
            let _ = write!(io::stdout(), "\x1b[?1006l\x1b[?1000l");
        }
        let _ = write!(io::stdout(), "\x1b[0m\x1b[?25h\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

fn base_theme_sequence(theme: &Theme) -> String {
    let mut seq = String::new();
    if let Some((r, g, b)) = parse_color_opt(&theme.bg) {
        seq.push_str(&format!("\x1b[48;2;{};{};{}m", r, g, b));
    }
    if let Some((r, g, b)) = parse_color_opt(&theme.fg) {
        seq.push_str(&format!("\x1b[38;2;{};{};{}m", r, g, b));
    }
    seq
}

fn parse_color_opt(v: &Option<String>) -> Option<(u8, u8, u8)> {
    v.as_deref().and_then(|hex| Theme::parse_color(hex).ok())
}
