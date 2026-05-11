# Ledger format

Each session is an append-only JSONL file. Conversation entries and runtime events are written to the same ordered log.

Use:

```bash
micro sessions export <SESSION_ID>
```

to print the file without transforming it.

## Files

```text
sessions/<id>.jsonl       conversation and ledger events
sessions/<id>.meta.json   metadata used by session listings
sessions/<id>.blobs/      content addressed by hash
```

See [Configuration](configuration.md) for the base data directory.

## Event envelope

Ledger events use this envelope:

```json
{
  "v": 1,
  "seq": 7,
  "ts": 1786754321987,
  "event": {
    "type": "turn_usage",
    "turn": 2
  }
}
```

`v` is the event schema version. `seq` orders events within the session. `ts` is milliseconds since the Unix epoch.

Conversation entries, labels, and compaction markers retain their existing line formats. Readers should preserve event types they do not understand and skip interpretation of their contents.

## Turn requests

`turn_request` is written after context hooks run and immediately before the provider call.

```json
{
  "type": "turn_request",
  "turn": 2,
  "provider": "anthropic",
  "model": "claude-opus-5",
  "prefix_hash": "9f2a...",
  "request_hash": "3c81...",
  "system_prompt_blob": "4c1f...",
  "tools_blob": "1b7c...",
  "model_blob": "aa03...",
  "prefix_spans": [
    { "source": "system_prompt", "bytes": 812, "hash": "5d2e..." },
    { "source": "project_instructions", "bytes": 392, "hash": "7ab1..." }
  ],
  "message_entry_ids": ["1", "2", "3"],
  "attempt": 1
}
```

`request_hash` is the SHA-256 hash of the serialized provider body. The body is reconstructed from the referenced system prompt, tool definitions, model configuration, and conversation entries.

`prefix_hash` covers the cacheable prompt head. `prefix_spans` attribute byte ranges to sources such as the system prompt, project instructions, skills, extensions, tools, users, model messages, compaction, sandbox output, and subagents.

Retries keep the same turn number and increment `attempt`.

## Usage

`turn_usage` records provider-reported token counts and the stop reason:

```json
{
  "type": "turn_usage",
  "turn": 2,
  "usage": {
    "input": 812,
    "output": 41,
    "cache_read": 0,
    "cache_write": 0
  },
  "stop_reason": "tool_use",
  "provider": "anthropic",
  "model": "claude-opus-5"
}
```

`micro bill` combines these counts with catalog prices. Prompt-source attribution uses the spans from the matching request and is an estimate. See [Sessions](sessions.md).

## Other event types

| Type | Meaning |
| --- | --- |
| `compaction` | Older context was summarized. Includes summary content and usage. |
| `head_moved` | The active conversation branch changed. |
| `tool_denied` | A hook or policy refused a tool call. |
| `sandbox_decision` | The command or file sandbox allowed or refused an operation. |
| `extension_crossing` | An extension requested a host operation and received a result. |
| `prefix_changed` | The cacheable prompt prefix changed. |
| `budget_stop` | The session reached its configured cost limit. |
| `marker` | A named runtime marker without a dedicated event type. |

New event types may be added without changing the outer envelope version.

## Prefix changes

Changes requested by reloads, tool selection, or extensions are applied at turn boundaries and recorded:

```json
{
  "type": "prefix_changed",
  "reason": "reload",
  "from_hash": "9f2a...",
  "to_hash": "41bd..."
}
```

`micro why-miss` compares adjacent requests, resolves changed spans from blobs, and reports the event between them. Compaction changes conversation context rather than the prefix and has its own event.

## Blobs

Large or repeated content is stored by SHA-256 below the session's blob directory. A stable system prompt referenced by many turns is written once.

Blob writes use a temporary file followed by a rename. Deleting the session also deletes its blob directory.

## Append and recovery guarantees

- Existing log lines are never rewritten.
- Events are appended while the run is active.
- Sequence numbers increase within a run and continue from the highest value after resume.
- A partial final line is isolated when the session is opened again.
- Unreadable lines are skipped rather than preventing the rest of the session from loading.
- Forking copies conversation state into a new session; it does not continue the original ledger sequence.

Sessions created before ledger events existed still open. Commands that require turn records report that those records are unavailable.

## Request reconstruction

```bash
micro sessions show <SESSION_ID> --turn 2 --raw
```

The command rebuilds the request and compares its hash with `request_hash`. It does not print a mismatched reconstruction as verified.

Anthropic subscription credentials are a special case: the request format may use client-specific tool names. Reconstruction without that credential uses the API-key spelling and reports the limitation.

## Optional metadata

The session sidecar supports optional `org_id` and `agent_id` fields. micro does not populate or transmit them. They are available for systems that file exported sessions against an organization or agent identity.
