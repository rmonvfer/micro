//! The list a command opens when it wants the user to choose.
//!
//! Shaped after ohm's selectors: a title, a filter the user types into, a list that narrows
//! as they type, and a marker on whatever is in use now. Choosing hands back the command
//! line the item carries, which the interface dispatches as though it had been typed.

use crate::fuzzy;
use micro_commands::Picker as Choices;
use micro_commands::PickerItem;
use std::ops::Range;

/// Rows a picker shows at once before it scrolls, matching ohm's selectors.
pub const MAX_VISIBLE: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    choices: Choices,
    query: String,
    /// Indices into `choices.items`, narrowed by the query and ranked by it.
    matches: Vec<usize>,
    selected: usize,
}

impl Picker {
    pub fn new(choices: Choices) -> Self {
        let mut picker = Picker {
            choices,
            query: String::new(),
            matches: Vec::new(),
            selected: 0,
        };
        picker.refilter();
        // Opening on whatever is in use saves a scroll to find where you already are.
        if let Some(current) = picker
            .matches
            .iter()
            .position(|index| picker.choices.items[*index].current)
        {
            picker.selected = current;
        }
        picker
    }

    pub fn title(&self) -> &str {
        &self.choices.title
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// How many items there are to choose from, before the query narrows them.
    pub fn total(&self) -> usize {
        self.choices.items.len()
    }

    /// The items the query left, in the order they should be shown.
    pub fn matches(&self) -> Vec<&PickerItem> {
        self.matches
            .iter()
            .map(|index| &self.choices.items[*index])
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        self.matches
            .get(self.selected)
            .map(|index| &self.choices.items[*index])
    }

    /// The command line to dispatch for the highlighted item.
    pub fn commit(&self) -> Option<String> {
        self.selected_item().map(|item| item.command.clone())
    }

    pub fn push(&mut self, text: &str) {
        self.query.push_str(text);
        self.refilter();
    }

    /// Remove the last character of the query, if there is one.
    pub fn backspace(&mut self) {
        if self.query.pop().is_some() {
            self.refilter();
        }
    }

    pub fn select_next(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.selected = match self.selected + 1 == self.matches.len() {
            true => 0,
            false => self.selected + 1,
        };
    }

    pub fn select_previous(&mut self) {
        if self.matches.is_empty() {
            return;
        }
        self.selected = match self.selected {
            0 => self.matches.len() - 1,
            index => index - 1,
        };
    }

    /// The slice on screen, keeping the selection near the middle of the window.
    pub fn window(&self, max_visible: usize) -> Range<usize> {
        let max_visible = max_visible.max(1);
        if self.matches.len() <= max_visible {
            return 0..self.matches.len();
        }
        let start = self
            .selected
            .saturating_sub(max_visible / 2)
            .min(self.matches.len() - max_visible);
        start..start + max_visible
    }

    /// An item is matched on its label and its detail together, so `/model` narrows on a
    /// context size as readily as on a name.
    fn refilter(&mut self) {
        let indexed: Vec<usize> = (0..self.choices.items.len()).collect();
        let items = &self.choices.items;
        self.matches = fuzzy::filter(indexed, &self.query, |index| {
            format!("{} {}", items[*index].label, items[*index].detail)
        });
        self.selected = self.selected.min(self.matches.len().saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices() -> Choices {
        Choices::new(
            "Select a model",
            vec![
                PickerItem::new("anthropic/claude-opus-5", "200k context", "/model opus-5"),
                PickerItem::new(
                    "anthropic/claude-sonnet-5",
                    "200k context",
                    "/model sonnet-5",
                )
                .current(true),
                PickerItem::new(
                    "google/gemini-2.5-pro",
                    "1M context",
                    "/model gemini-2.5-pro",
                ),
            ],
        )
    }

    fn labels(picker: &Picker) -> Vec<&str> {
        picker
            .matches()
            .into_iter()
            .map(|item| item.label.as_str())
            .collect()
    }

    #[test]
    fn a_picker_opens_on_whatever_is_in_use() {
        let picker = Picker::new(choices());
        assert_eq!(picker.title(), "Select a model");
        assert_eq!(picker.selected(), 1);
        assert_eq!(
            picker.selected_item().unwrap().label,
            "anthropic/claude-sonnet-5"
        );
        assert_eq!(picker.total(), 3);
    }

    #[test]
    fn a_picker_with_nothing_in_use_opens_at_the_top() {
        let picker = Picker::new(Choices::new(
            "Pick",
            vec![
                PickerItem::new("a", "", "/a"),
                PickerItem::new("b", "", "/b"),
            ],
        ));
        assert_eq!(picker.selected(), 0);
    }

    #[test]
    fn typing_narrows_the_list() {
        let mut picker = Picker::new(choices());
        picker.push("gem");
        assert_eq!(labels(&picker), vec!["google/gemini-2.5-pro"]);
        assert_eq!(picker.query(), "gem");

        picker.backspace();
        picker.backspace();
        picker.backspace();
        assert_eq!(labels(&picker).len(), 3);
    }

    #[test]
    fn the_detail_is_searched_alongside_the_label() {
        let mut picker = Picker::new(choices());
        picker.push("1M");
        assert_eq!(labels(&picker), vec!["google/gemini-2.5-pro"]);
    }

    #[test]
    fn a_query_that_matches_nothing_leaves_an_empty_list() {
        let mut picker = Picker::new(choices());
        picker.push("zzzz");
        assert!(picker.is_empty());
        assert_eq!(picker.commit(), None);
        // Moving through an empty list is not an error, it just does nothing.
        picker.select_next();
        assert_eq!(picker.selected(), 0);
    }

    #[test]
    fn narrowing_keeps_the_selection_inside_the_list() {
        let mut picker = Picker::new(choices());
        picker.select_next();
        picker.select_next();
        assert_eq!(picker.selected(), 0, "the selection wrapped");

        picker.push("anthropic");
        assert!(picker.selected() < picker.matches().len());
    }

    #[test]
    fn moving_wraps_at_both_ends() {
        let mut picker = Picker::new(choices());
        picker.push("a");
        let count = picker.matches().len();
        assert!(count > 1);

        let start = picker.selected();
        for _ in 0..count {
            picker.select_next();
        }
        assert_eq!(
            picker.selected(),
            start,
            "a full lap returns to where it began"
        );

        while picker.selected() != 0 {
            picker.select_previous();
        }
        picker.select_previous();
        assert_eq!(
            picker.selected(),
            count - 1,
            "before the top wraps to the end"
        );
    }

    #[test]
    fn choosing_hands_back_the_command_to_dispatch() {
        let mut picker = Picker::new(choices());
        picker.push("gemini");
        assert_eq!(picker.commit().as_deref(), Some("/model gemini-2.5-pro"));
    }

    #[test]
    fn the_window_keeps_the_selection_near_the_middle() {
        let items = (0..30)
            .map(|index| PickerItem::new(format!("item {index}"), "", format!("/pick {index}")))
            .collect();
        let mut picker = Picker::new(Choices::new("Pick", items));

        assert_eq!(picker.window(MAX_VISIBLE), 0..MAX_VISIBLE);
        for _ in 0..20 {
            picker.select_next();
        }
        let window = picker.window(MAX_VISIBLE);
        assert!(window.contains(&picker.selected()));
        assert_eq!(window.len(), MAX_VISIBLE);
        assert!(window.end <= 30);
    }
}
