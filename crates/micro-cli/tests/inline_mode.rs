//! Regression coverage for `Screen::fit_inline` (`crates/micro-tui/src/lib.rs`): when the inline
//! region needs to grow.

mod support;

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use support::FakeApi;
use support::Fixture;

fn drive_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pty/drive.py")
}

fn pty_command(fixture: &Fixture, micro_args: &[&str]) -> Command {
    let base = fixture.micro();
    let mut command = Command::new("python3");
    command.arg(drive_script());
    command.arg("--");
    command.arg(base.get_program());
    if let Some(dir) = base.get_current_dir() {
        command.current_dir(dir);
    }
    for (key, value) in base.get_envs() {
        match value {
            Some(value) => {
                command.env(key, value);
            }
            None => {
                command.env_remove(key);
            }
        }
    }
    command.args(micro_args);
    command
}

struct Grid {
    rows: usize,
    cols: usize,
    cells: Vec<Vec<char>>,
    row: usize,
    col: usize,
}

impl Grid {
    fn new(rows: usize, cols: usize) -> Grid {
        Grid {
            rows,
            cols,
            cells: vec![vec![' '; cols]; rows],
            row: 0,
            col: 0,
        }
    }

    fn put(&mut self, ch: char) {
        if self.row < self.rows && self.col < self.cols {
            self.cells[self.row][self.col] = ch;
        }
        self.col += 1;
    }

    fn newline(&mut self) {
        self.col = 0;
        if self.row + 1 < self.rows {
            self.row += 1;
        } else {
            self.cells.remove(0);
            self.cells.push(vec![' '; self.cols]);
        }
    }

    /// Erase in display, mode 0: from the cursor to the end of the screen.
    fn erase_to_end_of_screen(&mut self) {
        for cell in self.cells[self.row].iter_mut().skip(self.col) {
            *cell = ' ';
        }
        for line in self.cells.iter_mut().skip(self.row + 1) {
            line.fill(' ');
        }
    }

    fn erase_to_end_of_line(&mut self) {
        for cell in self.cells[self.row].iter_mut().skip(self.col) {
            *cell = ' ';
        }
    }

    fn lines(&self) -> Vec<String> {
        self.cells
            .iter()
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .collect()
    }
}

fn screen(raw: &[u8], rows: usize, cols: usize) -> Vec<String> {
    let mut grid = Grid::new(rows, cols);
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            0x1b if raw.get(i + 1) == Some(&b'[') => {
                let mut j = i + 2;
                while j < raw.len() && !(0x40..=0x7e).contains(&raw[j]) {
                    j += 1;
                }
                let final_byte = raw.get(j).copied();
                let params = String::from_utf8_lossy(&raw[i + 2..j.min(raw.len())]).into_owned();
                match final_byte {
                    Some(b'H') | Some(b'f') => {
                        let mut parts = params.split(';');
                        let r: usize = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
                        let c: usize = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
                        grid.row = r.saturating_sub(1).min(grid.rows.saturating_sub(1));
                        grid.col = c.saturating_sub(1).min(grid.cols.saturating_sub(1));
                    }
                    Some(b'J') if params.is_empty() || params == "0" => {
                        grid.erase_to_end_of_screen();
                    }
                    Some(b'K') if params.is_empty() || params == "0" => {
                        grid.erase_to_end_of_line();
                    }
                    _ => {}
                }
                i = (j + 1).min(raw.len());
            }
            0x1b if raw.get(i + 1) == Some(&b']') => {
                let mut j = i + 2;
                while j < raw.len()
                    && raw[j] != 0x07
                    && !(raw[j] == 0x1b && raw.get(j + 1) == Some(&b'\\'))
                {
                    j += 1;
                }
                i = if raw.get(j) == Some(&0x07) {
                    j + 1
                } else {
                    (j + 2).min(raw.len())
                };
            }
            0x1b if i + 1 < raw.len() => i += 2,
            b'\n' => {
                grid.newline();
                i += 1;
            }
            b'\r' => {
                grid.col = 0;
                i += 1;
            }
            b if b < 0x20 => i += 1,
            _ => {
                let rest = std::str::from_utf8(&raw[i..]).unwrap_or("");
                match rest.chars().next() {
                    Some(ch) => {
                        grid.put(ch);
                        i += ch.len_utf8();
                    }
                    None => i += 1,
                }
            }
        }
    }
    grid.lines()
}

/// Typing `/` opens the full command menu; typing `s` on top of it narrows the match count from all
/// of them down to a few.
#[test]
fn a_menu_that_narrows_replaces_the_wider_one_instead_of_stacking_beneath_it() {
    let api = FakeApi::start([]);
    let fixture = Fixture::new(&api);

    let rows = 15;
    let mut command = pty_command(&fixture, &["-m", "test", "--tui-mode", "regular"]);
    command.env("KEYS", "/~~s");
    command.env("WAIT", "4");
    command.env("GAP", "1.2");
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let output = command.output().expect("run the pty driver");
    if !output.status.success() && output.stdout.is_empty() {
        panic!(
            "the pty driver itself failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let lines = screen(&output.stdout, rows, 100);
    let on_screen = lines.join("\n");

    assert!(
        on_screen.contains("(1/9)") || on_screen.contains("session"),
        "the narrowed menu never reached the screen:\n{on_screen}"
    );
    assert!(
        !on_screen.contains("(1/33)"),
        "the wider, first menu is still on screen alongside the narrower one — the region \
         cloned instead of replacing itself:\n{on_screen}"
    );
}
