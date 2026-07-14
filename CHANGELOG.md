# Changelog

## 0.1.0

First release.

A coding agent for the terminal, in one binary with no runtime to install.

Talks to OpenRouter, GitHub Copilot, Google Gemini and Anthropic, with API keys or a
device sign-in, and switches between them mid-conversation without losing what was said.

Every conversation is recorded as it happens and reopens exactly as it was left, branches
included: going back to an earlier point keeps the answer that was there before.

Tool calls pass through a policy that asks before anything is changed or run, and a
project can be vouched for once with `/trust` instead of every time.

Skills are read from the workspace and from `~/.micro/skills`, announced to the model by
name so it reaches for one only when it applies.
