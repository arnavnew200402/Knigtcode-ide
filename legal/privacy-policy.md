---
title: Privacy Policy
slug: privacy-policy
---

## Summary

KnightCode sends one thing: an anonymous ping when it is installed or updated.
It carries the version, your operating system, your CPU architecture, and an
approximate location derived from the connecting address. It carries no
account, no file, no project name, no prompt, and no identifier that follows
you between versions. You are asked about it on first run and can turn it off
there or in Settings > AI at any time; turning it off stops it.

Beyond that, KnightCode plus a launch signal of four events — see
"What is sent" — is the whole of what leaves your machine, and none of it is
enabled until you answer the question on first run.

We hold no account for you, so there is nothing for us to hold about you.

## What is sent

**The install ping** (`ide_install`): version, OS, architecture, and the
country, region and city Vercel's edge derives from the connecting address. The
address itself is never stored: the identifier on the event is a truncated
SHA-256 of the address and the user agent, computed at request time and thrown
away, so repeat pings from one machine collapse into one person without a
machine identifier ever existing.

**Four launch signals**, each carrying nothing beyond a fixed word from a
closed list: whether first run finished and at which step it stopped; whether
the engine failed to start and which category of failure it was; the first
successful agent turn of a version and which provider served it; the first use
of each AI surface, once per version. No path, no username, no file name, no
error message, no model id, no project.

Nothing else. The upstream editor's own usage reporting is not connected to
anything: no KnightCode build compiles in an endpoint for it, so those events
are queued to a local log file and dropped.

## What is not sent

Your source code, your prompts, your model responses, your project names, your
file paths, your username, your credentials, and any identifier that persists
across versions or installs.

## Where your credentials and prompts go

KnightCode holds no API key of its own and runs no inference. When you sign in,
the credential is written to `auth.json` in your KnightCode configuration
directory on your machine, shared with the KnightCode CLI, and nowhere else.
Your prompts go from your machine to the provider you chose — Anthropic,
OpenAI, GitHub Copilot, or whatever your API key names — and their privacy
policy governs what happens next.

## Crash reports

None are uploaded. No KnightCode build compiles in a crash-report endpoint.

## Extensions

Opening the Extensions page, searching it, or installing an extension sends a
request to `api.zed.dev`, run by Zed Industries, Inc. That is the only
third-party service a KnightCode install contacts, it happens only after you
open that page, and their privacy policy applies to it.

## Contact

<https://github.com/KnightCodeAI/knightcode-ide/issues>

The current version of this document lives at
<https://knightcode.dev/legal/privacy-policy>.
