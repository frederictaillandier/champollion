mod ocr;
mod recorder;
mod screencast;
mod steam;
mod text;
mod tray;
mod window;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use clap::Parser;
use signal_hook::consts::{SIGINT, SIGTERM};
use tracing_subscriber::EnvFilter;

use screencast::ScreenCast;
use recorder::{Recorder, Settings};
use tray::{Command, State, Tray};

/// Kingdom Come: Deliverance II.
const DEFAULT_APP_ID: u32 = 1771300;

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

    /// Minutes of video per file
    #[arg(long, default_value_t = 10, env = "CHAMPOLLION_SEGMENT_MINUTES")]
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

/// A screencast being recorded.
struct Recording {
    cast: ScreenCast,
    recorder: Recorder,
    since: Instant,
}

impl Recording {
    fn start(
        rt: &tokio::runtime::Runtime,
        game: &steam::Game,
        output_dir: &std::path::Path,
        settings: &Settings,
    ) -> Result<Self, String> {
        let cast = rt
            .block_on(ScreenCast::open())
            .map_err(|e| format!("could not share the screen: {e}"))?;
        let now = chrono::Local::now();
        let prefix = output_dir
            .join(game.slug())
            .join(now.format("%Y-%m-%d").to_string())
            .join(now.format("%H-%M-%S").to_string());
        match Recorder::start(&cast, &prefix, settings) {
            Ok(recorder) => {
                tracing::info!("recording {:?} to {}_NNN.mkv", cast.size, prefix.display());
                Ok(Self {
                    cast,
                    recorder,
                    since: Instant::now(),
                })
            }
            Err(e) => {
                rt.block_on(cast.close());
                Err(format!("could not start recording: {e}"))
            }
        }
    }

    /// Ends the recording; `finish` waits for the current file to be
    /// finalized, which is pointless once the pipeline has failed.
    fn stop(self, rt: &tokio::runtime::Runtime, finish: bool) {
        if finish {
            self.recorder.stop();
        } else {
            drop(self.recorder);
        }
        rt.block_on(self.cast.close());
        tracing::info!("recording stopped");
    }
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
        text::Indexer::spawn(
            text::Config {
                recordings_dir: output_dir.clone(),
                text_dir: text_dir.clone(),
                samples_per_second: args.ocr_fps,
                workers: args.ocr_workers.max(1),
                ocr: ocr::Ocr {
                    lang: args.ocr_lang.clone(),
                    min_confidence: args.ocr_min_confidence,
                },
            },
            Arc::clone(&stop),
        )
    });

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

    let mut recording: Option<Recording> = None;
    let mut paused = false;
    // Why the last attempt to record failed. Kept until the game restarts so
    // a cancelled screen-sharing dialog is not shown again in a loop.
    let mut failure: Option<String> = None;
    let mut windows = window::Windows::new();
    // Logged once per launch, while Steam is still in front of the game.
    let mut waiting_for_window = false;
    let mut shown = String::new();

    while !stop.load(Ordering::Relaxed) {
        let tick = Instant::now();
        let game = steam::running_game(args.app_id);

        if let Some(rec) = &recording {
            if game.is_none() || paused {
                tracing::info!(
                    "{}",
                    if paused {
                        "recording paused"
                    } else {
                        "game stopped"
                    }
                );
                recording.take().unwrap().stop(&rt, true);
            } else if let Some(reason) = rec.recorder.failure() {
                tracing::error!("recording failed: {reason}");
                recording.take().unwrap().stop(&rt, false);
                failure = Some(reason);
            }
        }

        match &game {
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
                match Recording::start(&rt, game, &output_dir, &settings) {
                    Ok(rec) => recording = Some(rec),
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
            (Some(rec), _) => State::Recording { since: rec.since },
            (None, Some(reason)) => State::Failed(reason.clone()),
            (None, None) => match indexer
                .as_ref()
                .and_then(|i| i.status.lock().unwrap().clone())
            {
                Some(progress) => State::Reading(progress),
                None => State::Waiting,
            },
        };
        // Reading text is heavy on the CPU, so it only runs while the game
        // is not.
        if let Some(indexer) = &indexer {
            indexer.allowed.store(game.is_none(), Ordering::Relaxed);
        }
        let key = match &state {
            State::Waiting => "waiting".to_owned(),
            State::Recording { .. } => "recording".to_owned(),
            State::Paused => "paused".to_owned(),
            State::Failed(reason) => format!("failed: {reason}"),
            State::Reading(progress) => progress.clone(),
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
    if let Some(rec) = recording {
        rec.stop(&rt, true);
    }
    if let Some(indexer) = indexer {
        indexer.join();
    }
    drop(commands_tx);
}
