# Changelog

## 0.1.0

First release.

A coding agent for the terminal, in one binary with no runtime to install.

Talks to OpenRouter, GitHub Copilot, Google Gemini and Anthropic, with API keys or a
device sign-in, and switches between them mid-conversation without losing what was said.

Every conversation is recorded as it happens and reopens exactly as it was left, branches
included: going back to an earlier point keeps the answer that was there before.

The same log carries a ledger of everything else a run did: the exact request each turn
sent, with every stretch of the prompt attributed to whatever supplied it, what the provider
said it cost, what the sandbox refused, and what an extension asked for and was told.
`micro sessions show <id> --turn N --raw` rebuilds a request out of that record and checks
it against the hash of the body that went out, and `micro sessions export` prints the whole
ledger in a documented, versioned format.

`micro bill` itemizes what a session cost, turn by turn and source by source, pricing cache
reads, cache writes and fresh input separately; `/bill` reads the same from inside a
session, and `--diff` answers what one turn added and why. `--budget`, or a `budget`
setting, stops a session at the first turn boundary past what it was allowed to spend, and
says so in the ledger.

The head of a request is meant to stand still so that a provider can reuse it. Anything that
would move it is taken up at a turn boundary and recorded with its reason, and
`micro why-miss` prints the span that moved, the lines that differ, and the reason it was
recorded under.

Commands run under an operating-system sandbox: the workspace is writable, nothing else is,
the network is off, and `.git` and micro's own directories stay read-only inside a writable
workspace. `--sandbox` and a `sandbox` setting choose the policy for a run or for a project,
`micro sandbox try` says what would become of a command before you spend a turn on it, and
every refusal reaches both the model and the ledger.

A project is vouched for once with `/trust` rather than every time, and that one decision
is what settles whether micro will run the code the project ships. Tool calls themselves
are not gated after that: once micro is running, it acts.

Extensions run in a Bun process of their own and reach micro only by asking. An extension
declares what it needs — tools, commands, exec, the interface — and an ask outside that is
refused by a name it can catch while the session carries on; one that declares nothing is
asked about once and the answer remembered. `micro install` fetches a package with its
dependencies, `micro list` shows what each one may do, and an extension may export a
`deactivate` to put back what it changed outside micro.

Tools from MCP servers reach the model beside micro's own. Past a threshold, the extra ones
are described through a search tool rather than in every request, so a shelf of servers
costs a lookup instead of a standing share of the context window.

Skills are read from the workspace and from micro's own directory, announced to the model
by name so it reaches for one only when it applies.

A session can be handed to a paired phone with `/remote` and read and driven from there
while the terminal stays fully usable. What crosses the relay is ciphertext the relay
cannot open.

Where micro keeps things follows the XDG base directory specification, with what you wrote
kept apart from what micro produced. `MICRO_DIR` puts all of it in one named directory, and
an existing `~/.micro` keeps holding everything it already held.
