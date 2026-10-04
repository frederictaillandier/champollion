# champollion
A rust suite of tools to decipher languages using video games

## daemon

`champollion-daemon` watches for Kingdom Come: Deliverance II (Steam app
1771300) and records the screen while it runs, for later post-processing
(cutting frames, finding text...).

- Capture goes through the desktop's screen-sharing portal and PipeWire. The
  first time, the desktop asks which screen to share: pick the game's screen.
  The choice is remembered (token in `~/.local/state/champollion/`), so later
  launches start recording without asking. Delete that file to choose again.
- Frames stay on the GPU and are encoded to H.265 by NVENC (falls back to
  x265 on the CPU without an NVIDIA GPU). Default: 15 fps, constant quality
  QP 20, which keeps small text sharp.
- Recordings are split into 10-minute Matroska files, which stay readable if
  the daemon is killed:
  `~/.local/share/champollion/recordings/1771300-kingdom-come-deliverance-ii/<date>/<time>_NNN.mkv`

Requires GStreamer 1.24 with the `pipewire`, `gl` and `nvcodec` plugins.

### Install as a user service

```sh
cargo install --path daemon
mkdir -p ~/.config/systemd/user
cp daemon/systemd/champollion-daemon.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now champollion-daemon
journalctl --user -u champollion-daemon -f   # logs
```

Options (`champollion-daemon --help`) can also be set as environment
variables in the unit: `CHAMPOLLION_APP_ID` (another Steam game),
`CHAMPOLLION_OUTPUT_DIR`, `CHAMPOLLION_FRAMERATE`, `CHAMPOLLION_QP`
(lower is sharper and bigger), `CHAMPOLLION_SEGMENT_MINUTES`.

### Reading the text

While the game is not running, the daemon reads the finished recordings
with the [Tesseract](https://github.com/tesseract-ocr/tesseract) OCR library (through the `tesseract` crate), oldest
first, at low priority. It pauses as soon as the game starts again.

```sh
sudo apt install libtesseract-dev libleptonica-dev tesseract-ocr-ces   # Czech; see --ocr-lang
```

It samples 1 frame per second, reads 2 frames at a time (`--ocr-workers`;
each worker loads its own copy of the language model) and keeps words made of letters (2+
characters, OCR confidence >= 70). Output in
`~/.local/share/champollion/text/<game>/`:

- `<date>/<video>+MMmSSs.png`: a frame showing new text, with a `.json` next
  to it listing each word, its confidence, its box `[left, top, width,
  height]` and whether it is `new` (never seen in any earlier frame).

  To filter out OCR noise (game scenery misread as letters), a word only
  counts once it is read in 2 of 3 consecutive frames, and a frame is saved
  when at least 2 words appeared (or 1 never seen before) that were not on
  screen in the previous 10 seconds. Screens sharing 80% of their words with
  an already saved one (a menu shown every session) are skipped.
- `skip-screens.txt`: screens not to read, one per line as words they show
  (a frame with 3 of a line's words is skipped). Created with Steam's window
  and, for KCD2, its main and pause menus; add lines for other screens.
- `vocabulary.tsv`: every word seen, with the frame where it first appeared.
- `processed.txt`: recordings already read (delete a line to read it again).

Options: `--no-ocr`, `--ocr-lang`, `--ocr-fps`, `--ocr-workers`,
`--ocr-min-confidence`, `--text-dir` (or the matching `CHAMPOLLION_*`
environment variables).

### Extracting frames later

```sh
# one PNG per second
gst-launch-1.0 filesrc location=22-24-25_000.mkv ! decodebin ! videoconvert \
  ! videorate ! video/x-raw,framerate=1/1 ! pngenc ! multifilesink location=frame_%05d.png
```
