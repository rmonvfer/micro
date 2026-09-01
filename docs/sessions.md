# Sessions

micro saves each conversation as it runs. A session can be resumed, branched, exported, inspected, or deleted without a server-side account.

## List and resume sessions

List sessions for the current workspace:

```bash
micro sessions list
```

Include sessions from other workspaces:

```bash
micro sessions list --all
```

Resume a session:

```bash
micro --resume <SESSION_ID>
micro --continue
```

`--continue` selects the latest session for the current workspace.

Inside the interface, use `/sessions`, `/resume`, `/tree`, `/fork`, and `/clone` to navigate or branch a conversation.

## Inspect a session

```bash
micro sessions show <SESSION_ID>
micro sessions show <SESSION_ID> --turn 4
```

To see the provider request for one turn:

```bash
micro sessions show <SESSION_ID> --turn 4 --raw
```

Sessions enqueue the exact serialized provider body for storage in a content-addressed blob. Before `--raw` prints a retained body, micro checks it against the request hash recorded when the request was sent. A process crash can lose records that were still queued for disk.

For a session without a retained body, micro rebuilds the request from the recorded context. It prints that reconstruction only when its hash matches. A mismatch is an error.

Export the underlying JSONL file with:

```bash
micro sessions export <SESSION_ID>
```

See [Ledger format](ledger.md) for the event schema.

## Export, import, and share

Inside an interactive session, `/export [path]` writes a readable Markdown transcript. `/import <path>` imports a session JSONL file and switches to it.

`/share` uploads a transcript as a secret GitHub gist. It reads `GITHUB_TOKEN`, then `GH_TOKEN`; the token needs the `gist` scope. A secret gist is unlisted, not access-controlled, so anyone with the URL can read it.

These commands are explicit. micro does not upload sessions automatically.

## Billing

Select a session or one turn explicitly:

```bash
micro bill <SESSION_ID>
micro bill <SESSION_ID> --diff 4
```

The turn total is calculated from provider-reported token usage and the pricing snapshot recorded with the request. Older sessions without a snapshot use the current model catalog. The split between prompt sources is an estimate based on the number of bytes each source contributed. The estimated lines are adjusted to add up to the turn total.

Billing counts provider turns and compaction usage recorded under the session ID. The session total includes all branches; the current-branch subtotal includes only requests and compactions whose recorded path is an ancestor of the current head.

Output cost is attributed to the model. Compaction requests appear separately.

If the catalog sets a model price to zero, the report identifies it as a zero-priced model. If the catalog has no price, the report says the cost is unknown rather than calling it free.

The interactive `/bill` command reads the same records for the active session. Select a turn and press Enter to open its prompt-source and provider-usage breakdown. The terminal footer shows the running total.

## Budgets

Set a session budget in US dollars:

```bash
micro --budget 5
```

The budget applies to the complete session, including earlier runs resumed under the same ID. The provider reports usage after a request finishes, so micro enforces the limit at turn boundaries. A turn may take the total past the limit; no further turn starts after that.

Set a default with the `budget` key in `config.json`. A value of `0` disables the limit.

## Prompt-cache misses

Inspect local evidence for why a turn did not reuse its parent turn's cached prompt:

```bash
micro why-miss <SESSION_ID> 4
```

If the recorded prefix hash changed, micro identifies the changed prompt span and shows a line diff when the source content is available. It also reports a nearby local event, such as a reload, tool-list update, or extension hook.

If the prefix did not change, the report checks conversation-side changes such as compaction or branching. It cannot observe provider eviction, cache lifetime, eligibility, or the provider's cache-read decision, so its output is diagnostic rather than proof of the provider-side cause.

`/why-miss 4` runs the same analysis inside an interactive session. Without a turn, `/why-miss` selects the latest completed turn on the current branch whose prefix differs from its parent.

## Delete a session

```bash
micro sessions delete <SESSION_ID>
```

This removes the content-addressed blob directory and metadata before removing the log. Cleanup errors are reported, and the log remains available so deletion can be retried.

## Storage

Sessions are stored below micro's data directory:

```text
sessions/<id>.jsonl
sessions/<id>.meta.json
sessions/<id>.blobs/
```

The exact base path depends on `MICRO_DIR`, an existing `~/.micro`, or the XDG data directory. See [Configuration](configuration.md).

Session logs, metadata, and retained provider bodies are plaintext. On Unix, session directories use mode `0700` and files use mode `0600`.
