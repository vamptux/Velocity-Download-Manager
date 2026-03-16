# Security Policy

## Supported versions

| Version | Supported |
| --- | --- |
| `0.1.x` | Yes |
| Older builds | No |

## Reporting a vulnerability

Do not open a public issue for security-sensitive bugs.

If the repository host offers private vulnerability reporting, use that path first. If it does not, contact the maintainer privately before disclosure and include:

- the affected build or commit
- exact reproduction steps
- whether the issue involves the capture bridge, request replay, file writes, resume validation, or extension interception
- logs or screenshots with secrets removed
- any proof-of-concept material needed to reproduce the issue safely

## Areas of interest

- capture-bridge authentication and loopback request validation
- extension request-context handoff
- resume validator handling and guarded fallbacks
- filesystem finalization and overwrite behavior
- scheduler or host-planner behaviors that could corrupt or cross wires between downloads