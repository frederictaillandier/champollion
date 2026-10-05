use std::path::PathBuf;
use std::process::Command as Process;
use std::sync::mpsc::Sender;
use std::time::Instant;

use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip};

/// Requests from the tray menu, handled by the main loop.
pub enum Command {
    SetPaused(bool),
    Quit,
}

#[derive(Clone)]
pub enum State {
    Waiting,
    Recording {
        since: Instant,
    },
    Paused,
    Failed(String),
    /// Reading text from recordings, with a progress description.
    Reading(String),
    /// Too many cards are due: the game is closed until fewer than
    /// `unlock_below` are.
    Locked {
        due: u64,
        unlock_below: u64,
    },
}

/// The icon in the desktop's top bar, with its right-click menu.
pub struct Tray {
    pub state: State,
    pub game: String,
    pub recordings_dir: PathBuf,
    pub commands: Sender<Command>,
}

impl Tray {
    fn status_text(&self) -> String {
        match &self.state {
            State::Waiting => format!("Waiting for {}", self.game),
            State::Recording { since } => {
                let minutes = since.elapsed().as_secs() / 60;
                format!("Recording {} · {minutes} min", self.game)
            }
            State::Paused => "Recording paused".into(),
            State::Failed(reason) => format!("Recording failed: {reason}"),
            State::Reading(progress) => progress.clone(),
            State::Locked { due, unlock_below } => {
                format!(
                    "Time to study: {due} cards due, {} unlocks under {unlock_below}",
                    self.game
                )
            }
        }
    }

    fn open_recordings(&self) {
        let _ = std::fs::create_dir_all(&self.recordings_dir);
        if let Err(e) = Process::new("xdg-open").arg(&self.recordings_dir).spawn() {
            tracing::warn!("could not open {}: {e}", self.recordings_dir.display());
        }
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "champollion".into()
    }

    fn title(&self) -> String {
        "Champollion".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        [16, 22, 32, 48]
            .into_iter()
            .map(|size| draw_icon(&self.state, size))
            .collect()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Champollion".into(),
            description: self.status_text(),
            ..Default::default()
        }
    }

    /// Left click opens the recordings; right click shows the menu.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.open_recordings();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let paused = matches!(self.state, State::Paused);
        vec![
            StandardItem {
                label: self.status_text(),
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Pause recording".into(),
                checked: paused,
                activate: Box::new(move |this: &mut Self| {
                    let _ = this.commands.send(Command::SetPaused(!paused));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Open recordings folder".into(),
                activate: Box::new(|this: &mut Self| this.open_recordings()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.commands.send(Command::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

const RED: [u8; 3] = [0xe0, 0x1b, 0x24];
const GREY: [u8; 3] = [0x9a, 0x99, 0x96];

/// Draws the tray icon: a red dot while recording, a grey ring while
/// waiting, grey bars when paused, a red ring after a failure and red bars
/// while the game is locked.
fn draw_icon(state: &State, size: i32) -> Icon {
    let s = size as f32;
    let center = s / 2.0;
    let radius = s * 0.36;
    let ring = (s * 0.1).max(1.5);

    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let dist = ((px - center).powi(2) + (py - center).powi(2)).sqrt();
            // Coverage in 0..1, with a one-pixel soft edge for antialiasing.
            let disc = (radius - dist + 0.5).clamp(0.0, 1.0);
            let hole = (radius - ring - dist + 0.5).clamp(0.0, 1.0);
            let (color, alpha) = match state {
                State::Recording { .. } => (RED, disc),
                State::Waiting | State::Reading(_) => (GREY, disc - hole),
                State::Failed(_) => (RED, disc - hole),
                State::Paused | State::Locked { .. } => {
                    let bar_w = s * 0.2;
                    let gap = s * 0.14;
                    let in_y = (py - center).abs() < radius;
                    let left = (px - (center - gap / 2.0 - bar_w / 2.0)).abs() < bar_w / 2.0;
                    let right = (px - (center + gap / 2.0 + bar_w / 2.0)).abs() < bar_w / 2.0;
                    let color = if matches!(state, State::Paused) {
                        GREY
                    } else {
                        RED
                    };
                    (color, if in_y && (left || right) { 1.0 } else { 0.0 })
                }
            };
            data.extend([(alpha * 255.0) as u8, color[0], color[1], color[2]]);
        }
    }
    Icon {
        width: size,
        height: size,
        data,
    }
}
