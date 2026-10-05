# champollion
A rust suite of tools to decipher languages using video games

## android

A flashcard widget (Kotlin,
[Jetpack Glance](https://developer.android.com/develop/ui/compose/glance)) to
review the cards of the backend like in Anki: the word and the game sentence
it was read in; tapped, its translation, its meaning in that sentence and the
sentence's translation, with the Failed / Succeed buttons and ⚑ to flag a
wrong card: it leaves the reviews, and is kept in the backend (`flagged_at`)
to be fixed.

Each time a rating or a flag shows the next card, its word is spoken (the
backend's ElevenLabs recording, downloaded with the session, so it plays
offline too). The first card of a session, shown by a sync rather than a
tap, stays silent; 🔊 under the word plays it again, muted or not. 🔊
next to ⚑ mutes the words, 🔇 speaks them again.

It comes in two versions sharing one review session: one for the home screen,
and one for the Galaxy Z Flip's cover screen (Flex Window), which Samsung
requires to be a keyguard widget of at least 352×339 dp with a
`com.samsung.android.appwidget.provider` declaring `display="sub_screen"`.

Reviewing works offline. The phone downloads a session of cards due (up to
50: those already reviewed first, then new ones, each in random order) and
keeps them with the ratings and flags not sent yet (`Session`, in a
DataStore); "N left" counts the session's cards, those failed included.
Once the session is done, WorkManager sends its ratings and flags and
downloads the next one, as soon as the backend is reachable; it also syncs
every 30 minutes. A failed card goes back in the session at a random place
(after the next card, if there is one); the backend schedules the rest when
it gets the ratings.
Long-press the app icon → Reload cards to replace the session's cards by a
new download (e.g. after their definitions were made again).

Needs the Android SDK in `~/Android/Sdk` (or `ANDROID_HOME`) and a JDK 17+
for Gradle (Android Studio's own works: `org.gradle.java.home` in
`~/.gradle/gradle.properties`). With the phone plugged in and USB debugging on:

```sh
cd android
./gradlew installDebug
```

Then tap the Champollion app icon: it syncs, and asks the launcher to add
the widget if there is none (or long-press the home screen → Widgets →
Champollion → Flashcards). For the cover screen: Settings → Cover screen →
Widgets → Flashcards. The backend is only reachable with the phone's
WireGuard tunnel on.

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
- Recordings are split into 2-minute Matroska files, which stay readable if
  the daemon is killed, and whose text is read once they are finished:
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

The daemon reads the finished recordings with the
[Tesseract](https://github.com/tesseract-ocr/tesseract) OCR library
(through the `tesseract` crate), oldest first, while the game runs too: its
threads are `SCHED_IDLE`, so they only get the CPU time the game leaves,
and read one frame at a time while it runs. This takes about half a CPU
thread on a Ryzen 5 5600X.

```sh
sudo apt install libtesseract-dev libleptonica-dev tesseract-ocr-ces   # Czech; see --ocr-lang
```

It samples 1 frame per second, reads 2 frames at a time when the game is not
running (`--ocr-workers`;
each worker loads its own copy of the language model) and keeps words made of letters (2+
characters, OCR confidence >= 70). Output in
`~/.local/share/champollion/text/<game>/`:

- `<date>/<video>+MMmSSs.png`: a frame showing new text, with a `.json` next
  to it listing each word, its confidence, its box `[left, top, width,
  height]` and whether it is `new` (never seen in any earlier frame), and
  all the `lines` read, noise included, for Claude to find whole sentences
  in.

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

### Making flashcards

With `--backend-url` (`CHAMPOLLION_BACKEND_URL`, set in the systemd unit),
the daemon turns the new words of each `vocabulary.tsv` into flashcards every
minute, and sends them to the backend. It asks Claude, through the Claude
Code CLI in headless mode (`claude -p`, with its login, no tools nor
settings, thinking off), about 40 words at a time with the text of the
screens they were read on: the sentence each was read in, whole even across
lines and with OCR errors and button icons cleaned up (`Pomozmna nádvoří` →
`Pomoz na nádvoří`), their dictionary form (`králem` → `král`), part of speech and gender, English
translation, meaning in that sentence (slang included; of the dictionary
form, without the grammar of the form read) and the sentence's
translation, and whether to keep them at all (not English UI text, OCR
garbage, names or numbers). Sonnet takes about 20 seconds per batch and
understands the context much better than Haiku.

Files next to `vocabulary.tsv`:

- `cards.jsonl`: one card per word kept (`NewCard` in `api/`).
- `dropped.tsv`: the words not kept, with why.
- `generated.txt`, `uploaded.txt`: how many bytes of `vocabulary.tsv` were
  made into cards, and of `cards.jsonl` sent. Delete one to redo that step;
  the backend ignores what it already has.

### Time to study

With `--backend-url`, the daemon asks the backend every 30 seconds how many
cards are due (new ones included). Once 50 are (`--lock-at`,
`CHAMPOLLION_LOCK_AT`; 0 turns this off), it closes the game: a "Time to
study" notification shows, the game and everything Steam's `reaper`
started for it get SIGTERM, then SIGKILL 10 seconds later, without saving.
The game stays locked, closed again each time it is started, until fewer
than 10 cards are due (`--unlock-below`, `CHAMPOLLION_UNLOCK_BELOW`); the
tray shows how many are. The lock survives a restart of the daemon
(`~/.local/state/champollion/locked`). While the backend cannot be reached,
the game can be played.

A word counts a few minutes after it was on screen: its 2-minute file is
finished, then read, then made into a card within a minute or so.

Options: `--claude` (path of the `claude` executable), `--cards-model`
(default `sonnet`), `--no-cards` (only send the cards already made).

### Extracting frames later

```sh
# one PNG per second
gst-launch-1.0 filesrc location=22-24-25_000.mkv ! decodebin ! videoconvert \
  ! videorate ! video/x-raw,framerate=1/1 ! pngenc ! multifilesink location=frame_%05d.png
```

## backend

`champollion-backend` stores the flashcards made by the daemon in PostgreSQL
and schedules their review like Anki (SM-2), but with two ratings and gaps
starting at 10 minutes: a success multiplies the card's gap by its ease × 1.3
(10 min, 35 min, 2 h, 8 h, 1.3 days…), a failure brings it back to 10 minutes
and lowers the ease. There is one card per dictionary
form; each word read in a game is a sighting of its card (form, sentence,
translations), so `král` and `králem` make one card with two sightings. It runs on taillandier.io and only
listens on its WireGuard address, `10.0.0.1:8090`: the only devices that can
reach it are WireGuard peers (this PC at `10.0.0.50`, the phone at
`10.0.0.51`). Their tunnels only route `10.0.0.1` through the VPN.

| Request | Body / answer |
|---|---|
| `GET /health` | `ok` |
| `POST /cards` | `{"cards": [NewCard]}` → `{"added_cards": n, "added_sightings": n, "updated_sightings": n}`; known cards are kept, known sightings (same card, form and frame) take the sentence, its translation and the definition sent |
| `GET /cards/due?limit=100` | cards due now with their sightings, those already reviewed first, then new ones, each in random order |
| `GET /cards/due/count` | `{"due": n}`: how many cards are due now, new ones included |
| `GET /cards/{id}/audio` | the MP3 of the card's dictionary form, or 404 while it is not made |
| `POST /reviews` | `{"reviews": [Review]}` → `{"applied": n}`; a review id sent twice counts once |
| `POST /flags` | `{"flags": [Flag]}` → `{"applied": n}`; flagged cards are no longer due |

The JSON types are in `api/` (crate `champollion-api`, shared with the daemon).

The code is in layers, each calling only the next:

- `controllers/`: HTTP only. A handler reads the request, calls a service
  and answers.
- `services/`: what the backend does. A service holds the rules (e.g. how
  many due cards at most) and runs its queries in a transaction when they go
  together.
- `repositories/`: data access, one module per table, one function per
  query. They take a connection, so that a service can run several in one
  transaction.
- `schedule.rs`: when to show a card again, without database nor HTTP.
- `elevenlabs.rs`: the ElevenLabs text-to-speech API.

### Pronunciations

With an ElevenLabs API key (`ELEVENLABS_API_KEY`), a background task speaks
the dictionary form of every card that has no pronunciation yet, the
soonest due first, and stores the MP3 in the `pronunciations` table. It runs
when cards are added, and every 5 minutes after a failure (network, key,
quota). A word ElevenLabs refuses is skipped until the backend restarts.

The default model is `eleven_turbo_v2_5` (`CHAMPOLLION_ELEVENLABS_MODEL`),
which is told the card's language: otherwise a short word can be read as
English (`most`). The default voice is a premade one, `JBFqnCBsd6RMkjVDRZzb`
(`CHAMPOLLION_ELEVENLABS_VOICE`); a native voice from ElevenLabs' voice
library sounds better. Words already spoken keep their voice; to make them
again, delete their rows (`DELETE FROM pronunciations WHERE voice = '...'`)
and restart the backend.

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

The ElevenLabs API key stays out of the repository, in a file the unit
reads:

```sh
ssh -t taillandier.io 'sudo install -d /etc/champollion && sudo install -m 600 /dev/null /etc/champollion/backend.env && echo ELEVENLABS_API_KEY=... | sudo tee /etc/champollion/backend.env >/dev/null'
```

```sh
ssh taillandier.io journalctl -u champollion-backend -f   # needs sudo
```
