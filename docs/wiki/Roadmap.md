# Roadmap

PRs 1, 1.5, and 2 are on `main`. **PR 3** is implemented and open as [pull/12](https://github.com/lundgren-greg/portage-app/pull/12) (paused 2026-08-17 before merge). After that: **PR 4**.

Do the numbered PRs in [docs/design.md](https://github.com/lundgren-greg/portage-app/blob/main/docs/design.md). Short map:

| PR | What | Notes |
|---|---|---|
| ~~1~~ | Workspace + `portage` CLI stub + `init` | Merged |
| ~~1.5~~ | JSONL logs, metrics, coverage gate | Merged |
| ~~2~~ | BLAKE3 ids, paths, dual hasher | Merged |
| **3** | SQLite catalog + lock | Open [#12](https://github.com/lundgren-greg/portage-app/pull/12); paused |
| 4 | Local walk, placeholders, **removable volumes** | USB roles + volume serial |
| 5 | Incremental hash, `dups` / `search` | Useful on a full disk before any cloud |
| 6 | OAuth PKCE + DPAPI/keyring | Can overlap after PR 4 |
| 7–8 | Google Drive + OneDrive inventory | Read-only |
| 9 | YAML policy / collections | |
| 10 | Space-safe planner + 4 GiB fixture | Must have P-space / P-last-copy |
| 11 | Journal + serial apply (local/mock) | |
| 12 | Apply against Drive + Graph | First real move |
| 13 | Undo + doctor + ACL audit | **P0 complete** |
| 14 | README walkthrough, progress, release exe | |
| 15 | `portage-tui` | After 13 |
| 16 | `portage ask` / Grok | After planner; never applies |

## Later (not R1)

Microsoft 365 / SharePoint, Dropbox, S3, SMB/NAS, Android / other OS, a second PC, extra LLM vendors, published OAuth client, VM isolation (idea only).
