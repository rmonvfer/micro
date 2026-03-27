//! Composing an interface out of pieces that measure themselves.
//!
//! The alternate screen has a fixed shape, so it can be laid out by naming regions and
//! their sizes up front. Drawing inline into the terminal's own scrollback cannot: the
//! interface is only as tall as what it holds, and what it holds changes every frame. So
//! a piece has to be able to say how tall it is at a given width before there is anywhere
//! to put it.
//!
//! That is the whole contract. A [`Component`] renders itself to lines for a width;
//! containers render their children and stack the results. Everything else — the fixed
//! rows, the flexible ones, the caching — falls out of that.
//!
//! A component is measured once per width and the result kept, because measuring means
//! wrapping text and a frame asks several times.

use ratatui::text::Line;
use std::cell::RefCell;
use std::collections::HashMap;

/// Something that can draw itself at a given width.
pub trait Component {
    /// The lines this occupies at `width`.
    fn render(&self, width: usize) -> Vec<Line<'static>>;

    /// How tall this is at `width`.
    ///
    /// The default renders and counts, which is always right. A component that knows its
    /// height without building the lines says so instead, because a stack asks every
    /// child how tall it is before it draws any of them.
    fn height(&self, width: usize) -> usize {
        self.render(width).len()
    }
}

/// Lines held as they are, which is what most of micro's renderers already produce.
pub struct Lines(pub Vec<Line<'static>>);

impl Component for Lines {
    fn render(&self, _width: usize) -> Vec<Line<'static>> {
        self.0.clone()
    }

    fn height(&self, _width: usize) -> usize {
        self.0.len()
    }
}

/// Blank rows, for holding a gap open.
pub struct Spacer(pub usize);

impl Component for Spacer {
    fn render(&self, _width: usize) -> Vec<Line<'static>> {
        vec![Line::from(String::new()); self.0]
    }

    fn height(&self, _width: usize) -> usize {
        self.0
    }
}

/// How much room a child gets.
pub enum Sizing {
    /// Whatever it says it needs.
    Content,
    /// Whatever is left after the others, shared by weight.
    Flexible(usize),
}

/// A child and how it is sized.
pub struct Child {
    pub component: Box<dyn Component>,
    pub sizing: Sizing,
}

impl Child {
    pub fn content(component: impl Component + 'static) -> Self {
        Child {
            component: Box::new(component),
            sizing: Sizing::Content,
        }
    }

    pub fn flexible(component: impl Component + 'static, weight: usize) -> Self {
        Child {
            component: Box::new(component),
            sizing: Sizing::Flexible(weight.max(1)),
        }
    }
}

/// Children stacked one above the next.
#[derive(Default)]
pub struct Stack {
    children: Vec<Child>,
    /// The rows available, when there is a limit. Without one the stack is as tall as its
    /// children, which is what drawing inline needs.
    available: Option<usize>,
    cache: RefCell<HashMap<usize, Vec<Line<'static>>>>,
}

impl Stack {
    pub fn new() -> Self {
        Stack::default()
    }

    /// A stack that must fit in `rows`, which is what a fixed screen gives it.
    pub fn within(rows: usize) -> Self {
        Stack {
            available: Some(rows),
            ..Stack::default()
        }
    }

    pub fn with(mut self, child: Child) -> Self {
        self.children.push(child);
        self.cache.borrow_mut().clear();
        self
    }

    /// How many rows each child gets, for a caller drawing them itself.
    pub fn allocation(&self, width: usize) -> Vec<usize> {
        self.allocate(width)
    }

    /// How many rows each child gets.
    ///
    /// Every child that measures itself takes what it needs before anything flexible does;
    /// whatever is left is shared between the flexible ones by weight. Nothing left means a
    /// flexible child gets nothing, which is what makes a small terminal drop the
    /// conversation rather than the prompt.
    fn allocate(&self, width: usize) -> Vec<usize> {
        let mut rows: Vec<usize> = self
            .children
            .iter()
            .map(|child| match child.sizing {
                Sizing::Content => child.component.height(width),
                Sizing::Flexible(_) => 0,
            })
            .collect();

        let Some(available) = self.available else {
            // No limit, so a flexible child is as tall as it wants to be.
            for (index, child) in self.children.iter().enumerate() {
                if matches!(child.sizing, Sizing::Flexible(_)) {
                    rows[index] = child.component.height(width);
                }
            }
            return rows;
        };

        let taken: usize = rows.iter().sum();
        let mut left = available.saturating_sub(taken);

        let weights: usize = self
            .children
            .iter()
            .filter_map(|child| match child.sizing {
                Sizing::Flexible(weight) => Some(weight),
                _ => None,
            })
            .sum();

        if weights > 0 {
            let mut given = 0;
            let flexible: Vec<usize> = self
                .children
                .iter()
                .enumerate()
                .filter(|(_, child)| matches!(child.sizing, Sizing::Flexible(_)))
                .map(|(index, _)| index)
                .collect();

            for (position, index) in flexible.iter().enumerate() {
                let weight = match self.children[*index].sizing {
                    Sizing::Flexible(weight) => weight,
                    _ => 1,
                };
                // The last one takes the remainder, so no row is lost to rounding.
                let share = match position + 1 == flexible.len() {
                    true => left - given,
                    false => left * weight / weights,
                };
                rows[*index] = share;
                given += share;
            }
            left = 0;
        }

        // Still over budget, so the earliest children give way: the prompt and the footer
        // are at the bottom, and they are what has to stay.
        let mut over = rows.iter().sum::<usize>().saturating_sub(available);
        for row in rows.iter_mut() {
            if over == 0 {
                break;
            }
            let taken = (*row).min(over);
            *row -= taken;
            over -= taken;
        }
        let _ = left;
        rows
    }
}

impl Component for Stack {
    fn render(&self, width: usize) -> Vec<Line<'static>> {
        if let Some(cached) = self.cache.borrow().get(&width) {
            return cached.clone();
        }

        let rows = self.allocate(width);
        let mut out = Vec::new();
        for (child, allotted) in self.children.iter().zip(rows) {
            if allotted == 0 {
                continue;
            }
            let mut drawn = child.component.render(width);
            // A child shows its end rather than its beginning when it does not fit: what
            // was said most recently is what a reader is looking at.
            if drawn.len() > allotted {
                drawn = drawn.split_off(drawn.len() - allotted);
            }
            while drawn.len() < allotted {
                drawn.push(Line::from(String::new()));
            }
            out.extend(drawn);
        }

        self.cache.borrow_mut().insert(width, out.clone());
        out
    }

    fn height(&self, width: usize) -> usize {
        match self.available {
            Some(rows) => rows,
            None => self.allocate(width).iter().sum(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(count: usize) -> Lines {
        Lines(
            (0..count)
                .map(|index| Line::from(format!("line {index}")))
                .collect(),
        )
    }

    fn rendered(component: &dyn Component, width: usize) -> Vec<String> {
        component
            .render(width)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    /// Without a limit a stack is as tall as what it holds, which is what drawing inline
    /// needs: the interface occupies only the rows it is using.
    #[test]
    fn an_unbounded_stack_is_as_tall_as_its_children() {
        let stack = Stack::new()
            .with(Child::content(text(3)))
            .with(Child::content(Spacer(1)))
            .with(Child::content(text(2)));

        assert_eq!(stack.height(80), 6);
        assert_eq!(rendered(&stack, 80).len(), 6);
    }

    /// The flexible child takes what the others left.
    #[test]
    fn what_is_left_goes_to_the_flexible_child() {
        let stack = Stack::within(10)
            .with(Child::flexible(text(50), 1))
            .with(Child::content(text(2)));

        let drawn = rendered(&stack, 80);
        assert_eq!(drawn.len(), 10);
        // The flexible child shows its end: the most recent eight of its fifty lines.
        assert_eq!(drawn[0], "line 42");
        assert_eq!(drawn[7], "line 49");
        assert_eq!(drawn[8], "line 0", "then the fixed child");
    }

    /// Two flexible children share what is left by weight.
    #[test]
    fn flexible_children_share_by_weight() {
        let stack = Stack::within(9)
            .with(Child::flexible(text(50), 2))
            .with(Child::flexible(text(50), 1));

        let drawn = rendered(&stack, 80);
        assert_eq!(drawn.len(), 9, "every row is used");
    }

    /// A terminal too small keeps the bottom: the prompt and the footer are what a reader
    /// is using, and the conversation is what gives way.
    #[test]
    fn the_bottom_survives_a_small_terminal() {
        let stack = Stack::within(2)
            .with(Child::content(text(5)))
            .with(Child::content(text(2)));

        let drawn = rendered(&stack, 80);
        assert_eq!(drawn.len(), 2);
        assert_eq!(drawn, vec!["line 0", "line 1"], "the last child stayed");
    }

    /// Measuring means wrapping, and a frame asks more than once.
    #[test]
    fn a_measured_stack_is_only_built_once_per_width() {
        let stack = Stack::new().with(Child::content(text(3)));
        let first = stack.render(80);
        let second = stack.render(80);
        assert_eq!(first, second);
        assert_eq!(stack.cache.borrow().len(), 1);
        // A different width is measured separately.
        stack.render(40);
        assert_eq!(stack.cache.borrow().len(), 2);
    }
}
