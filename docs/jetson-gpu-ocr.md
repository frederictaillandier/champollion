# Reading game text on the Jetson's GPU

Session of 14–15 October 2026. It started with checking the study lock after
a Kingdom Come: Deliverance II session, and ended with a Rust program that
reads game frames on the Jetson's GPU faster than real time.

## 1. The study lock on 14 October

The daemon closed the game when 50 cards were due (`CHAMPOLLION_LOCK_AT`):

| Time  | Event |
|-------|-------|
| 22:20 | 9 cards due, under `CHAMPOLLION_UNLOCK_BELOW` (10): game unlocked |
| 22:22 | Game started, recording began |
| 22:26 | 2 cards made from the recording: *jízda* and *bezstarostný* |
| 22:34 | 50 cards due: game locked, notification shown, SIGTERM sent |
| 22:34 | Game quit on its own 2 s later; no SIGKILL needed |

`~/.local/state/champollion/locked` held `50`, so a daemon restart keeps the
lock. Everything worked as designed.

A GStreamer warning shows up each time recording starts and is harmless:
`gst_mini_object_unref: assertion 'mini_object != NULL' failed`.

### Why *jízda* and *bezstarostný* were only found now

The two words were on screen once before, on 6 October at 12:09, over busy
scenery. Tesseract read the line as `Dezstarostná jízdy` with confidence
under 70 (`CHAMPOLLION_OCR_MIN_CONFIDENCE`), so both were dropped as noise. On
14 October the same words showed up in the save list ("Bezstarostná jízda,
06/10/2026 12:22", the name of a save made that day). On that plain
background Tesseract read them correctly.

## 2. The desktop can't keep up while you play

Text extraction finished 9 minutes after the game closed. Timings for the six
2-minute segments of that session:

- While playing, the indexer uses a single reader (by design, to leave the
  CPU to the game). Segment `_000` took 12 minutes, about 1.5 s per frame at
  `CHAMPOLLION_OCR_FPS=4`.
- After the game closed, each segment took 1.5–2.5 minutes with the default 2
  workers.
- Tesseract alone takes 0.19 s per frame on one Ryzen 5600X core, and decoding
  a segment takes 7.5 s. The desktop has plenty of capacity when the game is
  closed. The daemon runs at nice 9, and the system showed 20% I/O wait at
  the time, which may explain the rest. Not investigated further.

Raising `CHAMPOLLION_OCR_WORKERS` (2 by default, on a 12-thread CPU) would
make catching up after a session faster. It doesn't help during play.

## 3. The Jetson

Found at **192.168.0.43**, hostname `tegra-ubuntu` (`tegra-ubuntu.local` over
mDNS). The network sweep missed its SSH port at first because it answers
slowly.

- Jetson AGX Xavier: 8 Carmel ARM cores, 30 GB RAM, 28 GB eMMC
- Ubuntu 20.04, L4T R35.6.5 (JetPack 5.1.6), kernel 5.10
- Power mode `MODE_30W_ALL`: CPU capped at 1.19 of 2.27 GHz. `sudo nvpmodel
  -m 0` (MAXN) should be faster, not measured yet.
- No CUDA, cuDNN or TensorRT installed, no `g++`, no pip, no WireGuard in the
  kernel (`CONFIG_WIREGUARD` is not set; `wireguard-go` would be needed to
  reach the backend at 10.0.0.1)
- Hardware H.265 decoding is available (`nvv4l2decoder`)
- Its user `fred` can SSH in with the desktop's key; sudo asks for a
  password

The disk was 95% full because of Ollama models. `gpt-oss:20b` (13 GB) and
`qwen2.5-coder:7b` (4.7 GB) were removed; 15 GB is now free. Ollama itself
is still installed and needs sudo to remove:

```
sudo systemctl disable --now ollama
sudo rm /usr/local/bin/ollama /etc/systemd/system/ollama.service
sudo rm -r /usr/local/lib/ollama /usr/share/ollama
sudo userdel ollama; sudo groupdel ollama
```

## 4. Plan for moving text reading off the desktop

Agreed so far, not built yet:

- **Videos go over the LAN, not through the server.** The Jetson copies
  segments from the desktop (rsync over SSH with a restricted, read-only
  key), and deletes each segment once it is read. A session makes about
  100 MB per 2 minutes, so 3 GB per hour of play. Sending that through the
  server would cost the home upload link and server disk for nothing.
- **The backend coordinates.** A `recordings` table is the job queue: the
  daemon registers finished segments and the Jetson claims them with
  `FOR UPDATE SKIP LOCKED`. A `words` table replaces `vocabulary.tsv`. Card
  generation stays on the desktop, since it uses the local `claude` program,
  and reads pending words from the backend.
- **No sudo at run time.** On the Jetson, a `champollion` system user in the
  `video` group runs a hardened `champollion-reader.service`. A one-time
  `setup-jetson.sh`, like `backend/deploy/setup-server.sh`, installs it. The
  Jetson joins WireGuard as a new peer through `setup-server.sh`.
- **The Jetson's disk** (eMMC) can't keep videos; add an NVMe drive if that
  changes.

## 5. Benchmarks

All on the same 120 frames: one per second from segment
`2026-10-14/22-22-39_000.mkv` (menus, news, the save list, gameplay).

| Reader | Time per frame | Frames/s | Confident words* |
|---|---|---|---|
| Desktop Tesseract 5.3.4, 1 core | 0.19 s | 5.4 | 317 |
| Desktop Tesseract while playing | ~1.5 s | ~0.7 | — |
| Jetson Tesseract 5.5.3, 1 core | 1.78 s | 0.6 | — |
| Jetson Tesseract, 8 cores | 0.32 s | 3.1 | 298 |
| **Jetson GPU, PP-OCRv5 mobile, FP32** | **0.16 s** | **~6** | **282** (230 shared with Tesseract) |

\* Distinct words of 2+ letters with confidence ≥ 70.

Real time is 4 frames per second, so 0.25 s per frame at most.

The Jetson's Tesseract was installed with micromamba from conda-forge, using
the desktop's `ces.traineddata`. It read 312 of the same confident words as
the desktop, so the setups match.

Quality notes for the GPU reader:

- It read "Bezstarostná jízda" at 100 and 98, and every item of the main
  menu, including the gothic game logo.
- It sometimes drops Czech accents: *ceské* (české), *zemé* (země), *stoji*
  (stojí). Tesseract makes errors of its own (*deliúerance*, *vlew*).
- It misses very small text (the English legal lines on the title screen),
  since frames are scaled to 1280 wide for detection.

## 6. How GPU OCR works here (no training involved)

An illustrated explainer was published as a Claude artifact during the
session: https://claude.ai/artifact/Q2MYWEApj8g8kr7GeotQ9f

- **The models are already trained.** PaddleOCR's PP-OCRv5 is two models:
  one finds the lines of text in a frame, the other reads each line. The
  Latin recognizer covers Czech, Č Ě Ř Ů included. No annotated data is
  needed to use them.
- **ONNX** is just the file format the models come in.
- **TensorRT** is NVIDIA's engine for running any model fast on their GPUs.
  It knows nothing about text. `trtexec` turns each ONNX file into an engine
  tuned for this exact GPU.
- **The Rust program** does everything around the models: scaling frames,
  turning the detector's probability map into boxes, cropping lines,
  decoding the recognizer's output (greedy CTC) into words with a
  confidence each, on the same 0–100 scale as Tesseract.

Fine-tuning would only be needed if the game's fonts were misread too often.
That would take roughly 2,000–5,000 labelled text lines for the recognizer.
After four sessions the daemon had already saved 391 distinct screens with
about 4,500 text sightings. Not planned for now.

## 7. The `gpu-ocr` crate

```
gpu-ocr/
  build.rs           compiles the C++ wrapper, links TensorRT and CUDA
  src/tensorrt.cpp   C interface: open an engine, set input shape, run
  src/engine.rs      safe Rust wrapper over it
  src/detect.rs      frame → text boxes (PP-OCRv5 detection, DB postprocess)
  src/recognize.rs   text boxes → words (PP-OCRv5 Latin recognition, CTC)
  src/main.rs        benchmark: reads PNG frames, prints words and timings
```

It is a workspace member but not in `default-members`, so `cargo build` on
the desktop does not need TensorRT.

Usage on the Jetson:

```
. ~/ocr/env.sh
cd ~/champollion && cargo build --release -p gpu-ocr
target/release/champollion-gpu-ocr [--det-width 1280] [--det det.engine] [--batch 16] \
    ~/ocr/models FRAME.png...
```

It prints `frame, x, y, confidence, word` per word on stdout, and a timing
summary on stderr.

### Choices and what was learnt

- **Mobile detector in FP32.** The large "server" detector built in FP16
  lost whole menu items (its probability map stayed dark), while FP32 read
  them but took 458 ms per frame. The mobile detector reads as well, at
  52 ms in FP32 and 40 ms in FP16. Results were the same in both precisions
  for the mobile one; FP32 is the default to be safe.
- **Fixed detection size, 1280×736.** An engine built for any size up to
  2560×1440 used 21 GB of memory after 29 minutes and never finished.
- **Preprocessing matches PaddleOCR's** (from the model's `inference.yml`):
  BGR channel order, ImageNet mean/std for detection, (x/127.5 − 1) for
  recognition, lines scaled to 48 px high and padded with zeros.
- **`fast_image_resize` instead of `image`'s resize.** Scaling a frame took
  140 ms with `image::imageops` on the ARM cores, more than the GPU work.
- **RPATH, not RUNPATH** (`-Wl,--disable-new-dtags`), so TensorRT's own
  dependencies (e.g. `libcudla.so.1`) are found without `LD_LIBRARY_PATH`.
- Per frame now: detection 99 ms (52 ms on the GPU), recognition 63 ms for
  about 9 lines. The program uses about one CPU core.
- **Two threads saturate the GPU.** With one thread, the GPU waits while the
  CPU scales frames and decodes words. Measured over 480 frames, with the
  17.5 s engine loading of each process taken out: 1 process ~182 ms per
  frame overall, 2 processes ~131 ms, 3 processes ~130 ms. So the real
  reader should use one hardware-decoding thread and two model threads, each
  with its own TensorRT execution context sharing the loaded engines (so the
  17.5 s startup happens once). The GPU is then the limit, which MAXN power
  mode would raise.
- Tesseract's Jetson figure (0.32 s per frame) already used all 8 cores, one
  process per core.

## 8. What is on the Jetson

| Path | What |
|---|---|
| `~/gpu/root` | NVIDIA packages unpacked with `dpkg -x` (no sudo): CUDA 11.4 runtime, cuBLAS, cuFFT, cuRAND, cuDLA, cuDNN 8.6, TensorRT 8.5 with headers and `trtexec` |
| `~/ocr/models` | `det-mobile.onnx`, `det.onnx` (server), `rec.onnx`, `dict.txt`, and the engines: `det.engine` (= `det-mobile-fp32.engine`), `det-mobile-fp16.engine`, `det-server-fp16.engine` (broken, see above), `det-server-fp32.engine`, `rec.engine` |
| `~/ocr/env.sh` | Build environment: Cargo, `CHAMPOLLION_GPU_ROOT`, conda's `g++` |
| `~/ocr/build-engines.sh` | First engine build script (its dynamic detection shape didn't work) |
| `~/champollion` | Copy of this workspace (rsync, without `.git`) |
| `~/.cargo`, `~/.rustup` | Rust 1.99 via rustup |
| `~/.local/bin/micromamba`, `~/bench/mamba` | Environments: `tess` (Tesseract 5.5.3), `cxx` (GCC 10), `gpu` (unused) |
| `~/bench` | Benchmark frames, lists and outputs |

Engines are built with, for example:

```
R=~/gpu/root
export LD_LIBRARY_PATH=$R/usr/lib/aarch64-linux-gnu:$R/usr/local/cuda-11.4/targets/aarch64-linux/lib
$R/usr/src/tensorrt/bin/trtexec --onnx=det-mobile.onnx --saveEngine=det-mobile-fp32.engine \
    --shapes=x:1x3x736x1280 --memPoolSize=workspace:2048
$R/usr/src/tensorrt/bin/trtexec --onnx=rec.onnx --fp16 --saveEngine=rec.engine \
    --minShapes=x:1x3x48x16 --optShapes=x:16x3x48x320 --maxShapes=x:32x3x48x1280
```

Model sources:

- Detection: [PaddlePaddle/PP-OCRv5_mobile_det_onnx](https://huggingface.co/PaddlePaddle/PP-OCRv5_mobile_det_onnx)
- Recognition and dictionary: [monkt/paddleocr-onnx](https://huggingface.co/monkt/paddleocr-onnx),
  `languages/latin`. PaddlePaddle also publishes
  [latin_PP-OCRv5_mobile_rec_onnx](https://huggingface.co/PaddlePaddle/latin_PP-OCRv5_mobile_rec_onnx),
  not tried yet.

## 9. Next steps

1. Read video directly with the Jetson's hardware decoder instead of PNG
   files. Scaling for detection could then happen in hardware too
   (`nvvidconv`).
2. Backend: the `recordings` job queue and the `words` table.
3. Pull segments from the desktop over the LAN.
4. `setup-jetson.sh` and the `champollion` service user; WireGuard peer.
5. Switch the daemon over, keeping local Tesseract as a fallback for a while.

Also worth trying: `sudo nvpmodel -m 0` for speed, and the official
PaddlePaddle Latin recognizer to see whether it keeps Czech accents better.

## 10. Follow-up: other recognizers and the accent errors

- **PaddlePaddle's official `latin_PP-OCRv5_mobile_rec_onnx`** is a different
  file from the community conversion used so far: 836 characters against
  502. It was no better: 281 confident words (226 shared with Tesseract)
  against 282 (230), with the same kind of accent losses (*jizda*,
  *stoleti*), some merged words (*playedkingdom*), and slower reading (78 ms
  per frame against 63 ms). Its dictionary must be read with a YAML parser:
  some characters are quoted there (`'-'`, `''''`). Kept in
  `~/ocr/models-official` on the Jetson; the community model stays the
  default.
- **Accent errors are intermittent.** All ten checked words (*jízda*,
  *století*, *země*, *království*…) are also read correctly in other frames.
  Adding a vertical margin to the line crops (to keep accents in) did not
  help: 0% margin is best (10% to 30% only add noise words). Voting across
  frames, or matching lines against the game's own text, would fix these.
- **The game's own Czech text** is in
  `KingdomComeDeliverance2/Localization/Czech_xml.pak` (a zip): 177,930
  dialogue lines in `text_ui_dialog.xml`, plus quests, menus, items,
  tutorials, each line with an ID. "Bezstarostná jízda" is in
  `text_ui_quest.xml`. `English_xml.pak` has the same files in English.
  The idea: keep OCR deciding what was seen and when, but replace each line
  it reads by the matching line of the files, so words come out exact. The
  game's `kcd.log` records nothing about dialogue or quests.
- **Decision:** the game keeps the desktop's whole CPU and GPU; OCR runs on
  the Jetson.

## 11. Reading recordings directly

The program now takes recordings as well as PNG files:

```
. ~/ocr/env.sh
champollion-gpu-ocr [--fps 4] [--workers 2] ~/ocr/models RECORDING...
```

- One thread decodes with the hardware decoder:
  `filesrc ! parsebin ! nvv4l2decoder ! videorate drop-only=true !
  video/x-raw(memory:NVMM),framerate=4/1 ! nvvidconv ! video/x-raw,format=RGBA
  ! appsink`. Frames are dropped while still in the decoder's memory, before
  conversion. `nvvidconv` has no packed RGB output, so the alpha byte is
  dropped in Rust. Decoding alone takes 8.5 s for a 2-minute segment.
- `--workers` threads each load their own engines and read frames.
- `h265parse` is in `gstreamer1.0-plugins-bad`, which isn't installed. Its
  plugin was unpacked to `~/gpu/gst`; `env.sh` sets `GST_PLUGIN_PATH`.
- **The recordings were MP4 files, though named `.mkv`** (fixed on
  16 October, see section 14). The recorder set `matroskamux` as
  `splitmuxsink`'s muxer, but the files came out as QuickTime/MP4
  (`gst-discoverer-1.0`). `decodebin` and `parsebin` coped; `matroskademux`
  didn't.

Segment `22-22-39_000` (2 minutes, 480 frames at 4 per second, in colour):

| Workers | Time | Per frame overall | Frames/s |
|---|---|---|---|
| 1 | 83 s | 174 ms | 5.8 |
| 2 | 61 s | 127 ms | 7.9 |
| 3 | 57 s | 119 ms | 8.4 |

With 2 workers a 2-minute segment is read in about 1 minute, twice as fast
as real time, still in the 30 W power mode. "Bezstarostná jízda" was read
at 100 and 99 at 0:54.

### More workers, and detecting at 960 wide

More than 2–3 workers doesn't help: 4 workers took 57 s like 3. The GPU is
saturated, so each worker's frames just take longer (detection 106 ms with
1 worker, 270 ms with 4). The 30 W mode already allows the GPU 1,339 of its
1,377 MHz, so MAXN would barely speed up the GPU reader (it mostly raises
the CPU clock).

Detecting on frames scaled to 960×544 (PaddleOCR's default size) instead of
1280×736, with the line crops still taken from the full-size frame. Engine
`det-mobile-fp32-960.engine`, run with `--det-width 960 --det
det-mobile-fp32-960.engine`:

| Detection size | GPU time | 2 workers | Frames/s | Confident words | With Czech accents |
|---|---|---|---|---|---|
| 1280×736 | 52 ms | 61 s | 7.9 | 304 | 94 |
| 960×544 | 30 ms | 43 s | 11.1 | 250 | 92 |

The words lost at 960 are mostly small English text (Steam overlay, the news
panel: *anniversary*, *downloads*, *wishlist*…). Czech words are nearly all
kept, with a few more misreadings (*přběhu*, *pkně*). This segment is mostly
menus; gameplay subtitles still need checking.

## 12. Matching OCR lines against the game's text

New crate `game-text` (binary `champollion-game-text`, a default workspace
member: no GPU needed).

```
champollion-game-text import ~/.steam/steam/steamapps/common/KingdomComeDeliverance2/Localization \
    ~/.local/share/champollion/game-text/1771300.sqlite
champollion-game-text match [--min-similarity 0.8] DB OCR.tsv...
```

- **`import`** reads `Czech_xml.pak` and `English_xml.pak` (zip archives)
  into SQLite: table `lines(file, id, czech, czech_alt, english)`, entities
  decoded and markup (`<br/>`, `<paragraph>`, `<accent>`…) removed. 204,083
  lines in 1.3 s, 57 MB, every one with its English. The two Czech columns
  differ in 71,000 dialogue rows (the second has spoken forms like *nevim*);
  both are kept and matched, since it isn't known yet which the subtitles
  show.
- **`match`** puts the OCR words of each screen line back together (same
  frame and position), and for each different line finds the game line it
  comes from. Texts are compared without accents, case or punctuation.
  Candidates are the game texts sharing the most three-letter sequences with
  the OCR line; the best is the one with a stretch closest in edit distance
  (Sellers' algorithm). Two directions are tried: the screen line as part of
  a game line (wrapped subtitles), and a whole game line within a longer
  screen line (a save's quest name followed by its playtime and date). Lines
  under 8 letters only match a whole, equal game text. Output per line: how
  many frames read it, similarity, OCR text, the game's exact text for it,
  `file:id`, and the whole game line in Czech and English. Loading the index
  takes 1 s; matching 273 lines another second.

On the 2-minute test segment, the matches fix the OCR's accent errors:

| OCR read | Game text |
|---|---|
| Král Václav IV. byl uvézněn | Král Václav IV. byl uvězněn |
| a svym bratrem Zikmundem, králem uherskym | a svým bratrem Zikmundem, králem Uherským |
| je české království zmitáno válkou | je České království zmítáno válkou |
| Bezstarostná jízda, 0h, 06/10/2026 12:22 | Bezstarostná jízda (quest `qname_prepadeni`, "Easy Riders") |

73 of 273 different screen lines matched at 1280-wide detection, 67 of 209
at 960. The unmatched lines are text that isn't in the game files: the news
panel, the Deep Silver account promotion, Steam's overlay. Both widths found
the same 31 Czech game lines; the 8 found only at 1280 are English overlay
text, two of them wrong matches (*Applications* → *Locations*). Lines whose
Czech equals their English (untranslated UI words like *Home*, *Activity*)
could be skipped.

Known limit: a short wrapped line can match a shorter game text containing
the same words instead of its real source (*nepřátele…*, the last line of
the account promotion, matched the dog command *Útočí na nepřátele*). The
other lines of the same frame could disambiguate.

## 13. Recording at 1080p and 8 frames per second

Simulated with `champollion-gpu-ocr --size WxH` (frames scaled by the
Jetson's hardware converter while decoding), reading 8 frames per second
with 2 workers and 960-wide detection, on the 2-minute test segment:

| Frame size | Frames/s read (8 needed) | Czech game lines found after matching |
|---|---|---|
| 2560×1440 | 10.6 | same set |
| 1920×1080 | 12.4 | same set |
| 1280×720 | 11.9 | same set |

The differences between sizes were false matches on junk lines or English
overlay text. 720p isn't faster than 1080p (the GPU is the limit) and
misreads small text more before matching (*Pokražovat*), so 1080p.

Daemon changes:

- `CHAMPOLLION_FRAMERATE` now defaults to 8 (was 15).
- New `CHAMPOLLION_MAX_HEIGHT`, default 1080: frames taller than that are
  scaled down keeping the screen's proportions (2560×1440 → 1920×1080), on
  the GPU (`glcolorscale`) before NVENC, or with `videoscale` on the CPU
  path. 0 keeps the screen's size.
- The recorder sets `splitmuxsink`'s `muxer-factory=matroskamux` instead of
  a `muxer` element, which was ignored: files really are Matroska now.
  Checked with a test pipeline (GL scaling + NVENC + splitmuxsink): Matroska
  at 1920×1080, 8 frames per second.

Installed with `cargo install --path daemon --locked` and the service
restarted on 15 October. The desktop's Tesseract reading now gets 1080p
frames until the Jetson takes over; its effect on Tesseract wasn't measured.

## 14. The Jetson connected to the daemon (16 October)

Simpler than the plan of section 4: no job queue, no WireGuard on the
Jetson, no service there.

- When a recording is finished, the daemon runs
  `ssh 192.168.0.43 '~/ocr/read-recording 8' < RECORDING` (setting
  `CHAMPOLLION_OCR_REMOTE`, `daemon/src/extract/remote.rs`). The Jetson writes
  the recording to a temporary file (so the demuxer can seek), reads it on
  the GPU, deletes it, and prints one
  JSON object per frame, in order: its time, and its text in Tesseract's
  TSV format.
- The daemon parses that TSV with the same code as its own Tesseract's, so
  word tracking, the vocabulary, saved screens and card making are
  unchanged. No Tesseract runs on the desktop any more.
- The Jetson's reader gives each word its own box (from the positions its
  letters were read at) and groups lines into paragraphs by geometry, so a
  subtitle wrapped over two lines stays one sentence.
- Only frames showing new text are saved as images; the desktop decodes
  those alone by seeking into the recording (about 2 s each).
- If the Jetson can't be reached (ssh exits with 255), the recording isn't
  marked as read: the daemon logs it once and tries again every minute.
- NVIDIA's decoder library prints "Opening in BLOCKING MODE" on the
  standard output; the daemon skips lines that aren't JSON.

Settings in the user service: `CHAMPOLLION_OCR_REMOTE=192.168.0.43`,
`CHAMPOLLION_OCR_FPS=8` (every recorded frame). On the Jetson,
`~/ocr/read-recording` is `gpu-ocr/jetson/read-recording`.

First run, a 2-minute recording of 16 October: read in 65 s, including
about 17 s of engine loading. It found a word Tesseract had missed, read as
"A pořádn!" (*pořádně* with its last letter lost): the game-text matching
isn't wired into the daemon yet.

Also fixed: `splitmuxsink` creates its muxer when linked, while the
pipeline description is parsed, so `muxer-factory` set afterwards was
ignored and recordings stayed MP4. It's now in the description.

### Correcting with the game's text, on the Jetson

The reader now corrects each line before sending it (`gpu-ocr/src/correct.rs`,
`--game-text`): lines found in the game's text with at least 80% of their
characters right are replaced by the exact text, words spread along the
line's box when OCR merged or split some; lines not in the game stay as read.
A per-line cache makes the many frames of one subtitle free.
`~/ocr/read-recording` passes `~/ocr/game-text.sqlite` (a copy of the
desktop's `~/.local/share/champollion/game-text/1771300.sqlite`) when it
exists. To refresh it after a game update:

```sh
champollion-game-text import ~/.steam/steam/steamapps/common/KingdomComeDeliverance2/Localization \
    ~/.local/share/champollion/game-text/1771300.sqlite
scp ~/.local/share/champollion/game-text/1771300.sqlite 192.168.0.43:ocr/game-text.sqlite
```

It was first wired into the desktop daemon, then moved to the Jetson so the
game keeps the whole PC: the matcher's index takes 133 MB (down from 360 MB
after keeping only each line's original text and three-letter index, and
rebuilding the rest for the 40 candidates compared) and some CPU per line.

On the 70-second recording of 16 October, read again with correction:
every Czech word came out right (*české, zmítáno, Václav IV, uvězněn, svým,
Uherským, země, poslední, pomazání, důležitých*), with whole sentences
("Král Václav IV. byl uvězněn odbojnou šlechtou a svým bratrem Zikmundem,
králem Uherským."). Only text outside the game stayed as read (logos,
"szs"). Before the correction, that recording had added 12 misspelled
words, 10 of which reached the backend as sightings with wrong forms.

## 15. Tesseract removed (17 October)

The daemon no longer reads text itself: the local reading path (decoding,
Tesseract worker threads) and the `tesseract` crate are gone, so building it
no longer needs `libtesseract-dev`. `CHAMPOLLION_OCR_REMOTE` is required to
read text; without it the daemon only records. `CHAMPOLLION_OCR_FPS` now
defaults to 8, and the card language is `CHAMPOLLION_LANG` (was
`CHAMPOLLION_OCR_LANG`). The daemon keeps the Tesseract TSV parser, which is
how it reads the Jetson's results.

The Jetson was cleaned of the benchmarks: test frames and outputs, the
Tesseract environment, micromamba's package cache, unused engines (server
detector, 1280-wide and FP16 mobile ones), the server detector's ONNX file
and the official recognizer. What's left:

| Path | What |
|---|---|
| `~/ocr/read-recording`, `~/ocr/env.sh` | The command the daemon runs; the build environment |
| `~/ocr/models` | `det.engine` (mobile detector, FP32, 960×544), `rec.engine`, `dict.txt`, and the two ONNX files to rebuild them |
| `~/ocr/game-text.sqlite` | The game's text, for correcting lines |
| `~/gpu/root`, `~/gpu/gst` | NVIDIA's libraries (loaded at run time) and the `h265parse` plugin |
| `~/bench/mamba/envs/cxx`, `~/.cargo`, `~/.rustup`, `~/champollion` | The C++ compiler, Rust and the sources, to rebuild the reader |

`champollion-gpu-ocr` now defaults to detection at 960 wide with
`det.engine`, so `read-recording` needs no engine options. Ollama is still
installed on the Jetson (its models were removed); removing it needs sudo.
