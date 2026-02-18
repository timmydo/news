use std::io::{self, Read};
use std::sync::mpsc;

#[derive(Debug, Clone, Copy)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

#[derive(Debug, Clone, Copy)]
pub enum MouseEvent {
    LeftClick { row: usize },
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    Key(Key),
    Mouse(MouseEvent),
}

pub fn spawn_input_thread(tx: mpsc::Sender<InputEvent>) {
    std::thread::spawn(move || {
        let mut stdin = io::stdin();
        let mut b = [0u8; 1];

        loop {
            if stdin.read_exact(&mut b).is_err() {
                break;
            }

            let event = match b[0] {
                b'\r' | b'\n' => Some(InputEvent::Key(Key::Enter)),
                127 | 8 => Some(InputEvent::Key(Key::Backspace)),
                0x1b => parse_escape(&mut stdin),
                c => Some(InputEvent::Key(Key::Char(c as char))),
            };

            if let Some(ev) = event {
                if tx.send(ev).is_err() {
                    break;
                }
            }
        }
    });
}

fn parse_escape(stdin: &mut io::Stdin) -> Option<InputEvent> {
    let mut b = [0u8; 1];
    stdin.read_exact(&mut b).ok()?;
    if b[0] == b'O' {
        stdin.read_exact(&mut b).ok()?;
        return match b[0] {
            b'H' => Some(InputEvent::Key(Key::Home)),
            b'F' => Some(InputEvent::Key(Key::End)),
            _ => None,
        };
    }
    if b[0] != b'[' {
        return None;
    }

    stdin.read_exact(&mut b).ok()?;
    match b[0] {
        b'A' => Some(InputEvent::Key(Key::Up)),
        b'B' => Some(InputEvent::Key(Key::Down)),
        b'H' => Some(InputEvent::Key(Key::Home)),
        b'F' => Some(InputEvent::Key(Key::End)),
        b'5' => {
            stdin.read_exact(&mut b).ok()?;
            if b[0] == b'~' {
                Some(InputEvent::Key(Key::PageUp))
            } else {
                None
            }
        }
        b'6' => {
            stdin.read_exact(&mut b).ok()?;
            if b[0] == b'~' {
                Some(InputEvent::Key(Key::PageDown))
            } else {
                None
            }
        }
        b'1' | b'7' => {
            stdin.read_exact(&mut b).ok()?;
            if b[0] == b'~' {
                Some(InputEvent::Key(Key::Home))
            } else {
                None
            }
        }
        b'4' | b'8' => {
            stdin.read_exact(&mut b).ok()?;
            if b[0] == b'~' {
                Some(InputEvent::Key(Key::End))
            } else {
                None
            }
        }
        b'<' => parse_sgr_mouse(stdin),
        _ => None,
    }
}

fn parse_sgr_mouse(stdin: &mut io::Stdin) -> Option<InputEvent> {
    let mut seq = Vec::with_capacity(16);
    let mut b = [0u8; 1];

    loop {
        stdin.read_exact(&mut b).ok()?;
        seq.push(b[0]);
        if b[0] == b'M' || b[0] == b'm' {
            break;
        }
        if seq.len() > 32 {
            return None;
        }
    }

    let s = String::from_utf8(seq).ok()?;
    let is_press = s.ends_with('M');
    let body = &s[..s.len().saturating_sub(1)];
    let mut parts = body.split(';');

    let cb = parts.next()?.parse::<usize>().ok()?;
    let _cx = parts.next()?.parse::<usize>().ok()?;
    let cy = parts.next()?.parse::<usize>().ok()?;

    if !is_press {
        return None;
    }

    match cb {
        0 => Some(InputEvent::Mouse(MouseEvent::LeftClick {
            row: cy.saturating_sub(1),
        })),
        64 => Some(InputEvent::Mouse(MouseEvent::ScrollUp)),
        65 => Some(InputEvent::Mouse(MouseEvent::ScrollDown)),
        _ => None,
    }
}
