# website

A static site explaining Champollion: the loop, the architecture, and every
part, with diagrams and charts of the measurements made so far. Plain HTML,
CSS and JavaScript, no build step and no dependencies (fonts come from
Google Fonts, with system fallbacks).

| Page | About |
|---|---|
| `index.html` | The loop, the architecture, what runs and what's next |
| `recording.html` | The daemon's recording pipeline, file sizes, the study lock |
| `reading.html` | Tesseract, the Jetson, GPU OCR explained, benchmarks, lessons |
| `game-text.html` | The game's localization text and matching OCR lines to it |
| `cards.html` | Card making with Claude, the review schedule, the backend API |
| `phone.html` | The Android flashcard widget |

Open `index.html` in a browser, or serve the folder:

```sh
python3 -m http.server -d website 8000   # http://localhost:8000
```

`assets/charts.js` draws the charts as SVG (bars, log-scale columns,
lines), with value labels, hover tooltips and a table view under each.
Colours are the tokens of `assets/style.css`, light and dark; the three
chart colours also stand for the machines (desktop blue, server orange,
Jetson aqua) in the diagrams.

The figures come from `docs/jetson-gpu-ocr.md` and the code; update both
when they change.

## Deploy

The site is only served to WireGuard peers, at
<http://10.0.0.1/champollion/>: nginx on the server answers for the host
`10.0.0.1` from `/var/www/champollion`, and refuses any address outside
`10.0.0.0/24` (`deploy/nginx-champollion.conf`). It isn't part of
taillandier.io's public site. The folder belongs to `fred`, so deploying
needs no sudo:

```sh
rsync -a --delete --chmod=D755,F644 --exclude README.md --exclude deploy/ \
    website/ taillandier.io:/var/www/champollion/
```

Set up once on the server, with the config copied to the home folder:

```sh
scp website/deploy/nginx-champollion.conf taillandier.io:champollion-nginx.conf
ssh -t taillandier.io '
  sudo install -d -o fred -g fred -m 755 /var/www/champollion &&
  sudo install -m 644 champollion-nginx.conf /etc/nginx/sites-available/champollion &&
  sudo ln -sf /etc/nginx/sites-available/champollion /etc/nginx/sites-enabled/champollion &&
  sudo nginx -t && sudo systemctl reload nginx'
```
