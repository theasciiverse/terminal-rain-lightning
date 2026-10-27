# Changes from upstream

This is the Asciiverse packaging of rmaake1's Terminal Rain
(github.com/rmaake1/terminal-rain-lightning, the `upstream` remote), rain
and lightning in the terminal, in Python and curses. Its history is kept
whole, up to upstream's last commit `cc3aa19` (2026-04-30); everything
after it is ours, on branch `asciiverse`. MIT License (`LICENSE`), like
the original; `NOTICE` names everyone who made it.

## The Rust port (`src/main.rs`)

`terminal_rain_lightning.py` stays as it is; the catalog ships
`terminal-rain`, the same program ported to one Rust file (crossterm), so
it is one binary on Linux, macOS and Windows and needs no Python or
curses. The binary keeps upstream's command name.

Kept from the script, constant for constant: the drops (`|`, `.`, `` ` ``
from the top row, 0.3 to 0.6 rows per update when calm, 0.3 to 1.0 in a
storm, how many and how often), dim rain when calm and bold in a storm,
the bolts (start in the middle half of the top fifth, up to three at once,
growth with branches and forks, `#` then `+` then `*` over 0.8 seconds),
lightning drawn under the rain, the three speeds (15, 45 and 100 ms per
update), the keys (`t` thunderstorm, `s` speed, `m` sound, `v` volume, `q`
or Esc quits, upper case too), every option with the same names, choices
and defaults (`--rain-color`, `--lightning-color`, `-t`/`--thunder`,
`--speed`, `--sound`, `--volume`), and the sound: the same MP3s played by
`ffplay` with the same volumes, loop, thunder delay (0.35 to 1.15 s),
cooldown (1.1 s), at most two thunders at once and no repeat of the last
two. Checked side by side with the script in two 100x30 tmux panes:
about 160 drops on screen in a storm for both, about 60 calm.

Changed:

- The sound files are built into the binary and written to a temporary
  folder only when sound is on, removed again on exit.
- Colours go out as the plain ANSI codes 30 to 37, as curses sends them,
  over the terminal's own background.
- It runs on the alternate screen and redraws only the cells that change.
- Ctrl+C, SIGTERM and SIGHUP quit like `q`: the terminal is put back and
  every `ffplay` is stopped (the script left them playing on SIGTERM).
  A panic puts the terminal back too.
- Session log: `RUST_LOG=debug terminal-rain 2>terminal-rain.log` (every
  key and resize, every bolt and thunder, a snapshot each second).
- Not on a terminal (or `TERM=dumb`), it says so and exits with status 1
  (the script exited 0).

## Packaging

- `.github/workflows/release.yml` builds static musl Linux (x86_64,
  aarch64), macOS (x86_64, arm64) and Windows (x86_64, arm64, MSVC) on
  tags `*-av.*`, each archive with the binary, `LICENSE`, `NOTICE`,
  `README.md` and `PATCHES.md`.
- `asciiverse/record-preview.sh` records the catalog preview,
  `asciiverse/terminal-rain-lightning.cast`: `terminal-rain -t` for 25
  seconds at 100x30, capped at 30 fps by `asciiverse/cast-cap-fps.py`.
- Checked here: q, Ctrl+C, SIGTERM, SIGHUP and SIGINT quit with status 0
  and the terminal restored, no `ffplay` left and the temporary folder
  gone; resized from 100x30 down to 1x1, 2x3 and 5x4 and up to 120x40;
  sound on and off, volume cycling, all three speeds. Static musl build in
  Alpine; the Windows build cross-compiles (x86_64-pc-windows-gnu) but has
  not been run. macOS not built here.

## Ours

- **How to play it, at the top of README.md.** A short note on installing and
  starting it with av, or from a release archive; the original text follows
  unchanged.
