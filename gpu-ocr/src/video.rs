//! Decodes recordings with the Jetson's hardware video decoder, keeping a
//! few frames per second.

use std::path::Path;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use image::RgbImage;

/// Hands `rate` frames per second of the video at `path` to `on_frame`,
/// with their time in the video, scaled to `size` (width, height) by the
/// hardware converter if given.
pub fn decode(
    path: &Path,
    rate: u32,
    size: Option<(u32, u32)>,
    on_frame: &mut dyn FnMut(f64, RgbImage) -> Result<(), String>,
) -> Result<(), String> {
    gst::init().map_err(|e| e.to_string())?;
    // parsebin finds the container (Matroska) and the H.265 parser. Frames
    // are dropped while still in the decoder's memory, before the costly
    // conversion to RGBA: nvvidconv has no packed RGB output.
    let size = size
        .map(|(w, h)| format!(",width={w},height={h}"))
        .unwrap_or_default();
    let pipeline = gst::parse::launch(&format!(
        "filesrc name=src ! parsebin ! nvv4l2decoder \
         ! videorate drop-only=true ! video/x-raw(memory:NVMM),framerate={rate}/1 \
         ! nvvidconv ! video/x-raw,format=RGBA{size} \
         ! appsink name=frames sync=false max-buffers=2"
    ))
    .map_err(|e| e.to_string())?
    .downcast::<gst::Pipeline>()
    .map_err(|_| "not a pipeline")?;
    pipeline
        .by_name("src")
        .ok_or("no filesrc")?
        .set_property("location", path);
    let sink = pipeline
        .by_name("frames")
        .and_then(|e| e.downcast::<AppSink>().ok())
        .ok_or("no appsink")?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| e.to_string())?;
    let result = pull(&pipeline, &sink, path, on_frame);
    let _ = pipeline.set_state(gst::State::Null);
    result
}

fn pull(
    pipeline: &gst::Pipeline,
    sink: &AppSink,
    path: &Path,
    on_frame: &mut dyn FnMut(f64, RgbImage) -> Result<(), String>,
) -> Result<(), String> {
    loop {
        let Ok(sample) = sink.pull_sample() else {
            // End of stream, or an error reported on the bus.
            return match pipeline
                .bus()
                .and_then(|b| b.pop_filtered(&[gst::MessageType::Error]))
            {
                Some(msg) => match msg.view() {
                    gst::MessageView::Error(err) => {
                        Err(format!("cannot decode {}: {}", path.display(), err.error()))
                    }
                    _ => Ok(()),
                },
                None => Ok(()),
            };
        };
        let (seconds, image) = to_rgb(&sample).ok_or("unexpected frame format")?;
        on_frame(seconds, image)?;
    }
}

/// The frame's time in the video and its pixels, without the alpha byte.
fn to_rgb(sample: &gst::Sample) -> Option<(f64, RgbImage)> {
    let caps = sample.caps()?;
    let s = caps.structure(0)?;
    let width = u32::try_from(s.get::<i32>("width").ok()?).ok()?;
    let height = u32::try_from(s.get::<i32>("height").ok()?).ok()?;
    let buffer = sample.buffer()?;
    let seconds = buffer.pts().map(|t| t.seconds_f64()).unwrap_or(0.0);
    let map = buffer.map_readable().ok()?;
    // Rows may be padded.
    let stride = map.len() / height as usize;
    let mut pixels = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height as usize {
        let row = map.get(y * stride..y * stride + width as usize * 4)?;
        for rgba in row.chunks_exact(4) {
            pixels.extend_from_slice(&rgba[..3]);
        }
    }
    Some((seconds, RgbImage::from_raw(width, height, pixels)?))
}
