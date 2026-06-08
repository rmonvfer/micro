//! Gantt charts: sections of tasks laid out as bars along a shared date axis.

use std::collections::{HashMap, HashSet};

use crate::canvas::{draw_text, Canvas};
use crate::labels::{ascii_lower, clean_label, fit_label, strip_controls};
use crate::types::Cls;
use crate::width::string_width;

/// Tasks past this and a chart says nothing useful in a terminal.
const MAX_TASKS: usize = 128;

const MAX_DURATION_DAYS: i64 = 100_000;

/// Refuse to allocate a canvas larger than this many cells, mirroring the bound
/// `layout::layout_canvas` places on graph diagrams.
const MAX_CANVAS_CELLS: usize = 1 << 21;

/// Task/section labels are truncated to this many columns, the same width flowchart node labels
/// wrap to.
const LABEL_MAX: usize = crate::labels::WRAP_WIDTH;

/// Minimum columns between axis ticks: `MM-DD` is five columns wide, plus one column of daylight so
/// adjacent labels never touch.
const TICK_MIN_GAP: usize = 6;

const MAX_TICKS: usize = 12;

struct Task {
    /// The id a later task's `after` can name.
    id: Option<String>,
    label: String,
    /// Day the task begins.
    start: i64,

    end: i64,
    milestone: bool,
    done: bool,
    active: bool,
    crit: bool,
}

struct Group {
    /// `None` for tasks that appear before the first `section` line.
    name: Option<String>,
    tasks: Vec<Task>,
}

struct Chart {
    title: Option<String>,
    groups: Vec<Group>,
}

/// Which days do not count toward a duration: weekends, explicit dates, or both.
#[derive(Default)]
struct Exclusions {
    weekends: bool,
    dates: HashSet<i64>,
}

impl Exclusions {
    fn is_excluded(&self, day: i64) -> bool {
        if self.weekends && matches!(weekday_from_days(day), 0 | 6) {
            return true;
        }
        self.dates.contains(&day)
    }
}

/// Draw `src` as a gantt chart, or answer nothing when it is not one.
pub(crate) fn render_gantt(src: &str) -> Option<Canvas> {
    let chart = parse_gantt(src)?;
    draw_gantt(&chart)
}

fn parse_gantt(src: &str) -> Option<Chart> {
    let src = strip_controls(src);
    let statements = crate::parse::statements_of(&src);
    let header = statements.first()?;
    if ascii_lower(header.split_whitespace().next().unwrap_or("")) != "gantt" {
        return None;
    }

    let mut title = None;
    let mut excl = Exclusions::default();
    let mut groups: Vec<Group> = vec![Group {
        name: None,
        tasks: Vec::new(),
    }];
    let mut by_id: HashMap<String, (i64, i64)> = HashMap::new();
    let mut task_count = 0usize;

    for st in &statements[1..] {
        let word = st.split_whitespace().next()?;
        let rest = st[word.len()..].trim();
        match ascii_lower(word).as_str() {
            "title" => title = Some(clean_label(rest)),
            "dateformat" => {
                if rest != "YYYY-MM-DD" {
                    return None;
                }
            }

            "axisformat" => {}
            "excludes" => {
                for token in rest.split(',') {
                    let token = token.trim();
                    if ascii_lower(token) == "weekends" {
                        excl.weekends = true;
                    } else {
                        excl.dates.insert(parse_date(token)?);
                    }
                }
            }
            "section" => {
                if rest.is_empty() {
                    return None;
                }
                groups.push(Group {
                    name: Some(clean_label(rest)),
                    tasks: Vec::new(),
                });
            }
            _ => {
                if task_count >= MAX_TASKS {
                    return None;
                }
                let task = parse_task(st, &by_id, &excl)?;
                if let Some(id) = &task.id {
                    by_id.insert(id.clone(), (task.start, task.end));
                }
                task_count += 1;
                groups.last_mut().unwrap().tasks.push(task);
            }
        }
    }

    if task_count == 0 {
        return None;
    }
    Some(Chart { title, groups })
}

/// One task line: `label :[tags,] [id,] start, duration`.
fn parse_task(st: &str, by_id: &HashMap<String, (i64, i64)>, excl: &Exclusions) -> Option<Task> {
    let (label, rest) = st.split_once(':')?;
    let label = clean_label(label.trim());
    if label.is_empty() {
        return None;
    }

    let mut done = false;
    let mut active = false;
    let mut crit = false;
    let mut milestone = false;
    let mut fields: Vec<&str> = Vec::new();
    for token in rest.split(',') {
        let token = token.trim();
        match ascii_lower(token).as_str() {
            "done" => done = true,
            "active" => active = true,
            "crit" => crit = true,
            "milestone" => milestone = true,
            "" => {}
            _ => fields.push(token),
        }
    }
    let (id, start_spec, dur_spec) = match fields.as_slice() {
        [start_spec, dur_spec] => (None, *start_spec, *dur_spec),
        [id, start_spec, dur_spec] => (Some((*id).to_string()), *start_spec, *dur_spec),
        _ => return None,
    };

    let start = match strip_after(start_spec) {
        Some(after_id) => by_id.get(after_id)?.1,
        None => parse_date(start_spec)?,
    };
    let duration = parse_duration(dur_spec)?;
    if !(0..=MAX_DURATION_DAYS).contains(&duration) {
        return None;
    }
    let end = add_working_days(start, duration, excl);

    Some(Task {
        id,
        label,
        start,
        end,
        milestone,
        done,
        active,
        crit,
    })
}

/// `after <id>`, split at the word boundary so `afterthought` is not misread as `after thought`.
fn strip_after(spec: &str) -> Option<&str> {
    let rest = spec.strip_prefix("after")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let id = rest.trim();
    if id.is_empty() {
        None
    } else {
        Some(id)
    }
}

/// A literal `YYYY-MM-DD` date.
fn parse_date(token: &str) -> Option<i64> {
    let mut parts = token.splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// `30d`, `2w` or `6h`, in days.
fn parse_duration(token: &str) -> Option<i64> {
    if token.len() < 2 {
        return None;
    }
    let (num, unit) = token.split_at(token.len() - 1);
    let n: i64 = num.parse().ok()?;
    if n < 0 {
        return None;
    }
    match unit {
        "d" => Some(n),
        "w" => Some(n * 7),
        "h" => Some((n + 23) / 24),
        _ => None,
    }
}

/// Step forward `n` days that are not excluded, returning the day landed on.
fn add_working_days(start: i64, mut n: i64, excl: &Exclusions) -> i64 {
    let mut day = start;
    while n > 0 {
        day += 1;
        if !excl.is_excluded(day) {
            n -= 1;
        }
    }
    day
}

/// Days since 1970-01-01 for a Gregorian calendar date.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of `days_from_civil`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn weekday_from_days(days: i64) -> i64 {
    if days >= -4 {
        (days + 4) % 7
    } else {
        (days + 5) % 7 + 6
    }
}

fn draw_gantt(chart: &Chart) -> Option<Canvas> {
    let tasks: Vec<&Task> = chart.groups.iter().flat_map(|g| g.tasks.iter()).collect();
    let min_start = tasks.iter().map(|t| t.start).min()?;

    let max_end = tasks.iter().map(|t| t.end.max(t.start + 1)).max()?;
    let total_days = (max_end - min_start).max(1) as usize;

    let label_w = label_column_width(chart);
    let grid_x = label_w + 1;
    let ticks = axis_ticks(min_start, total_days);

    let mut rows = usize::from(chart.title.is_some());
    for g in chart.groups.iter().filter(|g| !g.tasks.is_empty()) {
        rows += usize::from(g.name.is_some()) + g.tasks.len();
    }
    rows += 2;

    let last_tick_end = ticks
        .last()
        .map(|(offset, text)| grid_x + offset + string_width(text))
        .unwrap_or(grid_x);
    let width = (grid_x + total_days)
        .max(last_tick_end)
        .max(string_width(chart.title.as_deref().unwrap_or("")));

    if width.saturating_mul(rows) > MAX_CANVAS_CELLS {
        return None;
    }

    let mut canvas = Canvas::new(width, rows);
    let mut y = 0;
    if let Some(title) = &chart.title {
        draw_text(&mut canvas, title, 0, y, Cls::Title);
        y += 1;
    }

    for g in chart.groups.iter().filter(|g| !g.tasks.is_empty()) {
        if let Some(name) = &g.name {
            draw_text(&mut canvas, &fit_label(name, LABEL_MAX), 0, y, Cls::Text);
            y += 1;
        }
        for t in &g.tasks {
            draw_text(
                &mut canvas,
                &fit_label(&t.label, LABEL_MAX),
                2,
                y,
                Cls::Text,
            );
            let offset = (t.start - min_start) as usize;
            if t.milestone {
                draw_text(&mut canvas, "◆", grid_x + offset, y, Cls::Border);
            } else {
                let glyph = if t.done {
                    "█"
                } else if t.active {
                    "▒"
                } else if t.crit {
                    "▓"
                } else {
                    "░"
                };
                let width_days = (t.end - t.start).max(1) as usize;
                let bar = glyph.repeat(width_days);
                draw_text(&mut canvas, &bar, grid_x + offset, y, Cls::Border);
            }
            y += 1;
        }
    }

    let axis_y = rows - 2;
    let label_y = rows - 1;
    for x in 0..total_days {
        canvas.set(grid_x + x, axis_y, "─", Cls::Edge);
    }
    for (offset, text) in &ticks {
        canvas.set(grid_x + offset, axis_y, "┬", Cls::Edge);
        draw_text(&mut canvas, text, grid_x + offset, label_y, Cls::EdgeLabel);
    }

    Some(canvas)
}

fn label_column_width(chart: &Chart) -> usize {
    let mut width = 0usize;
    for g in &chart.groups {
        if let Some(name) = &g.name {
            width = width.max(string_width(&fit_label(name, LABEL_MAX)));
        }
        for t in &g.tasks {
            width = width.max(string_width(&fit_label(&t.label, LABEL_MAX)) + 2);
        }
    }
    width
}

fn axis_ticks(min_start: i64, total_days: usize) -> Vec<(usize, String)> {
    let step = TICK_MIN_GAP.max(total_days / MAX_TICKS + 1);
    let mut ticks = Vec::new();
    let mut offset = 0usize;
    while offset < total_days {
        let (_, m, d) = civil_from_days(min_start + offset as i64);
        ticks.push((offset, format!("{m:02}-{d:02}")));
        offset += step;
    }
    ticks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(src: &str) -> Vec<String> {
        render_gantt(src)
            .expect("it is a gantt chart")
            .to_lines()
            .plain
    }

    #[test]
    fn civil_dates_round_trip_and_know_their_weekday() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(weekday_from_days(0), 4, "1970-01-01 was a Thursday");

        for day in [-100_000i64, -1, 0, 1, 400, 40_000, 800_000] {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day, "{day} -> {y}-{m}-{d}");
        }
    }

    /// A single task draws its label and a bar the width of its duration, starting at the axis
    /// origin.
    #[test]
    fn a_task_is_drawn_as_a_bar_on_the_date_axis() {
        let rows = drawn("gantt\n  dateFormat YYYY-MM-DD\n  Design :des1, 2024-01-01, 3d");
        assert_eq!(
            rows,
            vec!["  Design ░░░", "         ┬──", "         01-01",]
        );
    }

    /// `done`, `active` and `crit` each draw in their own bar glyph, so three tasks in three
    /// different states are told apart at a glance.
    #[test]
    fn task_status_changes_the_bar_glyph() {
        let rows = drawn(
            "gantt\n\
             dateFormat YYYY-MM-DD\n\
             Finished :done, 2024-01-01, 2d\n\
             Ongoing  :active, 2024-01-01, 2d\n\
             Risky    :crit, 2024-01-01, 2d",
        );
        assert!(rows[0].contains("██"), "{rows:?}");
        assert!(rows[1].contains("▒▒"), "{rows:?}");
        assert!(rows[2].contains("▓▓"), "{rows:?}");
    }

    #[test]
    fn a_milestone_draws_as_a_single_marker() {
        let rows = drawn("gantt\n  dateFormat YYYY-MM-DD\n  Ship :milestone, m1, 2024-01-05, 0d");
        assert!(rows[0].contains('◆'), "{rows:?}");
    }

    #[test]
    fn after_chains_to_the_end_of_the_named_task_honouring_excluded_weekends() {
        let rows = drawn(
            "gantt\n\
             dateFormat YYYY-MM-DD\n\
             excludes weekends\n\
             Design :des1, 2024-01-05, 3d\n\
             Review  :des2, after des1, 1d",
        );

        assert!(rows[0].contains("░░░░░"), "{rows:?}");
        let review = &rows[1];
        let bar_col = review.find('░').expect("des2 has a bar");
        let design = &rows[0];
        let design_col = design.find('░').expect("des1 has a bar");
        assert_eq!(bar_col, design_col + 5, "{rows:?}");
    }

    /// Sections group their tasks under a heading, drawn above them.
    #[test]
    fn a_section_heading_is_drawn_above_its_tasks() {
        let rows = drawn(
            "gantt\n\
             dateFormat YYYY-MM-DD\n\
             section Build\n\
             Compile :2024-01-01, 1d\n\
             section Ship\n\
             Release :2024-01-02, 1d",
        );
        assert_eq!(rows[0], "Build");
        assert!(rows[1].trim_start().starts_with("Compile"), "{rows:?}");
        assert_eq!(rows[2], "Ship");
        assert!(rows[3].trim_start().starts_with("Release"), "{rows:?}");
    }

    #[test]
    fn what_is_not_a_gantt_chart_is_left_alone() {
        assert!(render_gantt("graph TD\n A --> B").is_none());
        assert!(render_gantt("gantt").is_none(), "no tasks at all");
        assert!(
            render_gantt("gantt\n  dateFormat DD/MM/YYYY\n  A :2024-01-01, 1d").is_none(),
            "an unsupported date format"
        );
        assert!(
            render_gantt("gantt\n  Bad Task Line With No Colon").is_none(),
            "a task line without a colon"
        );
        assert!(
            render_gantt("gantt\n  A :2024-01-01, 3x").is_none(),
            "an unrecognised duration unit"
        );
        assert!(
            render_gantt("gantt\n  A :id1, after nobody, 1d").is_none(),
            "after referencing a task that was never declared"
        );
    }

    #[test]
    fn too_many_tasks_are_refused() {
        let mut source = String::from("gantt\n  dateFormat YYYY-MM-DD\n");
        for index in 0..200 {
            source.push_str(&format!("Task {index} :2024-01-01, 1d\n"));
        }
        assert!(render_gantt(&source).is_none());
    }
}
