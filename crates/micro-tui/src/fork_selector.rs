//! The list `/fork` opens: the prompts of the conversation, any of which a new session can start
//! from.

use micro_commands::ForkPoint;

/// Prompts shown at once before the list scrolls.
pub const MAX_VISIBLE: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkSelector {
    points: Vec<ForkPoint>,
    selected: usize,
}

impl ForkSelector {
    /// The prompts oldest first, with the newest chosen.
    pub fn new(points: Vec<ForkPoint>) -> Self {
        let selected = points.len().saturating_sub(1);
        ForkSelector { points, selected }
    }

    pub fn points(&self) -> &[ForkPoint] {
        &self.points
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Move up, coming round to the bottom from the top.
    pub fn select_previous(&mut self) {
        if self.points.is_empty() {
            return;
        }
        self.selected = match self.selected {
            0 => self.points.len() - 1,
            index => index - 1,
        };
    }

    /// Move down, coming round to the top from the bottom.
    pub fn select_next(&mut self) {
        if self.points.is_empty() {
            return;
        }
        self.selected = (self.selected + 1) % self.points.len();
    }

    /// The command line that forks from the chosen prompt.
    pub fn commit(&self) -> Option<String> {
        self.points
            .get(self.selected)
            .map(|point| format!("/fork {}", point.entry_id))
    }

    /// The prompts to draw: at most `rows` of them, the chosen one kept in the middle.
    pub fn window(&self, rows: usize) -> std::ops::Range<usize> {
        let rows = rows.clamp(1, MAX_VISIBLE).min(self.points.len());
        let start = self
            .selected
            .saturating_sub(rows / 2)
            .min(self.points.len() - rows);
        start..start + rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selector(count: usize) -> ForkSelector {
        ForkSelector::new(
            (1..=count)
                .map(|index| ForkPoint {
                    entry_id: index.to_string(),
                    text: format!("prompt {index}"),
                })
                .collect(),
        )
    }

    #[test]
    fn the_newest_prompt_is_chosen_first() {
        let selector = selector(3);
        assert_eq!(selector.commit().as_deref(), Some("/fork 3"));
    }

    #[test]
    fn moving_past_either_end_comes_round_to_the_other() {
        let mut selector = selector(3);
        selector.select_next();
        assert_eq!(selector.selected(), 0);
        selector.select_previous();
        assert_eq!(selector.selected(), 2);
    }

    #[test]
    fn the_window_keeps_the_chosen_prompt_in_view() {
        let mut selector = selector(30);
        assert_eq!(selector.window(10), 20..30);
        for _ in 0..15 {
            selector.select_previous();
        }
        assert_eq!(selector.selected(), 14);
        assert_eq!(selector.window(10), 9..19);
        assert_eq!(selector.window(50).len(), MAX_VISIBLE);
    }
}
