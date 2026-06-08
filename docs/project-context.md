# Project context

micro can add instructions, skills, prompt templates, and system-prompt changes to a session. Project-provided resources load only after the project is trusted.

## Instruction files

micro reads `AGENTS.md` and `CLAUDE.md` from:

1. the user home directory;
2. each parent directory from the filesystem root to the workspace;
3. the workspace itself.

Files closer to the workspace are appended later. Use parent-directory files for rules shared by several repositories and a workspace file for project-specific instructions.

An instruction file may include another file:

```text
@import ./docs/conventions.md
```

Imports are resolved relative to the importing file and followed up to five levels deep.

Disable instruction discovery for one run with:

```bash
micro --no-context-files
```

`/reload` reads instruction files and skills again. The resulting prompt-prefix change is recorded in the session ledger.

## System prompts

`SYSTEM.md` replaces micro's built-in system prompt. `APPEND_SYSTEM.md` adds text after it.

User-level files live in micro's configuration directory. A trusted project may provide its own files under `.micro/`; project files take precedence.

Project instructions and the skill list are added separately after the system prompt.

## Skills

A skill is a Markdown file with frontmatter that tells the model when to load it:

```markdown
---
name: release-check
description: Check a release candidate before publishing it.
---

Read the changelog, run the release tests, and verify package metadata.
```

micro advertises each skill by name and description. The body remains on disk until the model chooses to read it.

Skills are discovered in:

```text
.micro/skills/            project-specific, requires trust
.agents/skills/           project-specific, requires trust
<micro config>/skills/    user skills
~/.agents/skills/         shared user skills
```

A directory containing `SKILL.md` defines one skill and may keep supporting files beside it. A directory without `SKILL.md` is searched recursively for Markdown skill files.

Project skills win name conflicts over user skills. Skill names use lowercase letters, digits, and single hyphens, with a maximum of 64 characters. Descriptions may be up to 1,024 characters.

Set `disable-model-invocation: true` in frontmatter to keep a skill available for explicit use without advertising it for automatic selection.

Load another file or directory for one run:

```bash
micro --skill ./skills/release-check
```

Use `/skills` to list what loaded. Startup diagnostics name invalid or unreadable skill files.

## Prompt templates

A Markdown file under `prompts/` becomes a slash command named after the file:

```markdown
---
description: Review a pull request
argument-hint: <number> [branch]
---

Review PR $1 against ${2:-main}. List merge blockers first.
```

Running `/review 42 dev` expands the template and submits the rendered text as the user prompt.

Supported placeholders include:

```text
$1, $2                 positional arguments
$@, $ARGUMENTS         all arguments
${1:-main}             default value
${@:2}, ${@:2:3}       argument slices
```

Templates are loaded from the user prompt directory and from `.micro/prompts/` in a trusted project. A project template wins a same-name conflict. Built-in slash commands always take precedence.

Load another template or directory for one run with `--prompt-template <PATH>`. Use `--no-prompt-templates` to disable discovery.

## Project trust

A project containing `.micro/` resources requires a trust decision before they load. User-level resources do not require project trust.

Use `/trust on` or `/trust off` to save a decision. `--approve` and `--no-approve` apply only to the current run.

See [Security model](security.md) for the complete decision order and noninteractive behavior.
