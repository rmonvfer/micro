# RPC mode

RPC mode runs micro without a terminal interface. It reads JSON objects from standard input and writes responses and agent events to standard output, one object per line.

```bash
micro --rpc -C /path/to/project -m opus
```

The process keeps one session open until standard input closes. It uses the same provider, tools, sandbox, session storage, and agent loop as interactive mode.

## Framing

Each input object must fit on one line and contain a `type` field:

```json
{ "type": "get_state", "id": "state-1" }
```

The optional `id` is copied to the command response:

```json
{
  "type": "response",
  "id": "state-1",
  "command": "get_state",
  "success": true,
  "data": { "model": "..." }
}
```

Failed commands use `success: false` and an `error` field. An unreadable input line produces a failed response and does not terminate the process.

JSON strings may contain escaped newlines. Records themselves are separated by the ASCII newline character.

## Send a prompt

```json
{ "type": "prompt", "id": "p1", "message": "explain the request path" }
```

micro first acknowledges the command, then streams serialized `AgentEvent` objects. Common event types include:

```text
agent_start        turn_start         message_start
message_delta      message_end        tool_start
tool_update        tool_end           turn_end
agent_end          agent_settled      retry
```

The `agent_end` event contains the messages produced by the complete run. `agent_settled` means no turn or queued follow-up remains.

## Control a running turn

Three commands are handled while a turn is active:

```json
{"type":"steer","id":"s1","message":"focus on the provider boundary"}
{"type":"follow_up","id":"f1","message":"then list the relevant tests"}
{"type":"abort","id":"a1"}
```

`steer` reaches the active run at its next steering boundary. `follow_up` waits behind the active turn. `abort` drops the active turn and anything queued behind it.

Other commands received during a turn are held until that turn finishes.

## Images

`prompt`, `steer`, and `follow_up` accept base64-encoded images:

```json
{
  "type": "prompt",
  "message": "describe this diagram",
  "images": [{ "data": "iVBORw0KGgo...", "mime_type": "image/png" }]
}
```

## Commands

| Type                      | Required fields        | Result                                                              |
| ------------------------- | ---------------------- | ------------------------------------------------------------------- |
| `prompt`                  | `message`              | Start a model run.                                                  |
| `steer`                   | `message`              | Add direction to the active run.                                    |
| `follow_up`               | `message`              | Queue another prompt in the same run.                               |
| `abort`                   | none                   | Stop the active run or clear queued prompts.                        |
| `new_session`             | none                   | Create and switch to a new session.                                 |
| `get_state`               | none                   | Return model, provider, thinking, session, and queue state.         |
| `set_model`               | `provider`, `model_id` | Select an exact catalog model.                                      |
| `cycle_model`             | none                   | Select the next catalog model.                                      |
| `get_available_models`    | none                   | Return catalog models and limits.                                   |
| `set_thinking_level`      | `level`                | Change reasoning effort.                                            |
| `cycle_thinking_level`    | none                   | Select the next reasoning level.                                    |
| `compact`                 | none                   | Compact the conversation immediately.                               |
| `set_auto_compaction`     | `enabled`              | Enable or disable automatic compaction.                             |
| `bash`                    | `command`              | Run a shell command under the session sandbox.                      |
| `abort_bash`              | none                   | Acknowledge the request; RPC bash commands are not background jobs. |
| `get_session_stats`       | none                   | Return session metadata and message count.                          |
| `switch_session`          | `session_path`         | Open another session file.                                          |
| `navigate_tree`           | `entry_id`             | Move the current session head to an earlier entry.                  |
| `fork`                    | `entry_id`             | Copy a branch into a new session.                                   |
| `clone`                   | none                   | Duplicate the current session at its current head.                  |
| `get_entries`             | none                   | Return conversation entries; optional `since` limits the result.    |
| `get_tree`                | none                   | Return the branch outline.                                          |
| `get_last_assistant_text` | none                   | Return the latest non-empty assistant text.                         |
| `set_session_name`        | `name`                 | Rename the session.                                                 |
| `get_messages`            | none                   | Return the current agent messages.                                  |
| `get_commands`            | none                   | Return available slash commands and their sources.                  |

The `bash` command accepts `exclude_from_context: true` when its output should not be added to the model conversation.

## Project trust

RPC mode cannot ask an interactive trust question. Use a saved decision, `--approve`, or `--no-approve` when the project contains `.micro/` resources. The command sandbox still applies.

## Minimal client

A shell pipeline is enough for simple requests:

```bash
printf '%s\n' \
  '{"type":"get_state","id":"1"}' \
  '{"type":"prompt","id":"2","message":"summarize src/"}' \
  | micro --rpc -q
```

A long-running client should parse every output line, route objects with `type: "response"` by `id`, and process agent events independently.
