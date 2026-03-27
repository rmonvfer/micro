//! Gantt charts, exercised through the public `render` entry point the way
//! `tests/render.rs` exercises every other kind.

use micro_mermaid::{diagram_kind, render};

fn plain(src: &str) -> Vec<String> {
    render(src).expect("drawn").plain
}

/// A task draws as a bar the width of its duration, sitting on a date axis
/// that begins the day the task does.
#[test]
fn a_task_draws_a_bar_on_the_date_axis() {
    let src = "gantt\n  dateFormat YYYY-MM-DD\n  Design :des1, 2024-01-01, 3d";
    assert_eq!(
        plain(src),
        vec!["  Design ░░░", "         ┬──", "         01-01",]
    );
}

/// `after` chains a task to the end of the one it names, and `excludes
/// weekends` is honoured when turning a duration into an end date.
#[test]
fn after_chains_to_the_end_of_the_named_task() {
    let src = "gantt\n\
               dateFormat YYYY-MM-DD\n\
               excludes weekends\n\
               Design :des1, 2024-01-05, 3d\n\
               Review  :des2, after des1, 1d";
    let rows = plain(src);
    let design_col = rows[0].find('░').expect("des1 has a bar");
    let review_col = rows[1].find('░').expect("des2 has a bar");
    // A Friday 3-day task crosses a weekend, so 5 calendar columns separate
    // where it starts from where the task after it starts.
    assert_eq!(review_col, design_col + 5, "{rows:?}");
}

/// An unparseable gantt chart renders nothing, the same way an unparseable
/// class diagram does.
#[test]
fn an_unparseable_gantt_chart_renders_nothing() {
    let src = "gantt\n  Not a task line";
    assert!(render(src).is_none());
    // `diagram_kind` still recognises the header, which is what tells a
    // syntax error apart from a diagram type this renderer does not draw.
    assert!(diagram_kind(src).is_some());
}
