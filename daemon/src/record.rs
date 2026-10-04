//! Records the screen while the game runs: finds the game (`steam`), waits for
//! its window (`window`), asks the desktop for the screen (`screencast`) and
//! encodes it to video files (`recorder`).

mod recorder;
mod screencast;
pub mod steam;
pub mod window;

use std::path::Path;
use std::time::Instant;

use recorder::Recorder;
pub use recorder::Settings;
use screencast::ScreenCast;

/// A screencast being recorded.
pub struct Recording {
    cast: ScreenCast,
    recorder: Recorder,
    pub since: Instant,
}

impl Recording {
    pub fn start(
        rt: &tokio::runtime::Runtime,
        game: &steam::Game,
        output_dir: &Path,
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

    /// Why the recording pipeline stopped working, if it did.
    pub fn failure(&self) -> Option<String> {
        self.recorder.failure()
    }

    /// Ends the recording; `finish` waits for the current file to be
    /// finalized, which is pointless once the pipeline has failed.
    pub fn stop(self, rt: &tokio::runtime::Runtime, finish: bool) {
        if finish {
            self.recorder.stop();
        } else {
            drop(self.recorder);
        }
        rt.block_on(self.cast.close());
        tracing::info!("recording stopped");
    }
}
