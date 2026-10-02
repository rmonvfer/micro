//! Asking the terminal what colours it draws with: its foreground, its background, and its ANSI
//! palette.

use crate::theme::Confidence;
use crate::theme::Detection;
use crate::theme::TerminalColors;
use crate::theme::Theme;
use std::time::Duration;

/// How long to wait for the replies.
const TIMEOUT: Duration = Duration::from_millis(100);

/// What a colour reply opens with.
const OSC: &[u8] = b"\x1b]";

/// What a primary device attributes reply opens with. Every terminal answers that request, and in
/// order, so its reply says the colour replies that were coming have all arrived.
const ATTRIBUTES: &[u8] = b"\x1b[?";

/// Longest reply worth accumulating.
const MAX_REPLY: usize = 64;

/// The questions, in the order the replies come back: the default foreground (OSC 10), the default
/// background (OSC 11), each of the sixteen ANSI colours (OSC 4), then the device attributes that
/// close the exchange.
pub fn queries() -> Vec<u8> {
    let mut out = b"\x1b]10;?\x07\x1b]11;?\x07".to_vec();
    for index in 0..16 {
        out.extend_from_slice(format!("\x1b]4;{index};?\x07").as_bytes());
    }
    out.extend_from_slice(b"\x1b[c");
    out
}

/// How the bytes read so far relate to the replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Replies so far, possibly ending part way through one.
    Incomplete,
    /// The device attributes arrived, so nothing more is coming.
    Complete,
    /// Something that is not a reply, such as a key press.
    Foreign,
}

/// What the replies read so far reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Replies {
    pub foreground: Option<(u8, u8, u8)>,
    pub background: Option<(u8, u8, u8)>,
    pub palette: [Option<(u8, u8, u8)>; 16],
}

impl Replies {
    /// The colours, keeping the palette only when every one of the sixteen was reported.
    pub fn colors(&self) -> TerminalColors {
        let mut palette = [(0, 0, 0); 16];
        let complete = self
            .palette
            .iter()
            .zip(palette.iter_mut())
            .all(|(reported, slot)| reported.map(|color| *slot = color).is_some());
        TerminalColors {
            foreground: self.foreground,
            background: self.background,
            palette: complete.then_some(palette),
        }
    }
}

/// The theme the setting asks for, asking the terminal for its colours when that is what it takes
/// to know.
pub fn detect_theme(setting: Option<&str>) -> Theme {
    let setting = setting.map(str::trim).filter(|value| !value.is_empty());
    if !needs_terminal(setting) {
        return Theme::resolve_setting(setting, crate::theme::detected_from_env());
    }
    theme_for(setting, &asked())
}

/// The theme the setting asks for, given the colours the terminal reported.
fn theme_for(setting: Option<&str>, colors: &TerminalColors) -> Theme {
    let detection = match colors.background {
        Some(background) => Detection {
            theme: crate::theme::appearance(background, colors.foreground),
            confidence: Confidence::High,
        },
        None => crate::theme::detected_from_env(),
    };
    Theme::for_terminal(setting, colors, detection)
}

/// Ask the terminal again and build the theme from what it says now. Only call this while nothing
/// else is reading the terminal's input, or the replies go to whoever reads first.
pub fn refresh_theme(setting: Option<&str>) -> Theme {
    let colors = query(TIMEOUT).colors();
    *ANSWER.lock().unwrap_or_else(|error| error.into_inner()) = Some(colors);
    theme_for(
        setting.map(str::trim).filter(|value| !value.is_empty()),
        &colors,
    )
}

/// What the terminal said its colours were, asking it the first time and remembering.
fn asked() -> TerminalColors {
    *ANSWER
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_or_insert_with(|| query(TIMEOUT).colors())
}

pub fn prime() {
    let _ = asked();
}

static ANSWER: std::sync::Mutex<Option<TerminalColors>> = std::sync::Mutex::new(None);

/// Whether the terminal has to be asked at all: for the theme built from its colours, which is the
/// default, and for a light/dark pair.
pub fn needs_terminal(setting: Option<&str>) -> bool {
    match setting.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => value == crate::theme::SYSTEM || value.contains('/'),
        None => true,
    }
}

/// Read the bytes accumulated so far, collecting every whole reply into `replies`, and say whether
/// more are expected.
pub fn progress(buffer: &[u8], replies: &mut Replies) -> Progress {
    let mut at = 0;
    while at < buffer.len() {
        let rest = &buffer[at..];
        if OSC.starts_with(rest) || ATTRIBUTES.starts_with(rest) {
            return Progress::Incomplete;
        }
        if let Some(body) = rest.strip_prefix(OSC) {
            match terminator(body) {
                Some((end, length)) => {
                    if let Ok(payload) = std::str::from_utf8(&body[..end]) {
                        record(payload, replies);
                    }
                    at += OSC.len() + end + length;
                    continue;
                }
                None if rest.len() >= MAX_REPLY => return Progress::Foreign,
                None => return Progress::Incomplete,
            }
        }
        if let Some(body) = rest.strip_prefix(ATTRIBUTES) {
            match body.iter().position(|byte| (0x40..=0x7e).contains(byte)) {
                Some(end) if body[end] == b'c' => return Progress::Complete,
                Some(_) => return Progress::Foreign,
                None if rest.len() >= MAX_REPLY => return Progress::Foreign,
                None => return Progress::Incomplete,
            }
        }
        return Progress::Foreign;
    }
    Progress::Incomplete
}

/// Where the payload ends, and how long its terminator is: BEL, or the two bytes of a string
/// terminator.
fn terminator(body: &[u8]) -> Option<(usize, usize)> {
    let bell = body.iter().position(|byte| *byte == 0x07);
    let string = body.windows(2).position(|pair| pair == [0x1b, b'\\']);
    match (bell, string) {
        (Some(bell), Some(string)) if string < bell => Some((string, 2)),
        (Some(bell), _) => Some((bell, 1)),
        (None, Some(string)) => Some((string, 2)),
        (None, None) => None,
    }
}

/// File one reply's colour under what it answers.
fn record(payload: &str, replies: &mut Replies) {
    if let Some(color) = payload.strip_prefix("10;") {
        replies.foreground = parse_color(color);
    } else if let Some(color) = payload.strip_prefix("11;") {
        replies.background = parse_color(color);
    } else if let Some(rest) = payload.strip_prefix("4;") {
        let Some((index, color)) = rest.split_once(';') else {
            return;
        };
        if let Some(slot) = index
            .parse::<usize>()
            .ok()
            .and_then(|index| replies.palette.get_mut(index))
        {
            *slot = parse_color(color);
        }
    }
}

/// The colour a reply names, in any of the forms terminals use for it.
pub fn parse_color(payload: &str) -> Option<(u8, u8, u8)> {
    let value = payload.trim();

    if let Some(hex) = value.strip_prefix('#') {
        let width = match hex.len() {
            6 => 2,
            12 => 4,
            _ => return None,
        };
        return Some((
            channel(&hex[..width])?,
            channel(&hex[width..width * 2])?,
            channel(&hex[width * 2..])?,
        ));
    }

    let value = value
        .strip_prefix("rgba:")
        .or_else(|| value.strip_prefix("rgb:"))
        .unwrap_or(value);
    let mut parts = value.split('/');
    let red = channel(parts.next()?)?;
    let green = channel(parts.next()?)?;
    let blue = channel(parts.next()?)?;
    Some((red, green, blue))
}

/// One channel, scaled from however many hex digits it was reported in down to a byte.
fn channel(text: &str) -> Option<u8> {
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let max = 16u64.checked_pow(text.len() as u32)?.checked_sub(1)?;
    let value = u64::from_str_radix(text, 16).ok()?;
    Some((value as f64 / max as f64 * 255.0).round() as u8)
}

/// Ask, and read the answers.
#[cfg(unix)]
fn query(timeout: Duration) -> Replies {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::time::Instant;

    let mut replies = Replies::default();
    let mut out = std::io::stdout();
    if out
        .write_all(&queries())
        .and_then(|()| out.flush())
        .is_err()
    {
        return replies;
    }

    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    let deadline = Instant::now() + timeout;
    let mut buffer = Vec::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || !readable(fd, remaining) {
            return replies;
        }

        let mut chunk = [0u8; 256];
        // SAFETY: the buffer is live for the call and its length is its true capacity.
        let read = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if read <= 0 {
            return replies;
        }
        buffer.extend_from_slice(&chunk[..read as usize]);

        let mut found = Replies::default();
        match progress(&buffer, &mut found) {
            Progress::Incomplete => replies = found,
            Progress::Complete | Progress::Foreign => return found,
        }
    }
}

/// Whether the descriptor has something to read, waiting no longer than `timeout`.
#[cfg(unix)]
fn readable(fd: std::os::fd::RawFd, timeout: Duration) -> bool {
    let mut watch = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let millis = timeout.as_millis().min(i32::MAX as u128) as i32;
    // SAFETY: one descriptor is passed, and the struct outlives the call.
    unsafe { libc::poll(&mut watch, 1, millis) > 0 }
}

/// Terminals that answer this are the ones this does not run on.
#[cfg(not(unix))]
fn query(_timeout: Duration) -> Replies {
    Replies::default()
}

/// The exchange itself, driven against a pipe standing in for a terminal.
#[cfg(all(test, unix))]
mod exchange {
    use super::*;
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::sync::Mutex;
    use std::time::Instant;

    /// Standard input is process-wide, so only one of these runs at a time.
    static STDIN: Mutex<()> = Mutex::new(());

    /// Run `query` with `reply` already waiting on standard input.
    fn against(reply: &[u8], timeout: Duration) -> (Replies, Duration) {
        let _guard = STDIN.lock().unwrap_or_else(|error| error.into_inner());

        let (read, mut write) = std::io::pipe().expect("a pipe");
        if !reply.is_empty() {
            write.write_all(reply).expect("the reply");
        }

        // SAFETY: fd 0 is saved before it is replaced and put back below, and the lock keeps
        // any other test from reading standard input in between.
        let saved = unsafe { libc::dup(0) };
        assert!(saved >= 0, "standard input could not be saved");
        unsafe { libc::dup2(read.as_raw_fd(), 0) };

        let started = Instant::now();
        let found = query(timeout);
        let elapsed = started.elapsed();

        unsafe {
            libc::dup2(saved, 0);
            libc::close(saved);
        }
        (found, elapsed)
    }

    #[test]
    fn a_terminal_that_answers_is_read_without_waiting_out_the_timeout() {
        let timeout = Duration::from_secs(5);
        let (found, elapsed) = against(
            b"\x1b]10;rgb:c6c6/d0d0/f5f5\x07\x1b]11;rgb:fdfd/f6f6/e3e3\x07\x1b[?62;22c",
            timeout,
        );
        assert_eq!(found.foreground, Some((198, 208, 245)));
        assert_eq!(found.background, Some((253, 246, 227)));
        assert!(elapsed < Duration::from_secs(1), "waited {elapsed:?}");
    }

    #[test]
    fn a_terminal_that_says_nothing_is_given_up_on() {
        let timeout = Duration::from_millis(80);
        let (found, elapsed) = against(b"", timeout);

        assert_eq!(found, Replies::default());
        assert!(
            elapsed < timeout * 4,
            "waited {elapsed:?} on a terminal that was never going to answer"
        );
    }

    #[test]
    fn a_keystroke_is_left_alone_rather_than_waited_out() {
        let timeout = Duration::from_secs(5);
        let (found, elapsed) = against(b"hello", timeout);

        assert_eq!(found, Replies::default());
        assert!(
            elapsed < Duration::from_millis(500),
            "typing should be recognised as not a reply at once, not after {elapsed:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bytes fed one at a time, the way they arrive from a terminal.
    fn feed(reply: &[u8]) -> (Progress, Replies) {
        let mut buffer = Vec::new();
        for byte in reply {
            buffer.push(*byte);
            let mut replies = Replies::default();
            match progress(&buffer, &mut replies) {
                Progress::Incomplete => continue,
                done => return (done, replies),
            }
        }
        let mut replies = Replies::default();
        (progress(&buffer, &mut replies), replies)
    }

    #[test]
    fn a_named_theme_settles_it_without_asking_the_terminal() {
        assert!(!needs_terminal(Some("dark")));
        assert!(!needs_terminal(Some("solarized-light")));
        assert!(!needs_terminal(Some("  dark  ")));
    }

    #[test]
    fn the_terminal_is_asked_when_the_setting_cannot_answer() {
        assert!(needs_terminal(None), "nothing set means the system theme");
        assert!(needs_terminal(Some("")), "set to nothing");
        assert!(needs_terminal(Some("   ")));
        assert!(needs_terminal(Some("system")));
        assert!(
            needs_terminal(Some("solarized-light/solarized-dark")),
            "the automatic form picks by what the terminal looks like"
        );
    }

    #[test]
    fn the_queries_ask_rather_than_set_and_end_with_the_device_attributes() {
        let queries = String::from_utf8(queries()).unwrap();
        assert!(queries.starts_with("\x1b]10;?\x07\x1b]11;?\x07\x1b]4;0;?\x07"));
        assert!(queries.contains("\x1b]4;15;?\x07"));
        assert!(queries.ends_with("\x1b[c"));
    }

    #[test]
    fn every_reply_is_filed_under_what_it_answers() {
        let mut reply = b"\x1b]10;rgb:ffff/ffff/ffff\x07\x1b]11;rgb:1e1e/1e1e/1e1e\x1b\\".to_vec();
        for index in 0..16 {
            reply.extend_from_slice(format!("\x1b]4;{index};rgb:{index:02x}/00/00\x07").as_bytes());
        }
        reply.extend_from_slice(b"\x1b[?1;2c");

        let (state, replies) = feed(&reply);
        assert_eq!(state, Progress::Complete);
        assert_eq!(replies.foreground, Some((255, 255, 255)));
        assert_eq!(replies.background, Some((30, 30, 30)));
        let colors = replies.colors();
        let palette = colors.palette.expect("all sixteen were reported");
        assert_eq!(palette[15], (15, 0, 0));
    }

    #[test]
    fn a_partial_palette_is_dropped() {
        let (_, replies) = feed(b"\x1b]11;#000000\x07\x1b]4;1;#ff0000\x07\x1b[?6c");
        assert_eq!(replies.palette[1], Some((255, 0, 0)));
        assert_eq!(replies.colors().palette, None);
        assert_eq!(replies.colors().background, Some((0, 0, 0)));
    }

    #[test]
    fn every_channel_width_scales_to_a_byte() {
        assert_eq!(parse_color("rgb:ff/ff/ff"), Some((255, 255, 255)));
        assert_eq!(parse_color("rgb:ffff/ffff/ffff"), Some((255, 255, 255)));
        assert_eq!(parse_color("rgb:0/0/0"), Some((0, 0, 0)));
        assert_eq!(parse_color("rgb:8080/8080/8080"), Some((128, 128, 128)));
        assert_eq!(parse_color("rgba:1e1e/1e1e/1e1e"), Some((30, 30, 30)));
    }

    #[test]
    fn a_hex_reply_is_read_at_either_width() {
        assert_eq!(parse_color("#1e1e1e"), Some((30, 30, 30)));
        assert_eq!(parse_color("#1e1e1e1e1e1e"), Some((30, 30, 30)));
        assert_eq!(parse_color("#ffffff"), Some((255, 255, 255)));
    }

    #[test]
    fn surrounding_space_does_not_matter() {
        assert_eq!(parse_color("  rgb:0000/0000/0000  "), Some((0, 0, 0)));
    }

    #[test]
    fn a_payload_that_makes_no_sense_reads_as_no_colour() {
        assert_eq!(parse_color(""), None);
        assert_eq!(parse_color("rgb:zz/zz/zz"), None);
        assert_eq!(parse_color("rgb:11/22"), None);
        assert_eq!(parse_color("#abc"), None);
        let (_, replies) = feed(b"\x1b]11;nonsense\x07\x1b[?6c");
        assert_eq!(replies.background, None);
    }

    /// The reason the replies cannot reach the editor: anything that is not one is refused on the
    /// byte that gives it away.
    #[test]
    fn a_keystroke_is_recognised_as_somebody_elses_the_moment_it_diverges() {
        let mut replies = Replies::default();
        assert_eq!(progress(b"h", &mut replies), Progress::Foreign);
        assert_eq!(
            progress(b"\x1b[A", &mut replies),
            Progress::Foreign,
            "an arrow key"
        );
        assert_eq!(
            progress(b"\x1b]11;#000000\x07x", &mut replies),
            Progress::Foreign
        );
    }

    #[test]
    fn a_reply_arriving_in_pieces_is_waited_for() {
        let mut replies = Replies::default();
        assert_eq!(progress(b"\x1b", &mut replies), Progress::Incomplete);
        assert_eq!(progress(b"\x1b]", &mut replies), Progress::Incomplete);
        assert_eq!(progress(b"\x1b]11;", &mut replies), Progress::Incomplete);
        assert_eq!(
            progress(b"\x1b]11;rgb:1e1e", &mut replies),
            Progress::Incomplete
        );
        assert_eq!(progress(b"\x1b[?", &mut replies), Progress::Incomplete);
        assert_eq!(progress(b"\x1b[?62;", &mut replies), Progress::Incomplete);
    }

    #[test]
    fn a_reply_that_never_ends_is_given_up_on() {
        let runaway = [b"\x1b]11;".as_slice(), &[b'a'; MAX_REPLY]].concat();
        assert_eq!(
            progress(&runaway, &mut Replies::default()),
            Progress::Foreign
        );
    }

    #[test]
    fn a_named_theme_or_a_silent_terminal_falls_back_to_the_built_in_themes() {
        assert_eq!(
            theme_for(Some("light"), &TerminalColors::default()),
            Theme::light()
        );
        let dark = TerminalColors {
            background: Some((20, 20, 20)),
            ..TerminalColors::default()
        };
        assert_eq!(theme_for(Some("light/dark"), &dark), Theme::dark());
        assert_eq!(theme_for(None, &dark).name, crate::theme::SYSTEM);
    }
}
