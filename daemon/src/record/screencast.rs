//! Screen capture through the XDG desktop portal.
//!
//! On Wayland an app cannot read the screen directly: it asks the desktop over
//! D-Bus, which shows a dialog to pick a screen and returns a PipeWire stream
//! that the recorder reads. The choice is remembered with a restore token, so
//! later recordings start without asking.

use std::fs;
use std::os::fd::OwnedFd;
use std::path::PathBuf;

use ashpd::desktop::PersistMode;
use ashpd::desktop::Session;
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::enumflags2::BitFlags;

/// A screen shared by the desktop through the ScreenCast portal.
///
/// The portal closes the PipeWire stream as soon as the session or its D-Bus
/// connection goes away, so this must outlive the recording.
pub struct ScreenCast {
    pub fd: OwnedFd,
    pub node_id: u32,
    pub size: Option<(i32, i32)>,
    proxy: Screencast,
    session: Session<Screencast>,
}

impl ScreenCast {
    /// Asks the desktop for a screen to record.
    ///
    /// The first time, the desktop shows a dialog to pick the screen. The
    /// returned restore token is saved so later calls reuse that choice
    /// without asking again (if the desktop supports persistence).
    pub async fn open() -> ashpd::Result<Self> {
        let token = load_token();
        let proxy = Screencast::new().await?;
        let session = proxy.create_session(Default::default()).await?;
        proxy
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(CursorMode::Hidden)
                    .set_sources(BitFlags::from(SourceType::Monitor))
                    .set_multiple(false)
                    .set_restore_token(token.as_deref())
                    .set_persist_mode(PersistMode::ExplicitlyRevoked),
            )
            .await?;
        let streams = proxy
            .start(&session, None, Default::default())
            .await?
            .response()?;

        match streams.restore_token() {
            Some(new) if Some(new) != token.as_deref() => save_token(new),
            Some(_) => {}
            None => tracing::warn!(
                "the desktop did not return a restore token, it will ask for the screen every time"
            ),
        }

        let stream = streams
            .streams()
            .first()
            .ok_or_else(|| ashpd::Error::NoResponse)?
            .to_owned();
        let fd = proxy
            .open_pipe_wire_remote(&session, Default::default())
            .await?;
        Ok(Self {
            fd,
            node_id: stream.pipe_wire_node_id(),
            size: stream.size(),
            proxy,
            session,
        })
    }

    pub async fn close(self) {
        if let Err(e) = self.session.close().await {
            tracing::debug!("could not close the screencast session: {e}");
        }
        drop(self.proxy);
    }
}

fn token_path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("champollion/screencast-token"))
}

fn load_token() -> Option<String> {
    let token = fs::read_to_string(token_path()?).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn save_token(token: &str) {
    let Some(path) = token_path() else { return };
    let result = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(&path, token));
    if let Err(e) = result {
        tracing::warn!(
            "could not save the screencast restore token to {}: {e}",
            path.display()
        );
    }
}
