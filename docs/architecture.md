# Architecture

Fourteen crates, arranged so that nothing depends on a layer above it. This document is
about the seams rather than the contents: where the boundaries are, and what each one buys.

## The graph

`micro-types` sits at the bottom and depends on nothing. It holds the conversation model —
content blocks, messages, usage, stop reasons — plus the two event enums every other layer
speaks: `StreamEvent`, what a provider emits while a response arrives, and `AgentEvent`,
what the loop emits to whatever is driving it.

Above it, four crates are independent of each other. `micro-provider` turns a `Context` into
an HTTP request and the response body into a stream of events. `micro-tools` holds the
capabilities the model can invoke. `micro-auth` holds credentials. `micro-context` assembles
project instructions and compacts a conversation that is outgrowing its window.
`micro-models` is the catalog of which models exist, what they cost, and where they live.

`micro-agent` depends on the provider, the tools, and the context crates, and runs the loop
that ties them together.
`micro-session` writes conversations to disk. `micro-tui` draws the interface over an agent.
`micro-cli` is the entry point that assembles all of it. `micro-testkit` provides the test
doubles the agent loop is exercised against, and `micro-config` and `micro-commands` hold
persisted settings and slash-command parsing.

## Providers know nothing about tools

`micro-provider`'s entire interface is one method:

```rust
pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn stream(&self, model: Model, context: Context, api_key: String)
        -> UnboundedReceiver<StreamEvent>;
    fn payload(&self, model: &Model, context: &Context) -> serde_json::Value;
}
```

`payload` is the same assembly `stream` sends, without sending it, which is what lets a
session record a request by its hash and rebuild it afterwards. See [the ledger](ledger.md).

A `Context` carries a system prompt, a list of messages, and a list of `ToolDefinition`s —
names, descriptions, and JSON Schemas. That is a description of what the model may ask for,
not a way to do any of it. No provider can execute a tool, and none of them knows that
executing one is even possible.

This is what keeps three wire formats from turning into three agent loops. Anthropic
Messages, OpenAI chat completions, and Google's generative API disagree about almost
everything — how a tool call is spelled, how a streamed response is framed, where usage is
reported — but they agree on the shape of the problem: here is a conversation and a set of
tools, produce the next message. Each provider translates in both directions at its own
edge, and the loop above sees one vocabulary. Adding a provider adds a translation, never a
branch in the agent.

It also means the agent decides what a tool call means. When a response stops because it hit
the output token limit, its tool calls are failed without being executed: streamed arguments
truncated mid-JSON can still parse into something plausible, and running that is worse than
asking the model to try again. That judgment lives in one place because the providers do not
get a vote.

## Trusting a project is decided once, before its code is read

A project can carry code micro would otherwise run without being asked: the extensions it
ships, the skills and prompts it offers, the settings it sets. That decision is made once,
in `main`, before anything of the project's is loaded:

```rust
let trusted = project_trusted(&root, &settings, !cli.print && !cli.rpc).await;
let built = runtime::build(&root, &selection, resume, &settings, trusted).await?;
```

Loading is where the gate belongs rather than at each point of use, because a skill that was
never read cannot steer a turn and an extension that was never started cannot register a
tool. Discovery takes the answer and skips the project's own directory, so nothing
downstream needs to know a decision was made.

A project carrying none of it is not asked about at all, which is what keeps the question
rare enough to mean something. Tool calls are not gated: `micro-tools` hands the agent a
list, `--tools` and `--exclude-tools` decide what is on it, and everything on it runs.

## Persistence goes through a recorder channel

The agent takes an optional `UnboundedSender<Message>` and sends every finalized message the
moment it is produced. A task on the other end appends each one to the session log:

```rust
let (recorder, receiver) = tokio::sync::mpsc::unbounded_channel();
let agent = Agent::new(provider, tools, model, api_key).with_recorder(recorder);
```

Writing after a run returns would mean a crash costs the entire conversation, and an agent
run is exactly the kind of long operation during which crashes happen — a tool panics, the
terminal goes away, the user gives up and closes the window. Streaming to disk as messages
are produced costs at most the line being written, and the log is append-only and never
rewritten, so a torn final line is recoverable by skipping it.

The channel also decouples the two rates. The agent never waits on a filesystem write, and
the writer never holds up a turn. Dropping the agent closes the sender, which ends the
writer, which is how the process knows every message reached the log before it exits.

Compaction is written down rather than only applied. When a conversation approaches the
context window, `micro-context` replaces its older half with a summary, and the summary is
recorded beside the log along with where the conversation now starts reading from. Nothing
is deleted — every message stays on disk, and the tree still shows the stretch the summary
stands for — but a resumed session opens on the summary instead of paying to write the
same one again.

## Stream events carry deltas

`StreamEvent` reports what changed rather than what the message now contains:

```rust
StreamEvent::TextDelta { index: 0, delta: "llo".into() }
```

The alternative — sending a rebuilt partial message with each token — would have every
provider clone the entire response once per token, which is quadratic in the length of the
answer and worst exactly where it hurts, on the long responses that take the longest to
stream. Deltas are constant per token.

The cost is that a consumer wanting the text so far has to accumulate it, so `micro-agent`
provides `PartialResponse` for the purpose: feed it events, ask it for text or blocks.
Consumers that do not need the running text — a logger, a headless run that only wants the
final message — pay nothing to skip it.

Terminal events are the exception and carry whole values. A stream always ends in exactly
one `StreamEvent::Done` with the assembled message, or one `StreamEvent::Error`, so a
consumer can drain until one arrives and never has to decide when a response is finished.
`AgentEvent::AgentEnd` does the same at the level above.

## The loop

`Agent::run` sends the prompt, then repeats: build a `Context` from the conversation, stream
one response, run the tools it asked for, append each result. It stops when a response asks
for no tools, or fails.

A request that fails before showing any content is retried with exponential backoff, but
only for transient HTTP statuses, and never once the user has seen output — text already on
screen cannot be unshown, so re-issuing would duplicate it. A tool that fails returns its
error as a tool result flagged as an error, rather than aborting the turn, because the model
can usually recover from a failed call if it is told what happened. A tool the model names
but that does not exist is the same case.

Every step is an `AgentEvent` on a channel, which is what lets a headless CLI, an interface,
and a test consume the identical loop. `micro-testkit` takes advantage of that: a scriptable
provider, a fake tool with a call counter, and a fake summarizer are enough to exercise the
whole loop with no network.
