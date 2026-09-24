# AGENTS.md

This file defines the commit and branch rules for this repository.

## Commit Messages

The format is `<type>: <description>`: one line, in English, imperative mood, no more than 50 characters, no trailing punctuation.

Common `<type>` values: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, `revert`.

Example: `fix: keep wrapped linear rows one fixed size`

## Branches

| Branch | Purpose |
| --- | --- |
| `dev` | Daily development branch, committed to frequently. |
| `main` | Stable branch, kept generally usable. |

- All development commits go to `dev` by default.
- Merging into `main` and pushing must be explicitly commanded by the user.
