# The ledger

A session log holds the conversation. The ledger is everything else that happened: the
exact request each turn issued, what the provider said it cost, where every stretch of the
prompt came from, and what something watching the run would not allow. It is written to the
same file as the conversation, one JSON object per line, in the order things happened.

The format is public and versioned. `micro sessions export <id>` prints it verbatim, and
what this document describes is what you get.

## Where it lives

```
$MICRO_DIR/sessions/1786754321.jsonl        the log: conversation and ledger together
$MICRO_DIR/sessions/1786754321.meta.json    the sidecar a listing reads
$MICRO_DIR/sessions/1786754321.blobs/       content the log names by hash
```

## The envelope

Every ledger line is an envelope with four fields, and it is the only kind of line that
says what it is:

```json
{"v":1,"seq":7,"ts":1786754321987,"event":{"type":"turn_usage","turn":2,"...":"..."}}
```

`v` is the schema version, `seq` orders this fact against every other one in the session,
and `ts` is when it was written in milliseconds since the epoch. The other line kinds — a
conversation entry, a label, a compaction marker — are unchanged and carry none of these,
so nothing written by the ledger can be mistaken for one of them.

A reader that meets an event type it has never heard of keeps the line and gives up only on
its contents. A later version of micro can add a type without making today's sessions
unreadable in either direction.

## What a turn records

Two events describe a turn. `turn_request` is written after everything watching the run has
had its say about the context and immediately before the provider is called; `turn_usage`
is written when the answer arrives.

```json
{"v":1,"seq":3,"ts":1786754321900,"event":{
  "type":"turn_request","turn":2,"provider":"anthropic","model":"claude-opus-5",
  "prefix_hash":"9f2a…","request_hash":"3c81…",
  "system_prompt_blob":"4c1f…","tools_blob":"1b7c…","model_blob":"aa03…",
  "prefix_spans":[
    {"source":"system_prompt","bytes":812,"hash":"5d2e…"},
    {"source":"project_instructions","bytes":392,"hash":"7ab1…"}],
  "message_entry_ids":["1","2","3"],"attempt":1}}
```

`request_hash` is the sha-256 of the serialized request body. The body itself is not
stored: what makes it rebuildable is the three blobs — the system prompt, the tool
definitions, and the model as it was configured — plus the entries the conversation stood
at. `prefix_hash` covers the system prompt and the tool definitions together, which is the
part of a request a provider can cache; two turns sharing it asked for the same cacheable
head.

`prefix_spans` say who supplied each stretch of the prompt. The spans tile the prompt
exactly, separators included, so what they add up to is the prompt itself. A source is
written as a kind, optionally with a name: `system_prompt`, `project_instructions`,
`skill:review`, `extension:deploy`, `tool:bash`, `user`, `model`, `compaction`, `sandbox`,
`subagent:scout`. A kind with no name stands for the whole of that kind rather than one
member of it — `skill` is the section describing every skill that loaded.

A turn re-issued after a transient failure is recorded once per attempt, with `attempt`
counting up and `turn` staying the same. The last attempt is the one that produced the
answer.

`turn_usage` carries what the provider reported, kept as its own fact so a session that is
later summarized still knows what it was billed for:

```json
{"type":"turn_usage","turn":2,"usage":{"input":812,"output":41,"cache_read":0,
 "cache_write":0},"stop_reason":"tool_use","provider":"anthropic","model":"claude-opus-5"}
```

## The other events

`compaction` says a stretch of the conversation was replaced by a summary, naming the
summary by hash and how many recent messages were kept. `head_moved` says the conversation
now continues from a different entry, which is what makes a branch survive being reopened.
`tool_denied` says something watching the run refused a call; the model was told in the
shape a failed call takes, and this is the record that it was a refusal rather than a
failure. `sandbox_decision` says what the sandbox allowed or refused and under which policy.
`extension_crossing` says an extension asked the host for something and what it was told.
`prefix_changed` says the cacheable head of the request changed, and why.
`budget_stop` says a run stopped because it had spent what it was allowed to. `marker` is
for anything that has not earned a kind of its own.

## Blobs

Content a fact refers to rather than contains is filed under the hex sha-256 of its bytes,
in a directory beside the log. The name is the content, so a write is never repeated: a
system prompt that stands unchanged for a hundred turns is on disk once and named by all
hundred of them. Blobs are written through a temporary file and renamed, so a crash
mid-write cannot leave one whose name lies about what is in it, and they are deleted with
the session that named them.

## What is guaranteed

The log is append-only. Nothing already written is ever rewritten, so the worst a crash
costs is the line being written, and a log that ends mid-line is sealed on the next open
and counted as one unreadable line rather than swallowing whatever is appended next.

Sequence numbers are assigned by the session and increase without gaps within a run.
Reopening a session carries on from the highest number the log holds, so a fact recorded
after a resume sorts after everything recorded before it.

A session written before the ledger existed holds messages and nothing else. It opens
exactly as it always did, and `micro sessions show` says it has no turns rather than
inventing any.

Forking copies a conversation, not a ledger. The session that comes out of `/fork` starts
its own numbering and names the session it came from in its sidecar, which is what keeps
each ledger an account of one run of one agent.

## Reading it

```
micro sessions show <id>                 the turns the session recorded
micro sessions show <id> --turn 2        what the model was shown at turn 2
micro sessions show <id> --turn 2 --raw  that turn's request, as it went out
micro sessions export <id>               the whole ledger as JSONL
```

`--raw` rebuilds the body from what was recorded and hashes it against `request_hash`
before printing. A mismatch is reported rather than passed off as the request, which is
what makes the record checkable rather than merely stored. One case rebuilds differently by
design: an Anthropic subscription credential is issued to a named client and spells the
tool names that client's way, and a reading of a request has no credential in hand, so it
reads the request as an API key sends it.

## org_id and agent_id

The sidecar carries an optional `org_id` and `agent_id` alongside the session's own
metadata. Nothing in micro fills them in and nothing is sent anywhere — there is no
telemetry in micro and no account to attach a session to. They are in the schema from its
first version because an exported ledger is a record somebody may need to file against an
organization or against the agent that produced it, and a field added later cannot be
backfilled onto sessions already written.
