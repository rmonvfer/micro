//! Terminal input translated into intent.
//!
//! Keeping the mapping pure — a crossterm event in, an [`Action`] out — is what lets the key
//! bindings be tested without a terminal, and keeps the event loop free of key matching.

use crossterm::event::Event;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Ignored,
    /// Ctrl+C: abort the running turn, or leave when there is nothing to abort.
    Interrupt,
    /// Ctrl+D: leave when there is nothing written, delete forward when there is.
    ///
    /// The same key means both because that is what a readline prompt has always meant,
    /// and a half-written message is not something to lose to a keystroke reaching for
    /// the character in front of the cursor.
    QuitOrDelete,
    Submit,
    Insert(String),
    Newline,
    Backspace,
    Delete,
    DeleteWordBefore,
    DeleteWordAfter,
    DeleteToLineStart,
    DeleteToLineEnd,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordLeft,
    MoveWordRight,
    MoveLineStart,
    MoveLineEnd,
    /// Pick the tool result before the current one.
    FocusPrevious,
    /// Pick the tool result after the current one.
    FocusNext,
    /// Open or close the picked tool result.
    ToggleFocused,
    ToggleThinking,
    /// Step reasoning effort to the next level.
    CycleThinking,
    /// Send the prompt after the turn in flight rather than instead of it.
    QueueFollowUp,
    /// Pull everything queued back into the buffer.
    Dequeue,
    /// Open the prompt in `$EDITOR`.
    ExternalEditor,
    /// Move the conversation a page at a time.
    PageUp,
    PageDown,
    /// Move the conversation by a few lines, from the wheel or arrows.
    ScrollUp,
    ScrollDown,
    /// Arm jump-to-char: the next printable key moves the cursor to it.
    ArmJump { forward: bool },
    /// Step to the next or previous model in the catalog.
    CycleModel { forward: bool },
    /// Open the model picker.
    SelectModel,
    /// Put the last answer on the system clipboard.
    CopyMessage,
    /// Take an image off the clipboard and attach it to the next prompt.
    PasteImage,
    /// Drop to the shell, leaving the interface to be resumed.
    Suspend,
    /// Insert the most recent kill.
    Yank,
    /// Replace what yank just inserted with the kill before it.
    YankPop,
    /// Step back one edit.
    Undo,
    /// Text arriving from the terminal's paste, rather than typed.
    Paste(String),
    /// Back out of whatever is asking: an approval, a picker, the command menu.
    Cancel,
    /// Take the highlighted completion, or indent when nothing is offering one.
    Tab,
    /// The terminal changed size; the next frame is measured against the new one.
    Resize,
}

/// The intent behind a terminal event.
pub fn action_for(event: &Event) -> Action {
    match event {
        Event::Key(key) => key_action(key),
        // Bracketed paste arrives whole, so a pasted newline inserts a line break instead of
        // submitting the prompt.
        // A paste is its own action: it is cleaned, and a large one is held aside behind a
        // marker rather than filling the prompt.
        Event::Paste(text) => Action::Paste(text.clone()),
        // The wheel is the natural way to read back through a conversation; drags stay
        // the terminal's to select with, which is why only scrolls are answered.
        Event::Mouse(mouse) => match mouse.kind {
            crossterm::event::MouseEventKind::ScrollUp => Action::ScrollUp,
            crossterm::event::MouseEventKind::ScrollDown => Action::ScrollDown,
            _ => Action::Ignored,
        },
        Event::Resize(..) => Action::Resize,
        Event::FocusGained | Event::FocusLost => Action::Ignored,
    }
}

fn key_action(key: &KeyEvent) -> Action {
    // Terminals that speak the kitty protocol report releases and repeats; acting on a
    // release would double every keystroke.
    if key.kind == KeyEventKind::Release {
        return Action::Ignored;
    }

    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        // Alt+Enter queues the prompt behind the turn in flight rather than breaking the
        // line, which is how ohm lets a follow-up be written while an answer is arriving.
        KeyCode::Enter if alt => Action::QueueFollowUp,
        KeyCode::Enter if shift || control => Action::Newline,
        KeyCode::Enter => Action::Submit,

        KeyCode::Backspace if control || alt => Action::DeleteWordBefore,
        KeyCode::Backspace => Action::Backspace,
        KeyCode::Delete if control || alt => Action::DeleteWordAfter,
        KeyCode::Delete => Action::Delete,

        KeyCode::Left if control || alt => Action::MoveWordLeft,
        KeyCode::Left => Action::MoveLeft,
        KeyCode::Right if control || alt => Action::MoveWordRight,
        KeyCode::Right => Action::MoveRight,
        // Alt+Up pulls everything queued back into the buffer to be edited.
        KeyCode::Up if alt => Action::Dequeue,
        KeyCode::Up if control => Action::FocusPrevious,
        KeyCode::Up => Action::MoveUp,
        KeyCode::Down if control => Action::FocusNext,
        KeyCode::Down => Action::MoveDown,

        // ohm claims these for the input. The transcript is the terminal's to scroll, and
        // it has its own wheel and its own keys for that.
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,

        KeyCode::Home => Action::MoveLineStart,
        KeyCode::End => Action::MoveLineEnd,

        KeyCode::Esc => Action::Cancel,

        // Combinations the plain control table cannot express, matched before it.
        KeyCode::Char(']') if control && alt => Action::ArmJump { forward: false },
        KeyCode::Char('p' | 'P') if control && shift => Action::CycleModel { forward: false },
        KeyCode::Char(character) if control => control_action(character),
        KeyCode::Char(character) if alt => alt_action(character),
        KeyCode::Char(character) => Action::Insert(character.to_string()),
        // Shift+Tab cycles reasoning effort, which recolours the rules around the input.
        KeyCode::BackTab => Action::CycleThinking,
        KeyCode::Tab if shift => Action::CycleThinking,
        KeyCode::Tab => Action::Tab,

        _ => Action::Ignored,
    }
}

fn control_action(character: char) -> Action {
    match character.to_ascii_lowercase() {
        'c' => Action::Interrupt,
        'd' => Action::QuitOrDelete,
        'a' => Action::MoveLineStart,
        'e' => Action::MoveLineEnd,
        'b' => Action::MoveLeft,
        'f' => Action::MoveRight,
        'w' => Action::DeleteWordBefore,
        'u' => Action::DeleteToLineStart,
        'k' => Action::DeleteToLineEnd,
        'h' => Action::Backspace,
        'j' => Action::Newline,
        'l' => Action::SelectModel,
        'o' => Action::ToggleFocused,
        't' => Action::ToggleThinking,
        'y' => Action::Yank,
        'g' => Action::ExternalEditor,
        ']' => Action::ArmJump { forward: true },
        'p' => Action::CycleModel { forward: true },
        'x' => Action::CopyMessage,
        'v' => Action::PasteImage,
        'z' => Action::Suspend,
        // ohm binds undo to ctrl+-, which most terminals deliver as ctrl+_.
        '-' | '_' => Action::Undo,
        _ => Action::Ignored,
    }
}

fn alt_action(character: char) -> Action {
    match character.to_ascii_lowercase() {
        'b' => Action::MoveWordLeft,
        'f' => Action::MoveWordRight,
        'd' => Action::DeleteWordAfter,
        'y' => Action::YankPop,
        // Some terminals send Alt+Backspace as Alt+Delete's control character.
        '\u{7f}' => Action::DeleteWordBefore,
        _ => Action::Ignored,
    }
}


/// A key press as a person writes it: `ctrl+h`, `alt+enter`, `shift+f5`.
///
/// The spelling ohm uses for a registered shortcut, so a key an extension asked for is
/// recognised by the name it asked for it under.
pub fn key_name(event: &Event) -> Option<String> {
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind == KeyEventKind::Release {
        return None;
    }

    let mut parts = Vec::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        parts.push("ctrl".to_string());
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        parts.push("alt".to_string());
    }
    // Shift is written only where it is not already in the character: `shift+f5`, but `A`
    // rather than `shift+a`.
    let named = match key.code {
        KeyCode::Char(character) => character.to_ascii_lowercase().to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Tab | KeyCode::BackTab => "tab".to_string(),
        KeyCode::Esc => "escape".to_string(),
        KeyCode::Backspace => "backspace".to_string(),
        KeyCode::Delete => "delete".to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Home => "home".to_string(),
        KeyCode::End => "end".to_string(),
        KeyCode::PageUp => "pageup".to_string(),
        KeyCode::PageDown => "pagedown".to_string(),
        KeyCode::F(number) => format!("f{number}"),
        _ => return None,
    };
    if key.modifiers.contains(KeyModifiers::SHIFT) && !matches!(key.code, KeyCode::Char(_)) {
        parts.push("shift".to_string());
    }
    parts.push(named);
    Some(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn plain(code: KeyCode) -> Event {
        key(code, KeyModifiers::NONE)
    }

    #[test]
    fn enter_submits_and_modified_enter_inserts_a_line_break() {
        assert_eq!(action_for(&plain(KeyCode::Enter)), Action::Submit);
        assert_eq!(
            action_for(&key(KeyCode::Enter, KeyModifiers::SHIFT)),
            Action::Newline
        );
        // Alt+Enter queues a follow-up rather than breaking the line.
        assert_eq!(
            action_for(&key(KeyCode::Enter, KeyModifiers::ALT)),
            Action::QueueFollowUp
        );
        assert_eq!(
            action_for(&key(KeyCode::Char('j'), KeyModifiers::CONTROL)),
            Action::Newline
        );
    }

    #[test]
    fn a_paste_is_its_own_action_and_never_submits() {
        let pasted = "first line\nsecond line\n";
        assert_eq!(
            action_for(&Event::Paste(pasted.to_string())),
            Action::Paste(pasted.to_string())
        );
    }

    #[test]
    fn typing_inserts_the_character() {
        assert_eq!(
            action_for(&plain(KeyCode::Char('a'))),
            Action::Insert("a".into())
        );
        assert_eq!(
            action_for(&key(KeyCode::Char('A'), KeyModifiers::SHIFT)),
            Action::Insert("A".into())
        );
    }

    #[test]
    fn word_motion_is_bound_to_both_conventions() {
        for modifier in [KeyModifiers::ALT, KeyModifiers::CONTROL] {
            assert_eq!(
                action_for(&key(KeyCode::Left, modifier)),
                Action::MoveWordLeft
            );
            assert_eq!(
                action_for(&key(KeyCode::Right, modifier)),
                Action::MoveWordRight
            );
        }
        assert_eq!(
            action_for(&key(KeyCode::Char('b'), KeyModifiers::ALT)),
            Action::MoveWordLeft
        );
        assert_eq!(
            action_for(&key(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            Action::DeleteWordBefore
        );
    }

    #[test]
    fn the_arrows_move_the_cursor() {
        assert_eq!(action_for(&plain(KeyCode::Up)), Action::MoveUp);
        assert_eq!(action_for(&plain(KeyCode::Down)), Action::MoveDown);
    }

    /// The page keys move the conversation; the prompt is for editing, not reading.
    #[test]
    fn the_page_keys_scroll_the_conversation() {
        assert_eq!(action_for(&plain(KeyCode::PageUp)), Action::PageUp);
        assert_eq!(action_for(&plain(KeyCode::PageDown)), Action::PageDown);
        assert_eq!(
            action_for(&key(KeyCode::Home, KeyModifiers::CONTROL)),
            Action::MoveLineStart
        );
    }

    #[test]
    fn the_wheel_scrolls_the_conversation() {
        use crossterm::event::MouseEvent;
        use crossterm::event::MouseEventKind;

        assert_eq!(
            action_for(&Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            })),
            Action::ScrollUp
        );
        assert_eq!(
            action_for(&Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            })),
            Action::ScrollDown
        );
        assert_eq!(
            action_for(&Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(crossterm::event::MouseButton::Left),
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            })),
            Action::Ignored,
            "a click is still the terminal's to select with"
        );
    }

    #[test]
    fn control_arrows_pick_a_tool_result_and_control_o_opens_it() {
        assert_eq!(
            action_for(&key(KeyCode::Up, KeyModifiers::CONTROL)),
            Action::FocusPrevious
        );
        assert_eq!(
            action_for(&key(KeyCode::Down, KeyModifiers::CONTROL)),
            Action::FocusNext
        );
        assert_eq!(
            action_for(&key(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            Action::ToggleFocused
        );
    }

    #[test]
    fn key_releases_are_ignored() {
        let mut event = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
        event.kind = KeyEventKind::Release;
        assert_eq!(action_for(&Event::Key(event)), Action::Ignored);
    }

    #[test]
    fn escape_backs_out() {
        assert_eq!(action_for(&plain(KeyCode::Esc)), Action::Cancel);
    }

    #[test]
    fn a_resize_asks_for_a_repaint() {
        assert_eq!(action_for(&Event::Resize(80, 24)), Action::Resize);
    }

    #[test]
    fn a_key_is_named_the_way_a_shortcut_asks_for_it() {
        assert_eq!(
            key_name(&key(KeyCode::Char('h'), KeyModifiers::CONTROL)).as_deref(),
            Some("ctrl+h")
        );
        assert_eq!(
            key_name(&key(KeyCode::Enter, KeyModifiers::ALT)).as_deref(),
            Some("alt+enter")
        );
        assert_eq!(
            key_name(&key(KeyCode::F(5), KeyModifiers::SHIFT)).as_deref(),
            Some("shift+f5")
        );
        assert_eq!(key_name(&plain(KeyCode::Esc)).as_deref(), Some("escape"));
        // A capital letter is the character, not a modifier and a letter.
        assert_eq!(
            key_name(&key(KeyCode::Char('A'), KeyModifiers::SHIFT)).as_deref(),
            Some("a")
        );
    }
}
