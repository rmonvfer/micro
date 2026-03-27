//! Asking the terminal what colour its background is.
//!
//! `COLORFGBG` answers this without a round trip, but most terminals do not set it. The
//! other way is OSC 11: write `ESC ] 11 ; ? BEL` and the terminal writes its background back
//! on the input stream. ohm does the same, in `packages/tui/src/terminal-colors.ts`.
//!
//! The exchange happens before the event stream is built, and that ordering is the whole
//! trick. Crossterm has no OSC parser: it strips the escape and re-reads the rest, so a
//! reply reaching the event stream would arrive as `Alt+]` followed by every character of
//! `11;rgb:...` typed into the prompt. Reading the reply off the terminal ourselves, before
//! anything else is listening, is what keeps it out of the editor.
//!
//! Which is why the terminal is asked exactly once, at startup, and the answer kept. A
//! session can ask for the automatic palette again — `/theme auto` — long after the event
//! stream has taken the input, and a second round trip then would be read by two readers at
//! once: some of the reply would land in the prompt, and some of what the user typed would
//! be eaten as though it were the reply. A terminal's background does not change under a
//! running program often enough to be worth that.

use crate::theme::theme_for_rgb;
use crate::theme::Confidence;
use crate::theme::Detection;
use crate::theme::Theme;
use std::time::Duration;

/// The query. `?` asks for the current value rather than setting one.
pub const QUERY: &[u8] = b"\x1b]11;?\x07";

/// How long to wait for a reply. Plenty for a terminal that answers, and short enough that
/// one that never will is not felt at startup.
const TIMEOUT: Duration = Duration::from_millis(100);

/// What a reply opens with. Anything that diverges from this is not ours.
const INTRODUCER: &[u8] = b"\x1b]11;";

/// Longest reply worth accumulating. The longest real one is around thirty bytes.
const MAX_REPLY: usize = 64;

/// How the bytes read so far relate to an OSC 11 reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// A prefix of a reply so far. Keep reading.
    Incomplete,
    /// A whole reply, and the background it named if that could be read.
    Complete(Option<(u8, u8, u8)>),
    /// Not a reply. Whoever is reading must stop at once, because these bytes belong to
    /// somebody else — most likely the user, typing.
    Foreign,
}

/// The palette to open with, asking the terminal if that is what it takes to know.
///
/// `MICRO_THEME` still decides. A name on its own settles the question without a round trip;
/// only ohm's `light-theme/dark-theme` form, and having nothing set at all, need an answer
/// from the terminal. A terminal that does not answer falls back to `COLORFGBG`, and then to
/// dark, exactly as before.
pub fn detect_theme() -> Theme {
    let setting = std::env::var("MICRO_THEME").ok();
    if !needs_terminal(setting.as_deref()) {
        return Theme::from_env();
    }

    match asked() {
        Some(rgb) => Theme::resolve_setting(
            setting.as_deref(),
            Detection {
                theme: theme_for_rgb(rgb),
                confidence: Confidence::High,
            },
        ),
        None => Theme::from_env(),
    }
}

/// What the terminal said its background was, asking it the first time and remembering.
///
/// Called again once the event stream is running, this hands back what was learned at
/// startup rather than asking a second time, because by then the input belongs to somebody
/// else.
fn asked() -> Option<(u8, u8, u8)> {
    *ANSWER.get_or_init(|| query(TIMEOUT))
}

/// Ask the terminal now, while nothing else is reading, so that asking later is free.
///
/// Called once before the event stream is built. Doing it here rather than leaving it to
/// the first caller is what makes `/theme auto` mid-session safe.
pub fn prime() {
    let _ = asked();
}

static ANSWER: std::sync::OnceLock<Option<(u8, u8, u8)>> = std::sync::OnceLock::new();

/// Whether the terminal has to be asked at all.
///
/// A setting naming one theme answers the question by itself, and asking anyway would spend
/// the timeout on an answer nothing would read. ohm's `light-theme/dark-theme` form is the
/// one that still needs to know, and so is having no setting.
fn needs_terminal(setting: Option<&str>) -> bool {
    match setting.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => value.contains('/'),
        None => true,
    }
}

/// Read the bytes accumulated so far, and say whether they are a reply yet.
pub fn progress(buffer: &[u8]) -> Progress {
    let shared = buffer.len().min(INTRODUCER.len());
    if buffer[..shared] != INTRODUCER[..shared] {
        return Progress::Foreign;
    }
    if buffer.len() < INTRODUCER.len() {
        return Progress::Incomplete;
    }

    let body = &buffer[INTRODUCER.len()..];
    match terminator(body) {
        Some(end) => Progress::Complete(
            std::str::from_utf8(&body[..end])
                .ok()
                .and_then(parse_background),
        ),
        // A reply that never ends is a terminal answering something else, or nothing.
        None if buffer.len() >= MAX_REPLY => Progress::Foreign,
        None => Progress::Incomplete,
    }
}

/// Where the payload ends: BEL, or the two bytes of a string terminator.
fn terminator(body: &[u8]) -> Option<usize> {
    body.iter()
        .position(|byte| *byte == 0x07)
        .or_else(|| body.windows(2).position(|pair| pair == [0x1b, b'\\']))
}

/// The colour a reply names, in any of the forms terminals use for it.
pub fn parse_background(payload: &str) -> Option<(u8, u8, u8)> {
    let value = payload.trim();

    if let Some(hex) = value.strip_prefix('#') {
        // `#rrggbb`, or `#rrrrggggbbbb` at the width X11 reports.
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

/// Ask, and read the answer.
#[cfg(unix)]
fn query(timeout: Duration) -> Option<(u8, u8, u8)> {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::time::Instant;

    let mut out = std::io::stdout();
    out.write_all(QUERY).ok()?;
    out.flush().ok()?;

    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    let deadline = Instant::now() + timeout;
    let mut buffer = Vec::new();

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || !readable(fd, remaining) {
            return None;
        }

        let mut chunk = [0u8; 32];
        // SAFETY: the buffer is live for the call and its length is its true capacity.
        let read = unsafe { libc::read(fd, chunk.as_mut_ptr().cast(), chunk.len()) };
        if read <= 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..read as usize]);

        match progress(&buffer) {
            Progress::Incomplete => continue,
            Progress::Complete(background) => return background,
            Progress::Foreign => return None,
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
fn query(_timeout: Duration) -> Option<(u8, u8, u8)> {
    None
}

/// The exchange itself, driven against a pipe standing in for a terminal.
///
/// The parsing is covered by the tests below without any of this; these cover the part that
/// only shows up against a real descriptor — that the wait is bounded, and that a terminal
/// which says nothing is given up on rather than waited out.
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
    fn against(reply: &[u8], timeout: Duration) -> (Option<(u8, u8, u8)>, Duration) {
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
    fn a_terminal_that_answers_is_read() {
        let (found, _) = against(b"\x1b]11;rgb:fdfd/f6f6/e3e3\x07", Duration::from_secs(1));
        assert_eq!(found, Some((253, 246, 227)));
    }

    #[test]
    fn a_terminal_that_says_nothing_is_given_up_on() {
        let timeout = Duration::from_millis(80);
        let (found, elapsed) = against(b"", timeout);

        assert_eq!(found, None);
        assert!(
            elapsed < timeout * 4,
            "waited {elapsed:?} on a terminal that was never going to answer"
        );
    }

    #[test]
    fn a_keystroke_is_left_alone_rather_than_waited_out() {
        let timeout = Duration::from_secs(5);
        let (found, elapsed) = against(b"hello", timeout);

        assert_eq!(found, None);
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
    fn feed(reply: &[u8]) -> Progress {
        let mut buffer = Vec::new();
        for byte in reply {
            buffer.push(*byte);
            match progress(&buffer) {
                Progress::Incomplete => continue,
                done => return done,
            }
        }
        Progress::Incomplete
    }

    #[test]
    fn a_named_theme_settles_it_without_asking_the_terminal() {
        assert!(!needs_terminal(Some("dark")));
        assert!(!needs_terminal(Some("solarized-light")));
        assert!(!needs_terminal(Some("  dark  ")));
    }

    #[test]
    fn the_terminal_is_asked_when_the_setting_cannot_answer() {
        assert!(needs_terminal(None), "nothing set");
        assert!(needs_terminal(Some("")), "set to nothing");
        assert!(needs_terminal(Some("   ")));
        assert!(
            needs_terminal(Some("solarized-light/solarized-dark")),
            "ohm's automatic form picks by what the terminal looks like"
        );
    }

    #[test]
    fn the_query_asks_rather_than_sets() {
        assert_eq!(QUERY, b"\x1b]11;?\x07");
    }

    #[test]
    fn an_xterm_reply_is_read() {
        assert_eq!(
            feed(b"\x1b]11;rgb:1e1e/1e1e/1e1e\x07"),
            Progress::Complete(Some((30, 30, 30)))
        );
    }

    #[test]
    fn a_reply_may_end_with_a_string_terminator() {
        assert_eq!(
            feed(b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\"),
            Progress::Complete(Some((255, 255, 255)))
        );
    }

    #[test]
    fn every_channel_width_scales_to_a_byte() {
        assert_eq!(parse_background("rgb:ff/ff/ff"), Some((255, 255, 255)));
        assert_eq!(
            parse_background("rgb:ffff/ffff/ffff"),
            Some((255, 255, 255))
        );
        assert_eq!(parse_background("rgb:0/0/0"), Some((0, 0, 0)));
        assert_eq!(
            parse_background("rgb:8080/8080/8080"),
            Some((128, 128, 128))
        );
        assert_eq!(parse_background("rgba:1e1e/1e1e/1e1e"), Some((30, 30, 30)));
    }

    #[test]
    fn a_hex_reply_is_read_at_either_width() {
        assert_eq!(parse_background("#1e1e1e"), Some((30, 30, 30)));
        assert_eq!(parse_background("#1e1e1e1e1e1e"), Some((30, 30, 30)));
        assert_eq!(parse_background("#ffffff"), Some((255, 255, 255)));
    }

    #[test]
    fn surrounding_space_does_not_matter() {
        assert_eq!(parse_background("  rgb:0000/0000/0000  "), Some((0, 0, 0)));
    }

    #[test]
    fn a_payload_that_makes_no_sense_reads_as_no_colour() {
        assert_eq!(parse_background(""), None);
        assert_eq!(parse_background("rgb:zz/zz/zz"), None);
        assert_eq!(parse_background("rgb:11/22"), None);
        assert_eq!(parse_background("#abc"), None);
        assert_eq!(feed(b"\x1b]11;nonsense\x07"), Progress::Complete(None));
    }

    /// The reason the reply cannot reach the editor: anything that is not one is refused on
    /// the byte that gives it away, so nothing further is taken from the stream.
    #[test]
    fn a_keystroke_is_recognised_as_somebody_elses_the_moment_it_diverges() {
        assert_eq!(progress(b"h"), Progress::Foreign);
        assert_eq!(progress(b"\x1b[A"), Progress::Foreign, "an arrow key");
        assert_eq!(
            progress(b"\x1b]10;"),
            Progress::Foreign,
            "a foreground reply"
        );
        assert_eq!(progress(b"\x1b]112"), Progress::Foreign);
    }

    #[test]
    fn a_reply_arriving_in_pieces_is_waited_for() {
        assert_eq!(progress(b"\x1b"), Progress::Incomplete);
        assert_eq!(progress(b"\x1b]"), Progress::Incomplete);
        assert_eq!(progress(b"\x1b]11;"), Progress::Incomplete);
        assert_eq!(progress(b"\x1b]11;rgb:1e1e"), Progress::Incomplete);
    }

    #[test]
    fn a_reply_that_never_ends_is_given_up_on() {
        let runaway = [b"\x1b]11;".as_slice(), &[b'a'; MAX_REPLY]].concat();
        assert_eq!(progress(&runaway), Progress::Foreign);
    }

    #[test]
    fn a_dark_background_and_a_light_one_are_told_apart() {
        use crate::theme::TerminalTheme;
        assert_eq!(theme_for_rgb((30, 30, 30)), TerminalTheme::Dark);
        assert_eq!(theme_for_rgb((255, 255, 255)), TerminalTheme::Light);
        assert_eq!(theme_for_rgb((253, 246, 227)), TerminalTheme::Light);
    }
}
