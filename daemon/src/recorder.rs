use std::os::fd::AsRawFd;
use std::path::Path;

use gstreamer as gst;
use gstreamer::prelude::*;

use crate::screencast::ScreenCast;

pub struct Settings {
    pub framerate: u32,
    /// Constant quantizer: lower is sharper and bigger (0-51).
    pub qp: u32,
    /// Length of each file, so a crash only loses the current one.
    pub segment_secs: u64,
}

/// Records a PipeWire screencast to segmented H.265 Matroska files.
pub struct Recorder {
    pipeline: gst::Pipeline,
}

impl Recorder {
    /// Starts recording; files are named `<prefix>_000.mkv`, `<prefix>_001.mkv`...
    pub fn start(
        cast: &ScreenCast,
        prefix: &Path,
        settings: &Settings,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if let Some(dir) = prefix.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let description = pipeline_description(cast, settings);
        tracing::debug!("pipeline: {description}");
        let pipeline = gst::parse::launch(&description)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "not a pipeline")?;

        let sink = pipeline.by_name("sink").ok_or("splitmuxsink missing")?;
        // Matroska stays readable up to the last frame if the process dies.
        sink.set_property("muxer", gst::ElementFactory::make("matroskamux").build()?);
        sink.set_property("location", format!("{}_%03d.mkv", prefix.display()));
        sink.set_property(
            "max-size-time",
            settings.segment_secs * gst::ClockTime::SECOND.nseconds(),
        );

        pipeline.set_state(gst::State::Playing)?;
        Ok(Self { pipeline })
    }

    /// Returns why the recording stopped on its own, if it did.
    pub fn failure(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        while let Some(msg) = bus.pop() {
            match msg.view() {
                gst::MessageView::Error(err) => {
                    return Some(format!("{} ({:?})", err.error(), err.debug()));
                }
                gst::MessageView::Eos(_) => return Some("the screencast stream ended".into()),
                gst::MessageView::Warning(w) => tracing::warn!("gstreamer: {}", w.error()),
                _ => {}
            }
        }
        None
    }

    /// Finishes the current file properly, then tears the pipeline down.
    pub fn stop(self) {
        self.pipeline.send_event(gst::event::Eos::new());
        if let Some(bus) = self.pipeline.bus() {
            let done = bus.timed_pop_filtered(
                gst::ClockTime::from_seconds(10),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            );
            if done.is_none() {
                tracing::warn!("timed out finishing the recording, the last file may be truncated");
            }
        }
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn pipeline_description(cast: &ScreenCast, settings: &Settings) -> String {
    let Settings { framerate, qp, .. } = *settings;
    let keyframe_interval = framerate * 2;
    // The compositor sends frames at the monitor's refresh rate, and only
    // when the screen changes. videorate drops to a constant framerate first,
    // so later elements only process the frames that are kept.
    let source = format!(
        "pipewiresrc fd={fd} path={node} do-timestamp=true keepalive-time=1000 \
         ! videorate \
         ! video/x-raw(memory:DMABuf),framerate={framerate}/1;video/x-raw,framerate={framerate}/1",
        fd = cast.fd.as_raw_fd(),
        node = cast.node_id,
    );
    let sink = "h265parse ! splitmuxsink name=sink";

    if gst::ElementFactory::find("nvcudah265enc").is_some() {
        // Frames stay in GPU memory: the compositor's DMA-BUF is imported into
        // OpenGL, converted to NV12 by a shader and handed to NVENC, which
        // runs on its own chip of the GPU.
        format!(
            "{source} ! glupload ! glcolorconvert ! video/x-raw(memory:GLMemory),format=NV12 \
             ! nvcudah265enc rate-control=cqp qp-i={qp} qp-p={qp} qp-b={qp} preset=p4 \
               tune=high-quality gop-size={keyframe_interval} \
             ! {sink}"
        )
    } else {
        tracing::warn!("nvcudah265enc not available, encoding on the CPU with x265enc");
        format!(
            "{source} ! videoconvert \
             ! x265enc speed-preset=superfast qp={qp} key-int-max={keyframe_interval} \
             ! {sink}"
        )
    }
}
