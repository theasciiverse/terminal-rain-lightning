//! Terminal Rain: rain and lightning in the terminal.
//!
//! A port to Rust of rmaake1's terminal_rain_lightning.py
//! (github.com/rmaake1/terminal-rain-lightning, MIT), kept as close to it
//! as a port can be: the same drops, bolts, colours, keys, speeds and
//! sounds. The Python script stays in this repository beside it.

use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    queue,
    style::{Attribute, Print, SetAttribute},
    terminal,
};

// ---------------------------------------------------------------- settings
// Names and values as in the Python script.

const UPDATE_INTERVAL: f64 = 0.015; // fast
const MEDIUM_MOTION_INTERVAL: f64 = 0.045;
const SLOW_MOTION_INTERVAL: f64 = 0.1;
const SPEED_MODES: [(&str, f64); 3] =
    [("fast", UPDATE_INTERVAL), ("medium", MEDIUM_MOTION_INTERVAL), ("slow", SLOW_MOTION_INTERVAL)];

/// (name, rain volume, thunder volume), ffplay's 0 to 100.
const VOLUME_PRESETS: [(&str, u32, u32); 3] = [("quiet", 50, 50), ("normal", 80, 70), ("loud", 100, 90)];

const RAIN_CHARS: [char; 3] = ['|', '.', '`'];

/// The eight curses colours by name, as their ANSI numbers (SGR 30 to 37,
/// which every colour terminal knows, as curses sends them).
const COLORS: [(&str, u8); 8] = [
    ("black", 0),
    ("red", 1),
    ("green", 2),
    ("yellow", 3),
    ("blue", 4),
    ("magenta", 5),
    ("cyan", 6),
    ("white", 7),
];

const LIGHTNING_CHANCE: f64 = 0.005;
const LIGHTNING_CHARS: [char; 3] = ['*', '+', '#']; // dimmest to brightest
const LIGHTNING_GROWTH_DELAY: f64 = 0.002;
const LIGHTNING_MAX_BRANCHES: i32 = 2;
const LIGHTNING_BRANCH_CHANCE: f64 = 0.3;
const FORK_CHANCE: f64 = 0.15;
const FORK_HORIZONTAL_SPREAD: i32 = 3;
const SEGMENT_LIFESPAN: f64 = 0.8; // seconds for a segment to fade from # to nothing
const THUNDER_DELAY_RANGE: (f64, f64) = (0.35, 1.15);

/// The sound files, built into the binary from upstream's assets folder.
const RAIN_SOUND: (&str, &[u8]) = ("rain.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/rain.mp3"));
const THUNDER_SOUNDS: [(&str, &[u8]); 9] = [
    ("thunder.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/thunder.mp3")),
    ("thunder-deep.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/thunder-deep.mp3")),
    ("thunder-bright.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/thunder-bright.mp3")),
    ("thunder-distant.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/thunder-distant.mp3")),
    ("thunder-roll.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/thunder-roll.mp3")),
    ("thunder-low-roll.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/thunder-low-roll.mp3")),
    ("thunder-sharp.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/thunder-sharp.mp3")),
    ("thunder-far.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/thunder-far.mp3")),
    ("thunder-short.mp3", include_bytes!("../terminal_rain_lightning_assets/sounds/thunder-short.mp3")),
];

/// random.randint: both ends included.
fn randint(a: i32, b: i32) -> i32 {
    fastrand::i32(a..=b)
}

/// random.uniform
fn uniform(a: f64, b: f64) -> f64 {
    a + (b - a) * fastrand::f64()
}

// ---------------------------------------------------------------- sound

/// Rain and thunder through `ffplay` (FFmpeg), as upstream plays them. The
/// MP3s are written to a temporary folder the first time one is needed.
struct SoundManager {
    enabled: bool,
    available: bool,
    volume_preset: usize,
    temp_dir: Option<PathBuf>,
    rain_process: Option<Child>,
    thunder_processes: Vec<Child>,
    recent_thunder_sounds: Vec<&'static str>,
    pending_thunder_time: Option<Instant>,
    last_thunder_time: Option<Instant>,
    thunder_cooldown: Duration,
    max_thunder_processes: usize,
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let names: &[String] = &if cfg!(windows) { vec![format!("{name}.exe"), name.to_string()] } else { vec![name.to_string()] };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.is_file())
}

impl SoundManager {
    fn new(enabled: bool, volume_preset: usize) -> SoundManager {
        let available = find_on_path("ffplay").is_some();
        log::info!("sound {enabled}, ffplay found {available}");
        SoundManager {
            enabled,
            available,
            volume_preset,
            temp_dir: None,
            rain_process: None,
            thunder_processes: Vec::new(),
            recent_thunder_sounds: Vec::new(),
            pending_thunder_time: None,
            last_thunder_time: None,
            thunder_cooldown: Duration::from_secs_f64(1.1),
            max_thunder_processes: 2,
        }
    }

    fn start(&mut self) {
        if self.enabled {
            self.start_rain();
        }
    }

    fn toggle(&mut self) {
        self.enabled = !self.enabled;
        log::info!("sound {}", if self.enabled { "on" } else { "off" });
        if self.enabled {
            self.start_rain();
        } else {
            self.stop();
        }
    }

    fn cycle_volume(&mut self) {
        self.volume_preset = (self.volume_preset + 1) % VOLUME_PRESETS.len();
        log::info!("volume {}", VOLUME_PRESETS[self.volume_preset].0);
        if let Some(p) = self.rain_process.take() {
            terminate(p);
            self.start_rain();
        }
    }

    fn update(&mut self) {
        if self.pending_thunder_time.is_some_and(|t| Instant::now() >= t) {
            self.pending_thunder_time = None;
            self.play_thunder();
        }
        self.cleanup_finished_thunder();
    }

    fn start_rain(&mut self) {
        if !self.enabled || !self.available || self.rain_process.is_some() {
            return;
        }
        let Some(rain_sound) = self.sound_path(RAIN_SOUND) else { return };
        let volume = VOLUME_PRESETS[self.volume_preset].1.to_string();
        self.rain_process = self.spawn(&["-nodisp", "-loop", "0", "-loglevel", "quiet", "-volume", &volume], &rain_sound);
    }

    fn schedule_thunder(&mut self) {
        if !self.enabled || !self.available || self.pending_thunder_time.is_some() {
            return;
        }
        let now = Instant::now();
        if self.last_thunder_time.is_some_and(|t| now - t < self.thunder_cooldown) {
            return;
        }
        let delay = uniform(THUNDER_DELAY_RANGE.0, THUNDER_DELAY_RANGE.1);
        self.pending_thunder_time = Some(now + Duration::from_secs_f64(delay));
    }

    fn play_thunder(&mut self) {
        if !self.enabled || !self.available {
            return;
        }
        let now = Instant::now();
        if self.last_thunder_time.is_some_and(|t| now - t < self.thunder_cooldown) {
            return;
        }
        self.last_thunder_time = Some(now);
        self.cleanup_finished_thunder();
        if self.thunder_processes.len() >= self.max_thunder_processes {
            return;
        }
        let thunder = self.choose_thunder_sound();
        let Some(thunder_sound) = self.sound_path(thunder) else { return };
        let volume = VOLUME_PRESETS[self.volume_preset].2.to_string();
        if let Some(p) = self.spawn(&["-nodisp", "-autoexit", "-loglevel", "quiet", "-volume", &volume], &thunder_sound) {
            log::debug!("thunder {}", thunder.0);
            self.thunder_processes.push(p);
            self.recent_thunder_sounds.push(thunder.0);
            let n = self.recent_thunder_sounds.len();
            self.recent_thunder_sounds.drain(..n.saturating_sub(2));
        }
    }

    fn stop(&mut self) {
        self.pending_thunder_time = None;
        if let Some(p) = self.rain_process.take() {
            terminate(p);
        }
        for p in self.thunder_processes.drain(..) {
            terminate(p);
        }
    }

    fn spawn(&mut self, args: &[&str], sound: &Path) -> Option<Child> {
        match Command::new("ffplay")
            .args(args)
            .arg(sound)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(p) => Some(p),
            Err(e) => {
                log::warn!("ffplay: {e}");
                self.available = false;
                None
            }
        }
    }

    fn sound_path(&mut self, (filename, data): (&str, &[u8])) -> Option<PathBuf> {
        if self.temp_dir.is_none() {
            let dir = std::env::temp_dir().join(format!("terminal-rain-{}-{:08x}", std::process::id(), fastrand::u32(..)));
            if let Err(e) = std::fs::create_dir(&dir) {
                log::warn!("{}: {e}", dir.display());
                self.available = false;
                return None;
            }
            self.temp_dir = Some(dir);
        }
        let path = self.temp_dir.as_ref()?.join(filename);
        if !path.exists() {
            if let Err(e) = std::fs::write(&path, data) {
                log::warn!("{}: {e}", path.display());
                self.available = false;
                return None;
            }
        }
        Some(path)
    }

    fn choose_thunder_sound(&self) -> (&'static str, &'static [u8]) {
        let choices: Vec<_> = THUNDER_SOUNDS.iter().filter(|s| !self.recent_thunder_sounds.contains(&s.0)).collect();
        if choices.is_empty() {
            THUNDER_SOUNDS[fastrand::usize(..THUNDER_SOUNDS.len())]
        } else {
            *choices[fastrand::usize(..choices.len())]
        }
    }

    fn cleanup_finished_thunder(&mut self) {
        self.thunder_processes.retain_mut(|p| matches!(p.try_wait(), Ok(None)));
    }
}

impl Drop for SoundManager {
    fn drop(&mut self) {
        self.stop();
        if let Some(dir) = self.temp_dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// Stops an ffplay and waits for it, so no player outlives us.
fn terminate(mut p: Child) {
    if matches!(p.try_wait(), Ok(None)) {
        let _ = p.kill();
        let _ = p.wait();
    }
}

// ---------------------------------------------------------------- rain and lightning

struct Raindrop {
    x: i32,
    y: f64,
    speed: f64, // rows fallen per update
    ch: char,
}

struct LightningBolt {
    target_length: usize,
    /// (y, x, creation time)
    segments: Vec<(i32, i32, Instant)>,
    last_growth_time: Instant,
    is_growing: bool,
    max_y: i32,
    max_x: i32,
}

impl LightningBolt {
    fn new(start_row: i32, start_col: i32, max_y: i32, max_x: i32) -> LightningBolt {
        let now = Instant::now();
        LightningBolt {
            target_length: randint(max_y / 2, max_y - 2) as usize,
            segments: vec![(start_row, start_col, now)],
            last_growth_time: now,
            is_growing: true,
            max_y,
            max_x,
        }
    }

    /// Grows the bolt; false once every segment has faded and it can go.
    fn update(&mut self) -> bool {
        let current_time = Instant::now();

        if self.is_growing && (current_time - self.last_growth_time).as_secs_f64() >= LIGHTNING_GROWTH_DELAY {
            self.last_growth_time = current_time;
            let mut new_segments: Vec<(i32, i32, Instant)> = Vec::new();
            let mut added_segment = false;
            let (last_y, last_x, _) = *self.segments.last().unwrap();

            if self.segments.len() < self.target_length && last_y < self.max_y - 1 {
                let mut branches = 1;
                if fastrand::f64() < LIGHTNING_BRANCH_CHANCE {
                    branches = randint(1, LIGHTNING_MAX_BRANCHES + 1);
                }

                let mut current_x = last_x;
                let mut next_primary_x = current_x;
                for i in 0..branches {
                    let offset = randint(-2, 2);
                    let next_x = (current_x + offset).clamp(0, self.max_x - 1);
                    let next_y = (last_y + 1).min(self.max_y - 1);
                    new_segments.push((next_y, next_x, current_time));
                    if i == 0 {
                        next_primary_x = next_x;
                    }
                    current_x = next_x;
                    added_segment = true;
                }

                if fastrand::f64() < FORK_CHANCE {
                    let mut fork_offset = randint(-FORK_HORIZONTAL_SPREAD, FORK_HORIZONTAL_SPREAD);
                    if fork_offset == 0 {
                        fork_offset = if fastrand::bool() { -1 } else { 1 };
                    }
                    let fork_x = (last_x + fork_offset).clamp(0, self.max_x - 1);
                    let fork_y = (last_y + 1).min(self.max_y - 1);
                    if fork_x != next_primary_x {
                        new_segments.push((fork_y, fork_x, current_time));
                        added_segment = true;
                    }
                }
            }

            if !added_segment || self.segments.len() >= self.target_length || last_y >= self.max_y - 1 {
                self.is_growing = false;
            }

            // One new segment per cell this step, in the order first added
            // (the script's dict of positions).
            let start = self.segments.len();
            for s in new_segments {
                if !self.segments[start..].iter().any(|t| (t.0, t.1) == (s.0, s.1)) {
                    self.segments.push(s);
                }
            }
        }

        self.segments.iter().any(|s| (current_time - s.2).as_secs_f64() <= SEGMENT_LIFESPAN)
    }

    fn draw(&self, f: &mut Frame, attr: Look) {
        let now = Instant::now();
        for &(y, x, created) in &self.segments {
            let norm_age = (now - created).as_secs_f64() / SEGMENT_LIFESPAN;
            if norm_age > 1.0 {
                continue;
            }
            let ch = if norm_age < 0.33 {
                LIGHTNING_CHARS[2]
            } else if norm_age < 0.66 {
                LIGHTNING_CHARS[1]
            } else {
                LIGHTNING_CHARS[0]
            };
            f.put(x, y, ch, attr);
        }
    }
}

// ---------------------------------------------------------------- screen

#[derive(Clone, Copy, PartialEq)]
enum Weight {
    Normal,
    Bold,
    Dim,
}

/// What curses calls a cell's attributes: a colour and bold or dim.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    color: u8,
    weight: Weight,
}

#[derive(Clone, Copy, PartialEq)]
struct Cell {
    ch: char,
    look: Look,
}

const BLANK: Cell = Cell { ch: ' ', look: Look { color: 9, weight: Weight::Normal } };

struct Frame {
    w: i32,
    h: i32,
    cells: Vec<Cell>,
}

impl Frame {
    fn new(w: i32, h: i32) -> Frame {
        Frame { w, h, cells: vec![BLANK; (w.max(0) * h.max(0)) as usize] }
    }
    fn put(&mut self, x: i32, y: i32, ch: char, look: Look) {
        if (0..self.w).contains(&x) && (0..self.h).contains(&y) {
            self.cells[(y * self.w + x) as usize] = Cell { ch, look };
        }
    }
}

struct Screen {
    out: BufWriter<io::Stdout>,
    prev: Option<Frame>,
}

impl Screen {
    /// Writes the cells that changed since the last frame, like curses'
    /// doupdate.
    fn draw(&mut self, f: Frame) -> io::Result<()> {
        let mut look: Option<Look> = None;
        let mut at: Option<(i32, i32)> = None;
        for y in 0..f.h {
            for x in 0..f.w {
                let c = f.cells[(y * f.w + x) as usize];
                if self.prev.as_ref().is_some_and(|p| p.cells[(y * f.w + x) as usize] == c) {
                    continue;
                }
                if at != Some((x, y)) {
                    queue!(self.out, cursor::MoveTo(x as u16, y as u16))?;
                }
                if c.ch != ' ' && look != Some(c.look) {
                    if look.map(|l| l.weight) != Some(c.look.weight) {
                        queue!(self.out, SetAttribute(Attribute::NormalIntensity))?;
                        match c.look.weight {
                            Weight::Bold => queue!(self.out, SetAttribute(Attribute::Bold))?,
                            Weight::Dim => queue!(self.out, SetAttribute(Attribute::Dim))?,
                            Weight::Normal => {}
                        }
                    }
                    queue!(self.out, Print(format_args!("\x1b[3{}m", c.look.color)))?;
                    look = Some(c.look);
                }
                queue!(self.out, Print(c.ch))?;
                at = Some((x + 1, y));
            }
        }
        self.out.flush()?;
        self.prev = Some(f);
        Ok(())
    }
}

// ---------------------------------------------------------------- main

struct Options {
    rain_color: usize,
    lightning_color: usize,
    thunder: bool,
    speed: usize,
    sound: bool,
    volume: usize,
}

fn names<T>(list: &[(&str, T)]) -> String {
    list.iter().map(|c| c.0).collect::<Vec<_>>().join(", ")
}

fn usage() {
    let colors = names(&COLORS);
    let speeds = names(&SPEED_MODES);
    let volumes = VOLUME_PRESETS.iter().map(|v| v.0).collect::<Vec<_>>().join(", ");
    println!("usage: terminal-rain [-h] [--rain-color COLOR] [--lightning-color COLOR] [-t]");
    println!("                     [--speed SPEED] [--sound] [--volume VOLUME]\n");
    println!("Simulates rain and thunderstorms in the terminal.\n");
    println!("options:");
    println!("  -h, --help             show this help message and exit");
    println!("  --rain-color COLOR     Color for the rain. Default: cyan. Choices: {colors}");
    println!("  --lightning-color COLOR");
    println!("                         Color for the lightning. Default: yellow. Choices: {colors}");
    println!("  -t, --thunder          Start the program in thunderstorm mode");
    println!("  --speed SPEED          Starting animation speed. Default: fast. Choices: {speeds}");
    println!("  --sound                Enable rain and thunder sounds. Requires ffplay from ffmpeg.");
    println!("  --volume VOLUME        Rain and thunder volume preset. Default: normal. Choices: {volumes}\n");
    println!("keys: t thunderstorm, s speed, m sound, v volume, q or Esc quits");
}

fn fail(msg: &str) -> ! {
    eprintln!("terminal-rain: error: {msg}");
    eprintln!("try 'terminal-rain --help'");
    std::process::exit(2);
}

fn choice(option: &str, value: Option<String>, list: &[&str]) -> usize {
    let Some(value) = value else { fail(&format!("argument {option}: expected one argument")) };
    list.iter().position(|c| *c == value).unwrap_or_else(|| {
        let quoted = list.iter().map(|c| format!("'{c}'")).collect::<Vec<_>>().join(", ");
        fail(&format!("argument {option}: invalid choice: '{value}' (choose from {quoted})"))
    })
}

fn parse_args() -> Options {
    let colors: Vec<&str> = COLORS.iter().map(|c| c.0).collect();
    let speeds: Vec<&str> = SPEED_MODES.iter().map(|s| s.0).collect();
    let volumes: Vec<&str> = VOLUME_PRESETS.iter().map(|v| v.0).collect();
    let mut o = Options { rain_color: 6, lightning_color: 3, thunder: false, speed: 0, sound: false, volume: 1 };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        // --option=value as well as --option value
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let mut value = || inline.clone().or_else(|| args.next());
        match name.as_str() {
            "--rain-color" => o.rain_color = choice("--rain-color", value(), &colors),
            "--lightning-color" => o.lightning_color = choice("--lightning-color", value(), &colors),
            "--speed" => o.speed = choice("--speed", value(), &speeds),
            "--volume" => o.volume = choice("--volume", value(), &volumes),
            "-t" | "--thunder" => o.thunder = true,
            "--sound" => o.sound = true,
            "-h" | "--help" => {
                usage();
                std::process::exit(0);
            }
            _ => fail(&format!("unrecognized arguments: {a}")),
        }
    }
    o
}

fn main() -> io::Result<()> {
    let opts = parse_args();
    if !io::stdout().is_terminal() || std::env::var("TERM").is_ok_and(|t| t == "dumb") {
        eprintln!("Error: This program requires a terminal.");
        std::process::exit(1);
    }
    // Only writes when RUST_LOG is set: RUST_LOG=debug terminal-rain 2>terminal-rain.log
    env_logger::init();

    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    for sig in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(sig, Arc::clone(&stop))?;
    }

    // Give the terminal back even if something panics.
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut out = io::stdout();
        let _ = crossterm::execute!(
            out,
            SetAttribute(Attribute::Reset),
            crossterm::style::ResetColor,
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
        hook(info);
    }));

    let mut sound = SoundManager::new(opts.sound, opts.volume);

    terminal::enable_raw_mode()?;
    let mut screen = Screen { out: BufWriter::with_capacity(1 << 16, io::stdout()), prev: None };
    queue!(screen.out, terminal::EnterAlternateScreen, cursor::Hide, terminal::Clear(terminal::ClearType::All))?;
    screen.out.flush()?;

    let result = simulate_rain(&mut screen, &opts, &mut sound, &stop);

    drop(sound);
    queue!(
        screen.out,
        SetAttribute(Attribute::Reset),
        crossterm::style::ResetColor,
        cursor::Show,
        terminal::LeaveAlternateScreen
    )?;
    screen.out.flush()?;
    terminal::disable_raw_mode()?;
    result
}

fn simulate_rain(screen: &mut Screen, opts: &Options, sound: &mut SoundManager, stop: &AtomicBool) -> io::Result<()> {
    let rain_color = COLORS[opts.rain_color].1;
    let lightning = Look { color: COLORS[opts.lightning_color].1, weight: Weight::Bold };
    let mut raindrops: Vec<Raindrop> = Vec::new();
    let mut active_bolts: Vec<LightningBolt> = Vec::new();
    let (w, h) = terminal::size()?;
    let (mut cols, mut rows) = (w as i32, h as i32);
    let mut is_thunderstorm = opts.thunder;
    let mut speed_mode_index = opts.speed;
    log::info!(
        "start {cols}x{rows} rain {} lightning {} thunderstorm {is_thunderstorm} speed {}",
        COLORS[opts.rain_color].0,
        COLORS[opts.lightning_color].0,
        SPEED_MODES[speed_mode_index].0
    );
    sound.start();
    let mut next_snapshot = Instant::now() + Duration::from_secs(1);

    loop {
        let tick = Instant::now();
        if stop.load(Ordering::Relaxed) {
            log::info!("signal, quitting");
            return Ok(());
        }

        // --- update
        sound.update();

        if rows >= 4 && is_thunderstorm && active_bolts.len() < 3 && fastrand::f64() < LIGHTNING_CHANCE {
            let start_col = randint(cols / 4, 3 * cols / 4);
            let start_row = randint(0, rows / 5);
            log::debug!("bolt at {start_col},{start_row}");
            active_bolts.push(LightningBolt::new(start_row, start_col, rows, cols));
            sound.schedule_thunder();
        }
        active_bolts.retain_mut(|b| b.update());

        let generation_chance = if is_thunderstorm { 0.5 } else { 0.3 };
        let max_new_drops = if is_thunderstorm { cols / 8 } else { cols / 15 };
        let min_speed = 0.3;
        let max_speed = if is_thunderstorm { 1.0 } else { 0.6 };

        if cols > 0 && fastrand::f64() < generation_chance {
            for _ in 0..randint(1, max_new_drops.max(1)) {
                raindrops.push(Raindrop {
                    x: randint(0, cols - 1),
                    y: 0.0,
                    speed: uniform(min_speed, max_speed),
                    ch: RAIN_CHARS[fastrand::usize(..RAIN_CHARS.len())],
                });
            }
        }
        for drop in &mut raindrops {
            drop.y += drop.speed;
        }
        raindrops.retain(|d| (d.y as i32) < rows);

        // --- draw: lightning first, rain over it
        let mut f = Frame::new(cols, rows);
        for bolt in &active_bolts {
            bolt.draw(&mut f, lightning);
        }
        for drop in &raindrops {
            let weight = if is_thunderstorm {
                Weight::Bold
            } else if drop.speed < 0.8 {
                Weight::Dim
            } else {
                Weight::Normal
            };
            f.put(drop.x, drop.y as i32, drop.ch, Look { color: rain_color, weight });
        }
        screen.draw(f)?;

        if Instant::now() >= next_snapshot {
            next_snapshot += Duration::from_secs(1);
            log::debug!(
                "{cols}x{rows} drops {} bolts {} thunderstorm {is_thunderstorm} speed {} sound {} volume {}",
                raindrops.len(),
                active_bolts.len(),
                SPEED_MODES[speed_mode_index].0,
                sound.enabled,
                VOLUME_PRESETS[sound.volume_preset].0
            );
        }

        // --- input until the next update is due
        let deadline = tick + Duration::from_secs_f64(SPEED_MODES[speed_mode_index].1);
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if !event::poll(left)? {
                break;
            }
            let ev = event::read()?;
            log::debug!("event {ev:?}");
            match ev {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                    KeyCode::Char('q' | 'Q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('t' | 'T') => {
                        is_thunderstorm = !is_thunderstorm;
                        log::info!("thunderstorm {is_thunderstorm}");
                    }
                    KeyCode::Char('s' | 'S') => {
                        speed_mode_index = (speed_mode_index + 1) % SPEED_MODES.len();
                        log::info!("speed {}", SPEED_MODES[speed_mode_index].0);
                    }
                    KeyCode::Char('m' | 'M') => sound.toggle(),
                    KeyCode::Char('v' | 'V') => sound.cycle_volume(),
                    _ => {}
                },
                Event::Resize(nw, nh) => {
                    (cols, rows) = (nw as i32, nh as i32);
                    log::info!("resize {cols}x{rows}");
                    raindrops.clear();
                    active_bolts.clear();
                    screen.prev = None;
                    queue!(screen.out, crossterm::style::ResetColor, terminal::Clear(terminal::ClearType::All))?;
                }
                _ => {}
            }
        }
    }
}
