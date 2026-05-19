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

New sessions retain the exact serialized provider body in a content-addressed blob. Before `--raw` prints it, micro checks the blob against the request hash recorded when the request was sent.

For an older session without a retained body, micro rebuilds the request from the recorded context. It prints that reconstruction only when its hash matches. A mismatch is an error.

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

The turn total is calculated from provider-reported token usage and the prices in the model catalog. The split between prompt sources is an estimate based on the number of bytes each source contributed. The estimated lines are adjusted to add up to the turn total.

Billing counts every provider turn recorded under the session ID, including spend on branches that are no longer active. When that differs from the current branch, the report shows the current-branch and other-branch subtotals.

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

Explain why a turn did not reuse its parent turn's cached prompt:

```bash
micro why-miss <SESSION_ID> 4
```

If the recorded prefix hash changed, micro identifies the changed prompt span and shows a line diff when the source content is available. It also reports the event that caused the change, such as a reload, tool-list update, or extension hook.

If the prefix did not change, the report checks conversation-side causes such as compaction or a branch change.

`/why-miss 4` runs the same analysis inside an interactive session. Without a turn, `/why-miss` selects the latest completed turn on the current branch whose prefix differs from its parent.

## Delete a session

```bash
micro sessions delete <SESSION_ID>
```

This removes the session log, metadata sidecar, and content-addressed blobs associated with it.

## Storage

Sessions are stored below micro's data directory:

```text
sessions/<id>.jsonl
sessions/<id>.meta.json
sessions/<id>.blobs/
```

The exact base path depends on `MICRO_DIR`, an existing `~/.micro`, or the XDG data directory. See [Configuration](configuration.md).
