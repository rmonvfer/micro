# The sandbox

A coding agent runs commands. The sandbox decides what those commands may touch, and the
operating system is what enforces it: by default a session may write inside its workspace
and nowhere else, and may not reach the network. Every refusal is told to the model in terms
it can act on and written into the session's ledger.

## The three policies

`read-only` reads anything and writes nothing. `workspace-write` — the default — reads
anything and writes inside the workspace, minus the paths listed below, with the network
off. `full` confines nothing: whatever micro runs, your account can do.

Reading is unrestricted under every policy. An agent that cannot read the system it is
working on is useless, and the damage worth preventing — writing outside the workspace,
reaching the network — is on the other side.

Inside a writable workspace, `.git` and `.micro` stay read-only, whether or not they exist
yet. A process that can write a git hook decides what runs on your next commit, and one
that can write `.micro/settings.json` decides what the next session is allowed to do; both
would turn a confined run into an unconfined one, one step later. micro's own directories —
`~/.micro`, or the XDG directories on a fresh install — are read-only for the same reason
wherever they fall inside a writable root, which is what happens when you work on your own
configuration.

That has a consequence worth knowing before you meet it: under `workspace-write` the agent
can edit your files but cannot commit them, because committing writes inside `.git`. Ask
for the commit yourself, or run that session under `full`.

There is no grant for `/tmp` or `$TMPDIR`. A toolchain that insists on a scratch directory
outside the workspace will notice; give it one explicitly rather than leaving every
session's commands a shared directory to write through.

## Saying which one

```
micro --sandbox read-only              for this run
micro sandbox try -- <command>         what would happen to this command
/set sandbox read-only                 for every run
```

Three places can settle the policy, each beating the one after it: `--sandbox` on the
command line, then `sandbox` in the project's `.micro/settings.json`, then `sandbox` in your
own `config.json`. With nothing said anywhere it is `workspace-write`.

A project's settings are read only once the project has been trusted, like everything else
it ships. Widening what a session may do is exactly what trust is asked about, so a checkout
nobody has vouched for gets no say.

A policy name nobody recognizes ends the run and says who asked for it. Falling back to a
default would run the session under something other than what was asked for, which is the
one outcome a policy exists to prevent.

`workspace-write` can also be spelled out, to grant more than the workspace:

```json
{ "sandbox": { "mode": "workspace-write", "writable_roots": ["/srv/cache"], "allow_network": true } }
```

That is how a project gives a toolchain the scratch directory it insists on, and it is what
`--sandbox` takes as well — a value starting with `{` is read as this table rather than as
a name.

## Two enforcers

A command micro spawns is wrapped before it runs, so the kernel decides what it may touch.
The file tools spawn nothing, so no kernel is in a position to stop them: they ask the same
policy about every path before they open it, and a symlink pointing out of the workspace is
judged by where it points rather than where it sits. One policy, answered in two places,
recorded in one.

| | macOS | Linux | Windows |
|---|---|---|---|
| Commands | Seatbelt profile | Landlock and seccomp | not yet |
| File tools | in-process checks | in-process checks | in-process checks |
| Network off | yes | yes | not yet |

On Windows, and anywhere else micro has no sandbox for, the file tools still enforce the
policy in process and commands run unconfined. `micro sandbox try` says so rather than
implying confinement it does not have.

The two answers are meant to agree, and differ in one place: Landlock can only rule on a
path that exists, so a protected directory that has not been created yet — a workspace with
no `.git` in it — is refused in process but not by the Linux kernel rules. Seatbelt takes
the path as a string and has no such gap.

## What is not confined

The policy covers what a session runs on the model's behalf: the `bash` tool, the file
tools, and anything an extension asks micro to run through `ctx.exec` or one of micro's own
built-in tools. It does not cover the programs you configured micro to start — the
extension host and any MCP servers — which are launched once at startup, from your own
settings, and run as you do. Nor does it cover a command you type yourself with `!`, which
is you running a command in your own terminal rather than the agent running one.

## The Linux helper

macOS confines a command from the outside. Linux cannot: the restrictions have to be applied
by the process that then becomes the command. So on Linux micro re-runs itself as that
process —

```
/path/to/micro __micro-sandbox-helper --rules <json> -- /bin/bash -c ls
```

— and the second run recognizes that first argument before it parses anything else, applies
the rules, and `execvp`s the command. The rules are worked out by the parent, where the
policy and the workspace are known, and handed over whole; the helper resolves nothing
itself. A build that did not dispatch on that argument would leave every command unconfined,
which is why it happens at the top of `main` rather than anywhere a failure could get past
it.

## What a refusal looks like

The model is told which policy stopped it, along with everything the command printed:

```
denied by policy workspace-write: exit code 1
touch: /etc/hosts: Operation not permitted
```

A file tool refuses before it opens anything, and says what the policy allows instead:

```
cannot write /etc/hosts: workspace-write allows writes under /home/you/project only
```

An extension that runs a command through `ctx.exec` gets the same judgment as two extra
fields on the answer it already receives, `denied: true` and `policy`, so it can act on a
refusal without reading the platform's wording out of `stderr` itself.

One shape follows from that. What micro spawns is the wrapper rather than the command
itself, so the wrapper is what reports a command it could not run: a binary that does not
exist comes back as an ordinary result with a non-zero `code` and a `stderr` naming the
command, rather than as an `error` field with no exit code. An extension deciding whether a
command worked reads the exit code, not the absence of an error.

There is no reliable signal for "the kernel refused this". A command that fails inside your
shell profile looks much like one that was turned down, so micro reads the exit status for
the signal seccomp raises and otherwise falls back to the wording the platforms use. It
decides how a failure is phrased, never whether something was allowed.

Every refusal becomes a `sandbox_decision` in the session's [ledger](ledger.md), with the
policy, the operation, what was being reached for, and which way it went. Ordinary work that
went through is not recorded; the ledger is for what did not. Two allowances are worth a
line all the same: a
command that looks refused while nothing was confining it, which is otherwise an afternoon
of confusion, and the start of a session running under `full`, which is said on screen at
the same time. Running unconfined is never quiet.

## Trying it out

```
$ micro sandbox try -- touch ../outside.txt
policy: workspace-write
enforced: yes, by a Seatbelt profile
running: /usr/bin/sandbox-exec -p <131 lines of policy> -DWRITABLE_ROOT_0=/home/you/project
  -DWRITABLE_ROOT_0_READ_ONLY_0=/home/you/project/.git
  -DWRITABLE_ROOT_0_READ_ONLY_1=/home/you/project/.micro -- touch ../outside.txt
output:
  touch: ../outside.txt: Operation not permitted
exit: exit status: 1
looks denied: true
```

It resolves the policy the same way a session does, so it answers what this workspace would
actually do rather than what a default would.

The profile itself is summarized by its length; the paths it is parameterized with are
printed in full, since those are what there is to check. Paths never go into the profile
text — they are passed as parameters, so a directory whose name contains policy syntax is a
directory name and not a policy.

## Checking it on macOS

The automated tests cover this platform when they run on it, and a checkout without a macOS
machine behind it cannot. This is the pass to make by hand. In a scratch workspace, with
somewhere outside it to aim at:

1. `micro sandbox try -- touch ../outside.txt` refuses, and the file is not created.
2. `micro sandbox try -- touch inside.txt` succeeds, and the file is.
3. `micro sandbox try -- curl -s -o /dev/null -w '%{http_code}' https://example.com` fails
   to resolve the host; the same command under
   `--sandbox '{"mode":"workspace-write","allow_network":true}'` answers `200`.
4. `micro sandbox try --sandbox read-only -- touch inside.txt` refuses.
5. `micro sandbox try --sandbox full -- touch ../outside.txt` succeeds and reports
   `enforced: no`.
6. In a session: ask for a shell command that writes outside the workspace. The model is
   told `denied by policy workspace-write`, and `micro sessions export <id>` holds a
   `sandbox_decision` with `"allowed":false`.

## Windows

Not yet. The policy resolves, the file tools enforce it, and commands run unconfined.
A Windows sandbox is a job for restricted tokens and AppContainer, and it is on the roadmap
rather than in the binary.
