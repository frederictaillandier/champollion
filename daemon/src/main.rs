mod cards;
mod extract;
mod record;
mod study;
mod tray;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use clap::Parser;
use signal_hook::consts::{SIGINT, SIGTERM};
use tracing_subscriber::EnvFilter;

use record::{Recording, Settings, steam, window};
use tray::{Command, State, Tray};

/// Kingdom Come: Deliverance II.
const DEFAULT_APP_ID: u32 = 1771300;

/// Time a locked game gets to quit after being asked, before being killed.
const QUIT_TIMEOUT: Duration = Duration::from_secs(10);

/// Records the screen while a Steam game runs so its on-screen text can be studied.
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Where recordings are stored
    /// [default: $XDG_DATA_HOME/champollion/recordings]
    #[arg(long, env = "CHAMPOLLION_OUTPUT_DIR")]
    output_dir: Option<PathBuf>,

    /// Steam app id of the game to record (default: Kingdom Come: Deliverance II)
    #[arg(long, default_value_t = DEFAULT_APP_ID, env = "CHAMPOLLION_APP_ID")]
    app_id: u32,

    /// Frames per second of the recording; each extra frame costs CPU time
    #[arg(long, default_value_t = 15, env = "CHAMPOLLION_FRAMERATE")]
    framerate: u32,

    /// Encoder quantizer, 0-51: lower keeps text sharper but makes bigger files
    #[arg(long, default_value_t = 20, env = "CHAMPOLLION_QP")]
    qp: u32,

    /// Minutes of video per file; text is read from a file once it is
    /// finished
    #[arg(long, default_value_t = 2, env = "CHAMPOLLION_SEGMENT_MINUTES")]
    segment_minutes: u64,

    /// Where the text read from recordings is stored
    /// [default: $XDG_DATA_HOME/champollion/text]
    #[arg(long, env = "CHAMPOLLION_TEXT_DIR")]
    text_dir: Option<PathBuf>,

    /// Don't read text from recordings
    #[arg(long, env = "CHAMPOLLION_NO_OCR")]
    no_ocr: bool,

    /// Tesseract language of the game's text (`tesseract --list-langs`)
    #[arg(long, default_value = "ces", env = "CHAMPOLLION_OCR_LANG")]
    ocr_lang: String,

    /// Frames read per second of video
    #[arg(long, default_value_t = 1, env = "CHAMPOLLION_OCR_FPS")]
    ocr_fps: u32,

    /// Frames read in parallel; each worker loads its own copy of the
    /// language model
    #[arg(long, default_value_t = 2, env = "CHAMPOLLION_OCR_WORKERS")]
    ocr_workers: usize,

    /// Minimum OCR confidence (0-100) for a word to be kept
    #[arg(long, default_value_t = 70.0, env = "CHAMPOLLION_OCR_MIN_CONFIDENCE")]
    ocr_min_confidence: f32,

    /// Backend receiving flashcards of the words read, e.g.
    /// `http://10.0.0.1:8090`; no cards are made without it
    #[arg(long, env = "CHAMPOLLION_BACKEND_URL")]
    backend_url: Option<String>,

    /// Claude Code executable, which makes the cards
    #[arg(long, default_value = "claude", env = "CHAMPOLLION_CLAUDE")]
    claude: PathBuf,

    /// Claude model making the cards (`sonnet`, `haiku`...)
    #[arg(long, default_value = "sonnet", env = "CHAMPOLLION_CARDS_MODEL")]
    cards_model: String,

    /// Don't make new cards, only send the ones already made
    #[arg(long, env = "CHAMPOLLION_NO_CARDS")]
    no_cards: bool,

    /// Cards due at which the game is closed and kept closed; 0 never
    /// closes it. Needs `--backend-url`
    #[arg(long, default_value_t = 50, env = "CHAMPOLLION_LOCK_AT")]
    lock_at: u64,

    /// Cards due under which the game can be played again
    #[arg(long, default_value_t = 10, env = "CHAMPOLLION_UNLOCK_BELOW")]
    unlock_below: u64,

    /// Seconds between two checks for the game
    #[arg(long, default_value_t = 2.0, env = "CHAMPOLLION_POLL_INTERVAL")]
    poll_interval: f64,
}

fn data_dir() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    data.join("champollion")
}

fn state_dir() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    state.join("champollion")
}

/// Waits until `deadline` for a command from the tray, waking early when a
/// shutdown is requested.
fn wait_for_command(
    deadline: Instant,
    stop: &AtomicBool,
    commands: &Receiver<Command>,
) -> Option<Command> {
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        match commands.recv_timeout((deadline - now).min(Duration::from_millis(200))) {
            Ok(command) => return Some(command),
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
        }
    }
    None
}

/// Environment variables read by libraries when they are loaded, before
/// `main` runs, so they can only be set by restarting the process.
const LOAD_TIME_ENV: [(&str, &str); 2] = [
    // Makes the NVIDIA driver sleep instead of spinning on the CPU while it
    // waits for the GPU, which otherwise doubles the CPU cost of recording.
    ("__GL_YIELD", "USLEEP"),
    // Tesseract's OpenMP threads compete with our own OCR workers.
    ("OMP_THREAD_LIMIT", "1"),
];

/// Re-executes the daemon with `LOAD_TIME_ENV` set, if it is not already.
fn reexec_with_load_time_env() {
    use std::os::unix::process::CommandExt;

    let missing: Vec<_> = LOAD_TIME_ENV
        .iter()
        .filter(|(name, _)| std::env::var_os(name).is_none())
        .collect();
    if missing.is_empty() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let error = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .envs(missing.iter().map(|(name, value)| (name, value)))
        .exec();
    eprintln!("could not restart with {missing:?}, continuing without: {error}");
}

fn main() {
    reexec_with_load_time_env();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,zbus=error".into()),
        )
        .init();

    let args = Args::parse();
    let output_dir = args
        .output_dir
        .clone()
        .unwrap_or_else(|| data_dir().join("recordings"));
    let text_dir = args
        .text_dir
        .clone()
        .unwrap_or_else(|| data_dir().join("text"));
    let poll_interval = Duration::from_secs_f64(args.poll_interval.max(0.1));
    let settings = Settings {
        framerate: args.framerate.max(1),
        qp: args.qp.min(51),
        segment_secs: args.segment_minutes.max(1) * 60,
    };

    gstreamer::init().expect("initialize GStreamer");
    // The portal's D-Bus connection is driven by this runtime's worker thread
    // for as long as a recording lasts.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("start tokio runtime");

    let stop = Arc::new(AtomicBool::new(false));
    for signal in [SIGINT, SIGTERM] {
        signal_hook::flag::register(signal, Arc::clone(&stop)).expect("register signal handler");
    }

    let game_name =
        steam::game_name(args.app_id).unwrap_or_else(|| format!("Steam app {}", args.app_id));
    tracing::info!(
        "waiting for {game_name}, storing recordings in {}",
        output_dir.display()
    );

    let indexer = (!args.no_ocr).then(|| {
        extract::Indexer::spawn(
            extract::Config {
                recordings_dir: output_dir.clone(),
                text_dir: text_dir.clone(),
                samples_per_second: args.ocr_fps,
                workers: args.ocr_workers.max(1),
                ocr: extract::Ocr {
                    lang: args.ocr_lang.clone(),
                    min_confidence: args.ocr_min_confidence,
                },
            },
            Arc::clone(&stop),
        )
    });

    let cards = args.backend_url.as_ref().map(|url| {
        cards::spawn(
            cards::Config {
                text_dir: text_dir.clone(),
                lang: args.ocr_lang.clone(),
                claude: (!args.no_cards).then(|| cards::Claude {
                    bin: args.claude.clone(),
                    model: args.cards_model.clone(),
                }),
                backend_url: url.trim_end_matches('/').to_owned(),
            },
            Arc::clone(&stop),
        )
    });

    let due_cards = args
        .backend_url
        .as_ref()
        .filter(|_| args.lock_at > 0)
        .map(|url| study::DueCards::spawn(url.trim_end_matches('/'), Arc::clone(&stop)));
    let mut lock = study::Lock::load(args.lock_at, args.unlock_below, state_dir().join("locked"));

    let (commands_tx, commands) = mpsc::channel();
    let tray = rt.block_on(ksni::TrayMethods::spawn(Tray {
        state: State::Waiting,
        game: game_name,
        recordings_dir: output_dir.clone(),
        commands: commands_tx.clone(),
    }));
    let tray = match tray {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!("no tray icon, the desktop has no system tray: {e}");
            None
        }
    };
    let show = |state: State| {
        if let Some(tray) = &tray {
            rt.block_on(tray.update(|t| t.state = state));
        }
    };

    // With its file prefix.
    let mut recording: Option<(Recording, PathBuf)> = None;
    let mut paused = false;
    // Why the last attempt to record failed. Kept until the game restarts so
    // a cancelled screen-sharing dialog is not shown again in a loop.
    let mut failure: Option<String> = None;
    let mut windows = window::Windows::new();
    // Logged once per launch, while Steam is still in front of the game.
    let mut waiting_for_window = false;
    let mut shown = String::new();
    // The locked game being closed: its `reaper`, when it was asked to quit,
    // and whether it was killed since.
    let mut closing: Option<(u32, Instant, bool)> = None;

    while !stop.load(Ordering::Relaxed) {
        let tick = Instant::now();
        let game = steam::running_game(args.app_id);

        if let Some((rec, _)) = &recording {
            if game.is_none() || paused {
                tracing::info!(
                    "{}",
                    if paused {
                        "recording paused"
                    } else {
                        "game stopped"
                    }
                );
                recording.take().unwrap().0.stop(&rt, true);
            } else if let Some(reason) = rec.failure() {
                tracing::error!("recording failed: {reason}");
                recording.take().unwrap().0.stop(&rt, false);
                failure = Some(reason);
            }
        }

        let due = due_cards.as_ref().and_then(|d| *d.due.lock().unwrap());
        let locked = lock.update(due).then(|| due.unwrap_or(0));
        match (&game, locked) {
            (Some(game), Some(due)) => match &mut closing {
                Some((pid, asked, killed)) if *pid == game.launcher_pid => {
                    if !*killed && asked.elapsed() >= QUIT_TIMEOUT {
                        tracing::warn!("{} did not quit, killing it", game.name);
                        game.signal(libc::SIGKILL);
                        *killed = true;
                    }
                }
                _ => {
                    tracing::info!("{due} cards are due, closing {}", game.name);
                    let body = format!(
                        "{due} cards are due. {} stays closed until fewer than {} are.",
                        game.name,
                        lock.unlock_below()
                    );
                    if let Err(e) = rt.block_on(study::notify("Time to study", &body)) {
                        tracing::warn!("could not show a notification: {e}");
                    }
                    game.signal(libc::SIGTERM);
                    closing = Some((game.launcher_pid, Instant::now(), false));
                }
            },
            _ => closing = None,
        }

        match &game {
            // Closed above, not worth recording.
            Some(_) if locked.is_some() => {}
            None => {
                failure = None;
                waiting_for_window = false;
            }
            // Steam shows its own (English) window for a while after the game
            // process starts; only record once the game's window is in front.
            Some(game)
                if recording.is_none()
                    && failure.is_none()
                    && !paused
                    && windows.game_in_front(game) == Some(false) =>
            {
                if !waiting_for_window {
                    tracing::info!("{} is starting, waiting for its window", game.name);
                    waiting_for_window = true;
                }
            }
            Some(game) if recording.is_none() && failure.is_none() && !paused => {
                tracing::info!("starting to record {} ({})", game.name, game.app_id);
                let prefix = record::file_prefix(&output_dir, game);
                // Before the first file exists, so it is not read unfinished.
                if let Some(indexer) = &indexer {
                    *indexer.recording.lock().unwrap() = Some(prefix.clone());
                }
                match Recording::start(&rt, &prefix, &settings) {
                    Ok(rec) => recording = Some((rec, prefix)),
                    Err(e) => {
                        tracing::error!("{e}");
                        failure = Some(e);
                    }
                }
            }
            Some(_) => {}
        }

        let state = match (&recording, &failure) {
            _ if paused => State::Paused,
            (Some((rec, _)), _) => State::Recording { since: rec.since },
            _ if let Some(due) = locked => State::Locked {
                due,
                unlock_below: lock.unlock_below(),
            },
            (None, Some(reason)) => State::Failed(reason.clone()),
            (None, None) => match indexer
                .as_ref()
                .and_then(|i| i.status.lock().unwrap().clone())
            {
                Some(progress) => State::Reading(progress),
                None => State::Waiting,
            },
        };
        if let Some(indexer) = &indexer {
            indexer.playing.store(game.is_some(), Ordering::Relaxed);
            *indexer.recording.lock().unwrap() = recording.as_ref().map(|(_, p)| p.clone());
        }
        let key = match &state {
            State::Waiting => "waiting".to_owned(),
            State::Recording { .. } => "recording".to_owned(),
            State::Paused => "paused".to_owned(),
            State::Failed(reason) => format!("failed: {reason}"),
            State::Reading(progress) => progress.clone(),
            State::Locked { due, .. } => format!("locked: {due}"),
        };
        if key != shown {
            show(state);
            shown = key;
        }

        match wait_for_command(tick + poll_interval, &stop, &commands) {
            Some(Command::SetPaused(p)) => paused = p,
            Some(Command::Quit) => {
                stop.store(true, Ordering::Relaxed);
                break;
            }
            None => {}
        }
    }

    tracing::info!("shutting down");
    if let Some((rec, _)) = recording {
        rec.stop(&rt, true);
    }
    if let Some(indexer) = indexer {
        indexer.join();
    }
    if let Some(cards) = cards {
        let _ = cards.join();
    }
    if let Some(due_cards) = due_cards {
        due_cards.join();
    }
    drop(commands_tx);
}
