# champollion
A rust suite of tools to decipher languages using video games

## android

An Android app with a widget, so far showing "Hello, world!" (Kotlin,
[Jetpack Glance](https://developer.android.com/develop/ui/compose/glance)), in
two versions: one for the home screen, and one for the Galaxy Z Flip's cover
screen (Flex Window), which Samsung requires to be a keyguard widget of at
least 352×339 dp with a `com.samsung.android.appwidget.provider` declaring
`display="sub_screen"`.

Needs the Android SDK in `~/Android/Sdk` (or `ANDROID_HOME`) and a JDK 17+
for Gradle (Android Studio's own works: `org.gradle.java.home` in
`~/.gradle/gradle.properties`). With the phone plugged in and USB debugging on:

```sh
cd android
./gradlew installDebug
```

Then tap the Champollion app icon, which asks the launcher to add the widget
(or long-press the home screen → Widgets → Champollion → Hello). For the
cover screen: Settings → Cover screen → Widgets → Hello.

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

With `--backend-url` (`CHAMPOLLION_BACKEND_URL`, set in the systemd unit),
the daemon sends new lines of each `vocabulary.tsv` to the backend every
minute, 500 words per request. `uploaded.txt` next to it holds how many bytes
were sent; delete it to send everything again (the backend ignores words it
already has).

### Extracting frames later

```sh
# one PNG per second
gst-launch-1.0 filesrc location=22-24-25_000.mkv ! decodebin ! videoconvert \
  ! videorate ! video/x-raw,framerate=1/1 ! pngenc ! multifilesink location=frame_%05d.png
```

## backend

`champollion-backend` stores the words read by the daemon in PostgreSQL and
schedules their review like Anki (SM-2). It runs on taillandier.io and only
listens on its WireGuard address, `10.0.0.1:8090`: the only devices that can
reach it are WireGuard peers (this PC at `10.0.0.50`, the phone at
`10.0.0.51`). Their tunnels only route `10.0.0.1` through the VPN.

| Request | Body / answer |
|---|---|
| `GET /health` | `ok` |
| `POST /words` | `{"words": [NewWord]}` → `{"added": n}`; known words are ignored |
| `GET /cards/due?limit=100` | cards due now, most overdue first |
| `POST /reviews` | `{"reviews": [Review]}` → `{"applied": n}`; a review id sent twice counts once |

The JSON types are in `api/` (crate `champollion-api`, shared with the daemon).

### Deploy

The server and this PC both run Ubuntu 24.04 (glibc 2.39), so a local build
runs there:

```sh
cargo build --release -p champollion-backend
ssh taillandier.io mkdir -p champollion-deploy
scp target/release/champollion-backend backend/deploy/* taillandier.io:champollion-deploy/
ssh -t taillandier.io 'cd champollion-deploy && sudo ./setup-server.sh ./champollion-backend'
```

`setup-server.sh` creates the database and the `champollion` user (it
connects through the local socket, without password), installs the systemd
unit and opens port 8090 on `wg0` only. Arguments `NAME=PUBLIC_KEY@IP` also
add WireGuard peers. Migrations in `backend/migrations/` run at startup.

```sh
ssh taillandier.io journalctl -u champollion-backend -f   # needs sudo
```
