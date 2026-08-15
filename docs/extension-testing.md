# Testing extensions

Two harnesses check that extensions work. One runs a real extension in a real micro process
and reads what a person would see. The other drives a real terminal, because whether
something reached the screen is not something an assertion about strings can answer.

## The compatibility sweep

`crates/micro-cli/tests/extension_compatibility.rs` runs every extension in
`examples/extensions` and reports which ones load and run a turn cleanly.

```
cargo test -p micro-cli --test extension_compatibility -- --nocapture
```

It takes a few minutes, because each extension gets its own micro process with its own
scratch home and its own Bun runtime. The output is a table of one row per extension, and a
count at the end.

Each extension is put where it belongs rather than copied into place. A single file goes
into `.micro/extensions`; a package is installed through micro's own install path, the same
way you would install it, so its dependencies are fetched exactly as they would be for you.

Failure is read from the line micro itself prints — `note: <path> was not loaded: <reason>`
— rather than from anything internal. That way a pass in the table means what you would
mean by it.

The extensions are ours, kept in the repository. They came from pi's own examples, which
makes them a fair test: real code written against pi's API by someone who was not thinking
about micro.

## The terminal harness

Some extensions only do anything with a person at a terminal — a widget, a custom editor,
an overlay, a game. Loading proves nothing about those.

`crates/micro-cli/tests/interactive_extension_compatibility.rs` runs micro under a real
pseudo-terminal, sends keystrokes, and looks at what came back. It uses
`crates/micro-cli/tests/pty/drive.py`, which forks a terminal, sizes it, answers the two
questions a real terminal has to answer, sends keys on a clock, and hands back everything
that was drawn.

Those two questions matter. A terminal is asked where the cursor is and what colour the
background is, and micro waits for the answers. A harness that does not send them looks
like a hang.

An extension the harness cannot drive is listed with the reason and not counted as passing.
A caveat is worth more than a green tick that means nothing.

## Reading the screen, not the stream

`crates/micro-cli/tests/inline_mode.rs` checks something the compatibility sweep cannot: not
what micro wrote, but what ended up on the screen.

The distinction is real. When the interface used to clone itself down the screen in inline
mode, the captured output contained exactly one copy of everything — the duplicates were
rows left behind from an earlier frame, and nothing was written for them. Counting matches
in the captured stream would have reported success.

So that test replays the escape sequences into a small grid, applying cursor moves and
erases in order, and reads the finished screen. Any test about what the interface looks like
should do the same.

## Writing your own

The fixtures in `crates/micro-cli/tests/support` stand up a scratch `MICRO_DIR`, a scratch
workspace, and a fake provider that needs no network and no key. Start there rather than
from a real session.

Where an extension needs a terminal, reach for `drive.py`. Where it does not, `--print` is
enough and much faster: it exits non-zero when a command handler throws, so a failure is
something a script can act on.
