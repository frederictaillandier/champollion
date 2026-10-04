//! Tells whether the game's window is in front, so recording does not start
//! while only Steam's window (and its English text) is on screen.
//!
//! Steam and Proton games are X11 windows under XWayland, and the compositor
//! keeps `_NET_ACTIVE_WINDOW` up to date among X11 windows.

use std::fs;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, Window};
use x11rb::rust_connection::RustConnection;

use crate::steam::Game;

pub struct Windows {
    x11: Option<X11>,
}

struct X11 {
    conn: RustConnection,
    root: Window,
    active_window: Atom,
    wm_pid: Atom,
}

impl Windows {
    pub fn new() -> Self {
        Self { x11: None }
    }

    /// Whether the active X11 window belongs to `game`; `None` when that
    /// cannot be known (no X server), in which case callers should not wait.
    pub fn game_in_front(&mut self, game: &Game) -> Option<bool> {
        if self.x11.is_none() {
            self.x11 = X11::connect()
                .inspect_err(|e| tracing::debug!("no X11 connection: {e}"))
                .ok();
        }
        let result = self.x11.as_ref()?.active_window_is(game);
        if let Err(e) = &result {
            // The X server may have restarted: reconnect next time.
            tracing::debug!("could not read the active window: {e}");
            self.x11 = None;
        }
        result.ok()
    }
}

type Error = Box<dyn std::error::Error>;

impl X11 {
    fn connect() -> Result<Self, Error> {
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        let atom = |name: &[u8]| -> Result<Atom, Error> {
            Ok(conn.intern_atom(false, name)?.reply()?.atom)
        };
        let active_window = atom(b"_NET_ACTIVE_WINDOW")?;
        let wm_pid = atom(b"_NET_WM_PID")?;
        Ok(Self {
            conn,
            root,
            active_window,
            wm_pid,
        })
    }

    fn active_window_is(&self, game: &Game) -> Result<bool, Error> {
        let reply = self
            .conn
            .get_property(false, self.root, self.active_window, AtomEnum::WINDOW, 0, 1)?
            .reply()?;
        let Some(window) = reply
            .value32()
            .and_then(|mut v| v.next())
            .filter(|&w| w != 0)
        else {
            return Ok(false);
        };

        // Proton names the window class of games `steam_app_<id>`.
        let class = self
            .conn
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)?
            .reply()?;
        let steam_class = format!("steam_app_{}", game.app_id);
        if class
            .value
            .split(|&b| b == 0)
            .any(|part| part == steam_class.as_bytes())
        {
            return Ok(true);
        }

        // Otherwise, the window's process must have been started by the
        // game's launcher.
        let pid = self
            .conn
            .get_property(false, window, self.wm_pid, AtomEnum::CARDINAL, 0, 1)?
            .reply()?
            .value32()
            .and_then(|mut v| v.next());
        Ok(pid.is_some_and(|pid| descends_from(pid, game.launcher_pid)))
    }
}

/// Whether process `pid` is `ancestor` or one of its descendants.
fn descends_from(mut pid: u32, ancestor: u32) -> bool {
    for _ in 0..64 {
        if pid == ancestor {
            return true;
        }
        match parent(pid) {
            Some(ppid) if ppid > 1 => pid = ppid,
            _ => return false,
        }
    }
    false
}

fn parent(pid: u32) -> Option<u32> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (comm) state ppid ...`; comm may contain spaces and parentheses.
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_own_ancestry() {
        let me = std::process::id();
        let parent = parent(me).unwrap();
        assert!(descends_from(me, me));
        assert!(descends_from(me, parent));
        assert!(!descends_from(parent, me));
    }
}
