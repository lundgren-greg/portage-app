# For agents

You are continuing Portage. Do not invent a new architecture.

## Start

1. Read [PROJECT.md](https://github.com/lundgren-greg/portage-app/blob/main/PROJECT.md).
2. Read the **PR Plan** at the bottom of [docs/design.md](https://github.com/lundgren-greg/portage-app/blob/main/docs/design.md).
3. `git -C C:\Repos\portage-app status` and `git log -1 --oneline`.
4. `gh auth status`. Remote is `https://github.com/lundgren-greg/portage-app.git`.

## Do now

Read `PROJECT.md` for the next numbered PR. Implement **that PR only** (`/next-pr`).

**Paused 2026-08-17.** PRs 1, 1.5, and 2 are on `main`. **PR 3** is implemented and waiting as [pull/12](https://github.com/lundgren-greg/portage-app/pull/12). After it merges, implement **PR 4** unless `PROJECT.md` says otherwise. Do not start PR 4 while #12 is open.

Branch + pull request. Prefer not to commit on `main`.

## Do not

- Skip ahead to providers, planner, or apply.
- Add share-link / `anyoneWithLink` helpers.
- Open OneDrive or DriveFS placeholders.
- Auto-apply plans. The LLM never applies.
- Commit secrets, tokens, or a real file inventory.
- Force-push `main`.

## Hard rules (even in PR 1, keep the door open)

Last-copy, typed plan id, staging reserve, private-only uploads, volume serial for USB, fail closed if a needed disk is unplugged. Details: [Safety](Safety) and [External drives](External-drives).
