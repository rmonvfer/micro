//! The list a command opens when it wants the user to choose.
//!
//! Shaped after ohm's selectors: a title, a filter the user types into, a list that narrows
//! as they type, and a marker on whatever is in use now. Choosing hands back the command
//! line the item carries, which the interface dispatches as though it had been typed.

use micro_models::fuzzy;
use micro_commands::Picker as Choices;
use micro_commands::PickerItem;
use std::ops::Range;

/// Rows a picker shows at once before it scrolls, matching ohm's selectors.
pub const MAX_VISIBLE: usize = 10;
/// The gap between a label's column and the detail beside it.
pub const COLUMN_GAP: usize = 2;

/// Which of a list's two views is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Everything there is.
    All,
    /// What the workspace put on its shortlist.
    Scoped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picker {
    choices: Choices,
    /// Which view is showing. A list with a shortlist opens on it, because a workspace that
    /// named a handful of models meant those to be the ones in front of you.
    scope: Scope,
    query: String,
    /// Indices into `choices.items`, narrowed by the query and ranked by it.
    matches: Vec<usize>,
    /// What the list leaves out, when it leaves anything out.
    hint: Option<String>,
    /// What is happening behind the list, when something is: the catalogs being refreshed,
    /// and then whether that worked.
    status: Option<(String, bool)>,
    selected: usize,
}

impl Picker {
    pub fn new(choices: Choices) -> Self {
        let scope = match choices.scoped.is_empty() {
            true => Scope::All,
            false => Scope::Scoped,
        };
        let mut picker = Picker {
            hint: choices.hint.clone(),
            status: None,
            scope,
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
            .position(|index| picker.showing()[*index].current)
        {
            picker.selected = current;
        }
        picker
    }

    pub fn title(&self) -> &str {
        &self.choices.title
    }

    /// Whether the providers are worth asking about while this list is open.
    pub fn refreshes(&self) -> bool {
        self.choices.refreshes
    }

    /// Whether this list has a shortlist to switch between at all.
    pub fn has_scopes(&self) -> bool {
        !self.choices.scoped.is_empty()
    }

    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// Switch between the shortlist and the whole of it, keeping whatever has been typed.
    pub fn toggle_scope(&mut self) {
        if !self.has_scopes() {
            return;
        }
        self.scope = match self.scope {
            Scope::All => Scope::Scoped,
            Scope::Scoped => Scope::All,
        };
        self.selected = 0;
        self.refilter();
    }

    /// The choices the current view offers.
    fn showing(&self) -> &[PickerItem] {
        match self.scope {
            Scope::All => &self.choices.items,
            Scope::Scoped => &self.choices.scoped,
        }
    }

    /// Whether the list has a line to narrow it by.
    pub fn searchable(&self) -> bool {
        self.choices.searchable
    }

    /// Whether the list names itself and says which keys work.
    pub fn titled(&self) -> bool {
        self.choices.titled
    }

    /// How wide the label's column is: the widest label there is, held between the bounds
    /// the list asked for. Measured over everything the query left rather than over what is
    /// on screen, so scrolling the list does not shift its second column.
    pub fn column(&self) -> usize {
        if self.choices.layout == micro_commands::PickerLayout::Badges {
            return 0;
        }
        let (min, max) = self.choices.column;
        let showing = self.showing();
        let widest = self
            .matches
            .iter()
            .map(|index| crate::wrap::text_width(&showing[*index].label))
            .max()
            .unwrap_or(0);
        (widest + COLUMN_GAP).clamp(min, max)
    }

    /// What this list leaves out, when it leaves anything out.
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// What is happening behind the list, and whether it went well.
    pub fn status(&self) -> Option<(&str, bool)> {
        self.status.as_ref().map(|(text, ok)| (text.as_str(), *ok))
    }

    pub fn set_status(&mut self, text: impl Into<String>, ok: bool) {
        self.status = Some((text.into(), ok));
    }

    /// Replace what the list offers, keeping where the reader is and what they typed.
    ///
    /// The catalogs finishing a refresh must not move the selection out from under a hand
    /// already on its way to pressing enter, so the chosen row is found again by name.
    pub fn replace_items(&mut self, choices: Choices) {
        let chosen = self.selected_item().map(|item| item.command.clone());
        self.choices = choices;
        self.refilter();
        if let Some(chosen) = chosen {
            if let Some(at) = self
                .matches
                .iter()
                .position(|index| self.showing()[*index].command == chosen)
            {
                self.selected = at;
            }
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// The items the query left, in the order they should be shown.
    pub fn matches(&self) -> Vec<&PickerItem> {
        let showing = self.showing();
        self.matches.iter().map(|index| &showing[*index]).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        self.matches.get(self.selected).map(|index| &self.showing()[*index])
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
    /// context size as readily as on a name — or on whatever text the item says it is found
    /// by, when that is more than the row shows.
    fn refilter(&mut self) {
        let items = match self.scope {
            Scope::All => &self.choices.items,
            Scope::Scoped => &self.choices.scoped,
        };
        let indexed: Vec<usize> = (0..items.len()).collect();
        self.matches = fuzzy::filter(indexed, &self.query, |index| match &items[*index].search {
            Some(search) => search.clone(),
            None => format!("{} {}", items[*index].label, items[*index].detail),
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
        assert_eq!(picker.matches().len(), 3);
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
