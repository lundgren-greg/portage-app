# How we work

The repo is public. Pull requests are the usual path onto `main`. CI should stay green (`Build & Test` on Windows and Ubuntu).

## What lives where

| Place | Job |
| --- | --- |
| This wiki | Orientation. Decisions. How we work. |
| `README.md` | Public pitch and use cases |
| `docs/design.md` | Spec an engineer can implement |
| `docs/FEATURES.md` | Checkbox feature set |
| `PROJECT.md` | Session resume, blockers, next PR |
| `configs/examples/` | Policy fixtures (not product verticals) |

Do not treat the wiki as a second design doc. If the spec changes, change `docs/design.md`, then update the wiki to match.

## Agents and contributors

1. Read `PROJECT.md`, then the design **PR Plan**.
2. Implement the **next numbered PR only** (`/next-pr`), unless the design marks it independent.
3. Branch from `main` and open a pull request.
4. Planner PRs are incomplete without P-space and P-last-copy tests.
5. No share-link APIs. No placeholder hydration. No apply without a typed plan id.

Local clone is `C:\Repos\portage-app`.
