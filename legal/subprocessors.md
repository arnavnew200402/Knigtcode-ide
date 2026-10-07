---
title: Subprocessor List
slug: subprocessors
---

KnightCode holds no account for you and stores no personal data, so there is
little here to list. What follows is every third party that sees anything at
all as a result of running KnightCode.

| Party | What reaches them | When |
| --- | --- | --- |
| Vercel | The HTTP request carrying the anonymous install ping and the four launch signals, and so the connecting IP address, which is not stored. | On install or update, and on the first-run and first-use events, unless you turned the ping off. |
| PostHog | The events themselves, already stripped of the address: version, OS, architecture, approximate location, and one fixed word per launch signal. | As above. |
| Cloudflare | DNS for `knightcode.dev`. | As above. |
| GitHub | The update check and the download of a new version. | Hourly while KnightCode is running, if automatic updates are on. |
| Zed Industries, Inc. | The extension search or install request. | Only after you open the Extensions page. |

Your AI provider is not a subprocessor of ours. You sign in to them directly,
the credential never leaves your machine, and your prompts go from your machine
to theirs without passing through anything we run.

The current version of this document lives at
<https://knightcode.dev/legal/subprocessors>.
