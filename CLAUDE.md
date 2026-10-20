# Terminal Rain (Asciiverse packaging)

rmaake1's terminal rain and lightning (Python, curses, MIT), packaged for
the Asciiverse catalog (kind `art`, id `terminal-rain-lightning`) as a Rust
port, `src/main.rs`, binary `terminal-rain`. Upstream history kept to
`cc3aa19` (2026-04-30); ours on branch `asciiverse`. `NOTICE` holds the
credits; keep them whole. Changes in `PATCHES.md`.

- Remotes: `upstream` is github.com/rmaake1/terminal-rain-lightning;
  `origin` is to be github.com/theasciiverse/terminal-rain-lightning
  (private).
- The port follows the script constant for constant; when upstream
  changes the script, port the change and note it in `PATCHES.md`.
- Sound: the MP3s in `terminal_rain_lightning_assets/sounds/` are built in
  with `include_bytes!` and played by `ffplay` when present, as upstream.
- Release: tag `v0.1.0+<date of upstream's last commit>-av.<n>` (first
  `v0.1.0+20260430-av.1`).
- av starts it as `terminal-rain -t`; q or Esc quits, t, s, m and v as
  upstream.
- Static build, in a copy of the tree: `docker run --rm --cpus 4 --memory
  8g -v "$PWD":/src -w /src rust:1-alpine sh -c 'apk add musl-dev && cargo
  build --release --locked -j2'`.
- Preview: `asciiverse/record-preview.sh target/release/terminal-rain
  asciiverse/terminal-rain-lightning.cast`.
- Session log: `RUST_LOG=debug terminal-rain 2>terminal-rain.log`.
