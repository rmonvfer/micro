//! The slash-command menu shown while a command is being typed.
//!
//! Behaviour follows ohm's editor autocomplete: the menu belongs to the text before the
//! cursor and is rebuilt on every keystroke, so the selection returns to the top whenever
//! the filter changes. Moving through it wraps at both ends, and committing replaces what
//! was typed with the command and a trailing space, ready for its argument.

use micro_models::fuzzy;
use micro_commands::Command;
use std::ops::Range;

/// Rows the menu shows at once before it scrolls, matching ohm's editor default.
pub const MAX_VISIBLE: usize = 5;

/// How many files a menu offers at once. Past this the list stops being a list.
const MAX_FILE_SUGGESTIONS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuItem {
    /// The command name, which is also what gets inserted.
    pub value: String,
    /// The argument hint and description, shown in a second column.
    pub description: String,
}

/// What the menu is offering, which decides what committing writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offering {
    /// Slash commands. Committing writes `/name ` with a trailing space.
    Commands,
    /// Workspace files, reached with `@`. Committing writes `@path ` so the path stays
    /// attached to the marker that introduced it.
    Files,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    items: Vec<MenuItem>,
    /// What the user has typed, including the leading marker, which committing replaces.
    prefix: String,
    selected: usize,
    offering: Offering,
}

impl Menu {
    /// The menu belonging to `line` with the cursor at byte offset `cursor`, or nothing when
    /// no menu belongs there.
    ///
    /// A menu opens on a line that begins with a slash and has no space yet: once the
    /// command word is finished, what follows is its argument, not a command to pick.
    pub fn open_for(line: &str, cursor: usize) -> Option<Menu> {
        let typed = line.get(..cursor)?;
        if !typed.starts_with('/') || typed.contains(char::is_whitespace) {
            return None;
        }

        let candidates = fuzzy::filter(
            micro_commands::commands().to_vec(),
            &typed[1..],
            |command| command.name.to_string(),
        );
        if candidates.is_empty() {
            return None;
        }

        Some(Menu {
            items: candidates.into_iter().map(MenuItem::from).collect(),
            prefix: typed.to_string(),
            selected: 0,
            offering: Offering::Commands,
        })
    }

    /// The file menu belonging to `line` with the cursor at `cursor`, or nothing.
    ///
    /// A file menu opens on the word under the cursor when it begins with `@`. Unlike a
    /// command it may appear anywhere in the line, because naming a file is something a
    /// sentence does in passing rather than a thing the whole line is.
    pub fn files_for(line: &str, cursor: usize, paths: &[String]) -> Option<Menu> {
        let typed = line.get(..cursor)?;
        let start = typed
            .rfind(char::is_whitespace)
            .map(|index| index + 1)
            .unwrap_or(0);
        let word = typed.get(start..)?;
        let query = word.strip_prefix('@')?;

        let candidates = fuzzy::filter(paths.to_vec(), query, |path| path.clone());
        if candidates.is_empty() {
            return None;
        }

        Some(Menu {
            items: candidates
                .into_iter()
                .take(MAX_FILE_SUGGESTIONS)
                .map(|path| MenuItem {
                    value: path,
                    description: String::new(),
                })
                .collect(),
            prefix: word.to_string(),
            selected: 0,
            offering: Offering::Files,
        })
    }

    pub fn items(&self) -> &[MenuItem] {
        &self.items
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&MenuItem> {
        self.items.get(self.selected)
    }

    /// What committing would replace.
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn select_next(&mut self) {
        self.selected = match self.selected + 1 == self.items.len() {
            true => 0,
            false => self.selected + 1,
        };
    }

    pub fn select_previous(&mut self) {
        self.selected = match self.selected {
            0 => self.items.len().saturating_sub(1),
            index => index - 1,
        };
    }

    /// The text the prefix becomes when the selection is committed. The trailing space puts
    /// the cursor where an argument would go, and closes the menu.
    pub fn commit(&self) -> Option<String> {
        let item = self.selected_item()?;
        Some(match self.offering {
            Offering::Commands => format!("/{} ", item.value),
            Offering::Files => format!("@{} ", item.value),
        })
    }

    /// The slice of items on screen, keeping the selection near the middle of the window.
    pub fn window(&self, max_visible: usize) -> Range<usize> {
        let max_visible = max_visible.max(1);
        if self.items.len() <= max_visible {
            return 0..self.items.len();
        }
        let start = self
            .selected
            .saturating_sub(max_visible / 2)
            .min(self.items.len() - max_visible);
        start..start + max_visible
    }
}

impl From<Command> for MenuItem {
    fn from(command: Command) -> Self {
        // The hint and the description read as one phrase, the way ohm joins them.
        let description = match command.argument {
            Some(argument) => format!("{argument} — {}", command.description),
            None => command.description.to_string(),
        };
        MenuItem {
            value: command.name.to_string(),
            description,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(menu: &Menu) -> Vec<&str> {
        menu.items.iter().map(|item| item.value.as_str()).collect()
    }

    #[test]
    fn a_slash_opens_the_whole_list() {
        let menu = Menu::open_for("/", 1).expect("a menu");
        assert_eq!(menu.items().len(), micro_commands::commands().len());
        assert_eq!(menu.selected(), 0);
    }

    #[test]
    fn ordinary_text_opens_nothing() {
        assert!(Menu::open_for("explain this", 12).is_none());
        assert!(Menu::open_for("", 0).is_none());
    }

    #[test]
    fn the_menu_closes_once_the_command_word_is_finished() {
        assert!(Menu::open_for("/model", 6).is_some());
        assert!(
            Menu::open_for("/model ", 7).is_none(),
            "a space begins the argument"
        );
        assert!(Menu::open_for("/model opus", 11).is_none());
    }

    #[test]
    fn the_menu_belongs_to_the_text_before_the_cursor() {
        // Cursor left of the slash: nothing has been typed toward a command yet.
        assert!(Menu::open_for("/model", 0).is_none());
        let menu = Menu::open_for("/model", 2).expect("a menu");
        assert_eq!(menu.prefix(), "/m");
    }

    #[test]
    fn typing_narrows_the_list() {
        assert_eq!(values(&Menu::open_for("/com", 4).unwrap()), vec!["compact"]);
        assert_eq!(
            values(&Menu::open_for("/co", 3).unwrap()),
            vec!["copy", "compact", "clone", "changelog"]
        );

        let menu = Menu::open_for("/c", 2).unwrap();
        assert_eq!(
            values(&menu),
            vec!["clone", "changelog", "copy", "compact", "clear", "cwd"]
        );
    }

    #[test]
    fn matching_is_fuzzy_rather_than_a_prefix() {
        let menu = Menu::open_for("/mdl", 4).expect("a fuzzy match still opens the menu");
        assert_eq!(values(&menu), vec!["model"]);
    }

    #[test]
    fn a_command_that_matches_nothing_closes_the_menu() {
        assert!(Menu::open_for("/zzzz", 5).is_none());
    }

    #[test]
    fn an_argument_hint_is_shown_beside_the_description() {
        let menu = Menu::open_for("/fork", 5).unwrap();
        assert_eq!(
            menu.items()[0].description,
            "[index] — branch the conversation at a message"
        );

        let menu = Menu::open_for("/clear", 6).unwrap();
        assert_eq!(menu.items()[0].description, "start a fresh conversation");
    }

    #[test]
    fn moving_through_the_list_wraps_at_both_ends() {
        let mut menu = Menu::open_for("/c", 2).unwrap();
        assert_eq!(menu.selected(), 0);

        let last = values(&menu).len() - 1;
        menu.select_next();
        assert_eq!(menu.selected(), 1);
        for _ in 0..last {
            menu.select_next();
        }
        assert_eq!(menu.selected(), 0, "past the end wraps to the top");

        menu.select_previous();
        assert_eq!(menu.selected(), last, "before the top wraps to the end");
    }

    #[test]
    fn committing_inserts_the_command_and_a_space() {
        let mut menu = Menu::open_for("/c", 2).unwrap();
        assert_eq!(menu.commit().as_deref(), Some("/clone "));
        menu.select_next();
        assert_eq!(menu.commit().as_deref(), Some("/changelog "));
    }

    #[test]
    fn the_window_keeps_the_selection_near_the_middle() {
        let mut menu = Menu::open_for("/", 1).unwrap();
        let total = menu.items().len();
        assert!(total > MAX_VISIBLE, "the test needs a scrolling list");

        assert_eq!(menu.window(MAX_VISIBLE), 0..MAX_VISIBLE);

        for _ in 0..4 {
            menu.select_next();
        }
        let window = menu.window(MAX_VISIBLE);
        assert!(window.contains(&menu.selected()));
        assert_eq!(window.len(), MAX_VISIBLE);

        // At the end the window stops rather than running past the last item.
        menu.select_previous();
        for _ in 0..total {
            menu.select_next();
        }
        let window = menu.window(MAX_VISIBLE);
        assert_eq!(window.end, total.min(window.start + MAX_VISIBLE));
        assert!(window.end <= total);
    }

    #[test]
    fn a_short_list_needs_no_window() {
        let menu = Menu::open_for("/com", 4).unwrap();
        assert_eq!(menu.window(MAX_VISIBLE), 0..1);
    }
}

#[cfg(test)]
mod files {
    use super::*;

    fn paths() -> Vec<String> {
        vec![
            "src/main.rs".to_string(),
            "src/app.rs".to_string(),
            "docs/architecture.md".to_string(),
            "README.md".to_string(),
        ]
    }

    /// A name after `@` offers the workspace's files.
    #[test]
    fn an_at_sign_offers_files() {
        let menu = Menu::files_for("@main", 5, &paths()).expect("it offers something");
        assert_eq!(menu.selected_item().map(|item| item.value.as_str()), Some("src/main.rs"));
    }

    /// Committing keeps the marker, so the path stays attached to what introduced it.
    #[test]
    fn committing_keeps_the_marker() {
        let menu = Menu::files_for("@main", 5, &paths()).unwrap();
        assert_eq!(menu.commit().as_deref(), Some("@src/main.rs "));
        assert_eq!(menu.prefix(), "@main");
    }

    /// A file can be named part way through a sentence, not only at the start.
    #[test]
    fn a_file_can_be_named_anywhere_in_the_line() {
        let menu = Menu::files_for("please read @arch", 17, &paths()).expect("it opens");
        assert_eq!(
            menu.selected_item().map(|item| item.value.as_str()),
            Some("docs/architecture.md")
        );
        assert_eq!(menu.prefix(), "@arch");
    }

    /// A word that is not a file reference offers nothing.
    #[test]
    fn a_plain_word_offers_nothing() {
        assert!(Menu::files_for("main", 4, &paths()).is_none());
        assert!(Menu::files_for("", 0, &paths()).is_none());
    }

    /// A name matching nothing offers nothing rather than the whole workspace.
    #[test]
    fn a_name_matching_nothing_offers_nothing() {
        assert!(Menu::files_for("@zzzzz", 6, &paths()).is_none());
    }

    /// A command still opens the command menu, not the file one.
    #[test]
    fn a_slash_still_means_a_command() {
        let menu = Menu::open_for("/mod", 4).expect("commands still work");
        assert!(menu.commit().is_some_and(|written| written.starts_with('/')));
    }
}
