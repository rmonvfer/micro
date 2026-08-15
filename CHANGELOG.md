# Changelog

## 0.1.0

First release.

A coding agent for the terminal, in one binary with no runtime to install.

Talks to OpenRouter, GitHub Copilot, Google Gemini and Anthropic, with API keys or a
device sign-in, and switches between them mid-conversation without losing what was said.

Every conversation is recorded as it happens and reopens exactly as it was left, branches
included: going back to an earlier point keeps the answer that was there before.

A project is vouched for once with `/trust` rather than every time, and that one decision
is what settles whether micro will run the code the project ships. Tool calls themselves
are not gated after that: once micro is running, it acts.

Skills are read from the workspace and from micro's own directory, announced to the model
by name so it reaches for one only when it applies.
