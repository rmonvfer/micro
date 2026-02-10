//! What the terminal can actually do.
//!
//! Every terminal claims to be one of a handful of things and then differs anyway, so this
//! identifies the emulator from its environment and reports only what that emulator is known
//! to support. The default when nothing is recognised is *off*, which matters most for
//! hyperlinks: a terminal that does not understand OSC 8 swallows the sequence, and the URL
//! disappears from the output entirely. Better to print it plainly than to lose it.

/// How a terminal draws an image, when it can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
    /// Kitty's graphics protocol, also spoken by Ghostty, WezTerm and Warp.
    Kitty,
    /// iTerm2's inline image escape.
    ITerm2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub images: Option<ImageProtocol>,
    pub true_color: bool,
    pub hyperlinks: bool,
}

impl Capabilities {
    /// Nothing beyond plain text, which is what an unrecognised terminal gets.
    const fn plain(true_color: bool) -> Self {
        Capabilities {
            images: None,
            true_color,
            hyperlinks: false,
        }
    }

    const fn text_only(true_color: bool, hyperlinks: bool) -> Self {
        Capabilities {
            images: None,
            true_color,
            hyperlinks,
        }
    }

    const fn graphical(images: ImageProtocol) -> Self {
        Capabilities {
            images: Some(images),
            true_color: true,
            hyperlinks: true,
        }
    }
}

/// Work out what this terminal supports.
///
/// The order matters: a multiplexer is checked first, because what it is running inside
/// says nothing about what it will forward.
pub fn detect() -> Capabilities {
    detect_from(&Environment::current())
}

/// The environment variables terminals identify themselves by, gathered so the detection
/// can be tested without setting real ones.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub term_program: String,
    pub terminal_emulator: String,
    pub term: String,
    pub color_term: String,
    pub tmux: bool,
    pub kitty_window_id: bool,
    pub ghostty: bool,
    pub wezterm: bool,
    pub warp: bool,
    pub iterm_session: bool,
    pub windows_terminal: bool,
    /// Whether the tmux in use forwards OSC 8 to the terminal underneath it.
    pub tmux_forwards_hyperlinks: bool,
}

impl Environment {
    pub fn current() -> Self {
        let variable = |name: &str| std::env::var(name).unwrap_or_default().to_lowercase();
        let present = |name: &str| std::env::var(name).is_ok();
        let term = variable("TERM");

        Environment {
            term_program: variable("TERM_PROGRAM"),
            terminal_emulator: variable("TERMINAL_EMULATOR"),
            tmux: present("TMUX") || term.starts_with("tmux"),
            kitty_window_id: present("KITTY_WINDOW_ID"),
            ghostty: present("GHOSTTY_RESOURCES_DIR"),
            wezterm: present("WEZTERM_PANE"),
            warp: present("WARP_SESSION_ID") || present("WARP_TERMINAL_SESSION_UUID"),
            iterm_session: present("ITERM_SESSION_ID"),
            windows_terminal: present("WT_SESSION"),
            tmux_forwards_hyperlinks: false,
            color_term: variable("COLORTERM"),
            term,
        }
    }
}

pub fn detect_from(environment: &Environment) -> Capabilities {
    let true_color_hint =
        environment.color_term == "truecolor" || environment.color_term == "24bit";

    // Image protocols are unreliable through a multiplexer, so they stay off even when the
    // terminal underneath would manage them.
    if environment.tmux {
        return Capabilities::text_only(true_color_hint, environment.tmux_forwards_hyperlinks);
    }
    // screen forwards neither.
    if environment.term.starts_with("screen") {
        return Capabilities::plain(true_color_hint);
    }

    if environment.kitty_window_id || environment.term_program == "kitty" {
        return Capabilities::graphical(ImageProtocol::Kitty);
    }
    if environment.ghostty
        || environment.term_program == "ghostty"
        || environment.term.contains("ghostty")
    {
        return Capabilities::graphical(ImageProtocol::Kitty);
    }
    if environment.wezterm || environment.term_program == "wezterm" {
        return Capabilities::graphical(ImageProtocol::Kitty);
    }
    if environment.warp || environment.term_program == "warpterminal" {
        return Capabilities::graphical(ImageProtocol::Kitty);
    }
    if environment.iterm_session || environment.term_program == "iterm.app" {
        return Capabilities::graphical(ImageProtocol::ITerm2);
    }

    // Known, and known not to draw images.
    if environment.windows_terminal
        || environment.term_program == "vscode"
        || environment.term_program == "alacritty"
    {
        return Capabilities::text_only(true, true);
    }
    if environment.terminal_emulator == "jetbrains-jediterm" {
        return Capabilities::text_only(true, false);
    }

    // Unrecognised: assume nothing. A swallowed OSC 8 loses the URL, so the plain
    // `text (url)` form is the safer default.
    Capabilities::plain(true_color_hint)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn environment(program: &str) -> Environment {
        Environment {
            term_program: program.to_string(),
            ..Environment::default()
        }
    }

    #[test]
    fn kitty_and_the_terminals_that_speak_its_protocol_draw_images() {
        for program in ["kitty", "ghostty", "wezterm", "warpterminal"] {
            let capabilities = detect_from(&environment(program));
            assert_eq!(
                capabilities.images,
                Some(ImageProtocol::Kitty),
                "{program} speaks the kitty protocol"
            );
            assert!(capabilities.hyperlinks);
        }
    }

    #[test]
    fn iterm_uses_its_own_image_escape() {
        let capabilities = detect_from(&environment("iterm.app"));
        assert_eq!(capabilities.images, Some(ImageProtocol::ITerm2));
    }

    #[test]
    fn a_terminal_that_draws_no_images_can_still_take_hyperlinks() {
        for program in ["vscode", "alacritty"] {
            let capabilities = detect_from(&environment(program));
            assert_eq!(capabilities.images, None);
            assert!(capabilities.hyperlinks, "{program} understands OSC 8");
        }
    }

    /// Images are unreliable through a multiplexer, and hyperlinks depend on whether that
    /// multiplexer forwards them, so it is asked rather than assumed.
    #[test]
    fn tmux_turns_images_off_and_defers_on_hyperlinks() {
        let inside = Environment {
            tmux: true,
            term_program: "kitty".into(),
            ..Environment::default()
        };
        assert_eq!(detect_from(&inside).images, None);
        assert!(!detect_from(&inside).hyperlinks);

        let forwarding = Environment {
            tmux_forwards_hyperlinks: true,
            ..inside
        };
        assert!(detect_from(&forwarding).hyperlinks);
    }

    #[test]
    fn screen_forwards_neither() {
        let capabilities = detect_from(&Environment {
            term: "screen.xterm-256color".into(),
            ..Environment::default()
        });
        assert_eq!(capabilities.images, None);
        assert!(!capabilities.hyperlinks);
    }

    /// An unknown terminal gets the plain form. A swallowed OSC 8 loses the URL entirely,
    /// which is worse than printing it.
    #[test]
    fn an_unrecognised_terminal_is_assumed_to_support_nothing() {
        let capabilities = detect_from(&Environment::default());
        assert_eq!(capabilities.images, None);
        assert!(!capabilities.hyperlinks);
        assert!(!capabilities.true_color);
    }

    #[test]
    fn a_truecolor_hint_is_believed_even_when_the_terminal_is_unknown() {
        let capabilities = detect_from(&Environment {
            color_term: "truecolor".into(),
            ..Environment::default()
        });
        assert!(capabilities.true_color);
    }
}
