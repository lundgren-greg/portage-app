# portage-app — project tracker

> **Resume here** when starting a new session. Keep this file current when you stop work.
> Brainstorm / convo context: [`docs/brainstorm.md`](docs/brainstorm.md)

| Field | Value |
|-------|--------|
| **Local path** | `C:\Repos\portage-app` |
| **GitHub** | `lundgren-greg/portage-app` |
| **Branch** | `feat/pr3-catalog-sqlite` (tracks `origin`) |
| **Last commit** | `a6937fb` feat(catalog): SQLite WAL schema, migrations, file/blob/replica queries |
| **Remote** | `origin` → https://github.com/lundgren-greg/portage-app.git |
| **Status** | **Paused 2026-08-17.** PRs 1, 1.5, 2 and Dependabot #10/#11 are on `main`. PR 3 is implemented and open as [#12](https://github.com/lundgren-greg/portage-app/pull/12). Do not start PR 4 until #12 merges. |
| **Updated** | 2026-08-17 |

---

## Goal

Ship a Windows-first tool (`portage` CLI, then `portage-tui`, plus `portage ask`) that inventories local disks, Google Drive, and personal OneDrive, then produces a user-confirmed, space-safe shuttle plan so gaming clips and other libraries can live where the user asked — locally and/or on the cloud with more free space — **without losing files**, filling a 4 GiB-free disk, deleting the last copy, corrupting a multi-GB file, or making anything public. NL (Grok first) proposes; it never applies.

---

## Stopped at

1. Created `lundgren-greg/file-portage` from `repo-template` (Rust CI, MIT).
2. Wrote and reviewed the design (`docs/design.md`). Reviewer approved after 3 rounds.
3. Checked in feature set (`docs/FEATURES.md`) and `configs/examples/gaming-clips.yaml`.
4. Wiki draft lives in `docs/wiki/`. GitHub Wiki tab needs one “Create the first page” click, then `.\scripts\Publish-Wiki.ps1`.
5. Ported the brainstorm decisions (`docs/brainstorm.md`): clarify-then-plan agent, **local or online** LLM (Grok default; Ollama/LM Studio), desire + priority `Intent`, redacted catalog digest for online providers.
6. **PR 1 merged:** Cargo workspace, eight crate stubs, `portage --help`, `portage init`.
7. **PR 1.5 merged:** `portage-core::obs` — JSONL logging with enforced redaction, metrics registry, Prometheus snapshot, `portage status`, CI coverage gate (≥80% on core/catalog/engine).
8. **PR 2 merged:** `portage-core` ids (BLAKE3 `ContentId`, typed ids), streaming `MultiHasher` (BLAKE3+MD5/SHA1/SHA256, 1 MiB buffer), `QuickHash` 64 KiB prefilter, `paths::ensure_inside` (traversal/ADS/symlink containment). QuickXor deferred to PR 8.
9. Dependabot CI bumps merged: `actions/cache` v6 (#10), `actions/checkout` v7 (#11).
10. **PR 3 implemented and opened** as [#12](https://github.com/lundgren-greg/portage-app/pull/12) on `feat/pr3-catalog-sqlite` (`a6937fb`). SQLite WAL catalog (`migrations/0001` + `0002`), exclusive/shared `portage.lock`, proto-blob on file insert, lookup by path and `ContentId`, capacity snapshots, `portage doctor` + `--backup`. Local `cargo test --workspace`, clippy, and llvm-cov (~85% lines) were green; CI on #12 was still running when we paused.
11. **Paused** (user request). Leftover *uncommitted* local files are not part of #12: `.agents/skills/next-pr/`, `.cursor/`, `.grok/`, `CLAUDE.md`, plus dirty `AGENTS.md` / copilot-instructions / skills README / `How-we-work.md` (next-pr skill wiring). Leave them unless a later session wants that as its own PR.

---

## Next steps (ordered)

1. Resume: check CI on [#12](https://github.com/lundgren-greg/portage-app/pull/12), merge if green. Then **PR 4** — local provider: volumes (including USB), overlay roots, placeholders, walk. Do not start PR 4 on this pause.
2. Follow PRs 5–13 exactly as written. **No-data-loss P0** (apply + undo refuse) before TUI/NL. Merge gates for planner PRs: P-space and P-last-copy tests. Every PR: unit tests + ≥1 integration test per touched boundary.
4. PR 14 polish → PR 15 `portage-tui` → PR 16 `portage ask` (clarify-then-plan agent, local or online).
5. Keep this file's **Stopped at** current.

---

## Blockers

| Blocker | Detail | Unblock |
|---------|--------|---------|
| OAuth client ids | v1 is bring-your-own (`PORTAGE_GOOGLE_CLIENT_ID`, `PORTAGE_MS_CLIENT_ID`) | Needed only at PR 6+ live tests. Mocks first. |

---

## Resolved questions (user, 2026-08-14 — final)

| # | Question | Answer |
|---|----------|--------|
| 1 | OAuth client ids | **BYO in R1:** `PORTAGE_GOOGLE_CLIENT_ID`, `PORTAGE_MS_CLIENT_ID` |
| 2 | OneDrive scope | **Personal `/me/drive` in Release 1.** M365/SharePoint = **Release 2** |
| 3 | Interaction / keep_local | **Clarify-then-plan agent (local or online).** Desire + priority → questions → Intent → dry-run. **Never applies.** Default `prefer` + warning; “must stay on C:” → `required`. |
| 4 | Undo / data loss | **No data loss = R1 P0.** Reverse-plan + second typed id. Refuse last-copy or reserve breach. Never auto-redownload. |
| 5 | Catalog location | `init` measures C:. If < 8 GiB, recommend largest non-overlay volume. NL may confirm `data_dir`. Engine rejects unsafe dirs. No silent move. |
| 6 | Google scope | **Full `drive`.** Consent copy explains why `drive.file` cannot inventory existing clips. |
| 7 | TUI | **`portage-tui` in R1 after safety MVP (PR 15).** Color/hotkeys. Does not block PRs 1–13. |
| 8 | First user of apply | **One personal machine.** No published OAuth client in R1. |
| 9 | Machines in R1 | **One PC.** Catalog on that host. Multi-machine is later. |
| 10 | What “organize” means | **LLM conversation clarifies with the user** each time (`portage ask`). Engine default remains placement (where copies live). No autonomous rename/rebuild. |
| 11 | External drive job | **Both hop and home.** Default `roles: [shuttle, final]`. Each plan chooses how that run uses the disk. |
| 12 | Cloud accounts in R1 | **One Google, one Microsoft** (personal). A second account is another provider later. |
| 13 | Evict aggressiveness | **Only cloud-only or unpinned collections.** Never evict `required` / pinned. `prefer` may drop local with a warning. |
| 14 | Name | **Product Portage. Repo `portage-app`.** Binary `portage`. |

---

## What's implemented

### Layout

```
portage-app/
  PROJECT.md
  README.md, LICENSE, SECURITY.md, CODEOWNERS, AGENTS.md
  docs/design.md
  docs/FEATURES.md
  configs/examples/gaming-clips.yaml
  .github/workflows/ci.yml   # build/test/clippy/fmt matrix + coverage gate
  crates/portage-core/       # error, units, obs, ids, hash, paths, DataPaths
  crates/portage-catalog/    # SQLite WAL, lock, files/blobs/replicas, doctor
  crates/portage-{auth,providers,media,engine,sim}/  # stubs
  migrations/0001_init.sql + 0002_plans_journal.sql
```

### Commands

```powershell
cd C:\Repos\portage-app
cargo run -p portage-cli -- --help
cargo run -p portage-cli -- init
cargo run -p portage-cli -- status --format=prom
cargo run -p portage-cli -- doctor
cargo test --workspace
```

---

## Roadmap (not done)

| Item | Notes |
|------|--------|
| ~~PR 1 workspace + CLI stub~~ | Merged (#5) |
| ~~PR 1.5 observability~~ | Merged (#7) |
| ~~PR 2 core ids/hash/paths~~ | Merged (#8) |
| PR 3 catalog | Open as [#12](https://github.com/lundgren-greg/portage-app/pull/12) — paused before merge |
| PR 4–5 local inventory | Walk + incremental hash; useful on D: of clips before any cloud |
| PR 6–8 Drive + OneDrive read | Unified inventory, still no mutations |
| PR 9–10 planner dry-run | 4 GiB fixture, no writes |
| PR 11–13 confirmed apply + undo | First real transfers; P0 no-data-loss gate |
| PR 14 polish | Safety MVP tag |
| PR 15 `portage-tui` | Color plan review; typed-id apply |
| PR 16 `portage ask` / Grok | NL → policy + plan; never applies |
| Release 2+ | M365/SharePoint, Dropbox/S3/SMB, extra LLMs — see design Future releases |

---

## Decisions log

| Date | Decision |
|------|----------|
| 2026-08-14 | Standard repo kit from `lundgren-greg/repo-template`. License MIT. Default branch `main`. |
| 2026-08-14 | Product name **Portage**; repo **portage-app** (renamed from file-portage); binary **portage**. |
| 2026-08-14 | Stack: Rust + SQLite WAL + BLAKE3. Not Python, not C#, not rclone. |
| 2026-08-14 | Cloud-to-cloud is always a local shuttle. Placeholders are not replicas. |
| 2026-08-14 | Apply requires typing the plan id. Last-copy + private ACL + staging reserve are non-negotiable. |
| 2026-08-14 | Design approved after 3 review rounds. Source of truth: `docs/design.md`. |
| 2026-08-14 | Repo is **public**. |
| 2026-08-15 | Maintainer can push `main`. Contributors use PRs + CI. |
| 2026-08-15 | External / USB volumes are first-class (`shuttle` hop and/or `final` dest; identity = volume serial). README leads with capabilities and names concrete situations (gaming clips **and** docs, archives, full disk, dups) so the audience is not one niche. |
| 2026-08-14 | User resolved open questions: BYO OAuth; personal OneDrive in R1 / M365 in R2; Grok-first NL never applies; no-data-loss P0; undo = reverse-plan + second id; init+NL catalog recommendation with engine reject; full Google `drive`; TUI PR 15 after safety MVP. |
| 2026-08-15 | NL is a **clarify-then-plan agent** (desire + priority, ≤3 clarify rounds), **local or online**: Grok default, any OpenAI-compatible localhost (Ollama/LM Studio). Online sees a redacted catalog digest unless `nl.send_paths`. Source: `docs/brainstorm.md`. |
| 2026-08-15 | Observability: structured **JSON-lines logs** + **Prometheus-text metrics snapshot**, local files only (Grafana Alloy/Loki/textfile-collector friendly). No listener, no push, no telemetry. New **PR 1.5**. |
| 2026-08-15 | Testing: CI `cargo llvm-cov` gate ≥80% on core/catalog/engine; every PR ships unit tests + ≥1 integration test per touched boundary; redaction tests merge-blocking. |
| 2026-08-15 | **Dedupe semantics (user):** consolidating duplicates across providers = one verified copy at the desired destination, evict redundant **local/USB** replicas in R1. Deleting the redundant **cloud** copy stays **R2** (`--allow-cloud-delete`, last-copy gated). |

---

## Session resume checklist

When starting a new agent/chat session:

1. Read **this file** (`PROJECT.md`).
2. Read `docs/design.md` **PR Plan** and implement the next unchecked PR only.
3. `git -C C:\Repos\portage-app status` and `git log -1 --oneline`.
4. `gh auth status`.
5. Update **Stopped at** / **Next steps** before ending the session.

---

## Do not

- Commit secrets, tokens, or real file inventories (use `samples/private/` locally; gitignored).
- Add share-link / `anyoneWithLink` / public ACL helpers. Ever.
- Hydrate OneDrive or DriveFS placeholders.
- Auto-apply plans. No daemon in R1. **LLM proposes, never applies.**
- Start TUI (PR 15) or NL compile-to-plan (PR 16) before PR 13 undo is merged.
- Skip the planner property tests (P-space, P-last-copy).
- Force-push or rewrite history on `main` without a reason.
- Implement PRs out of order unless the design marks them independent (PR 6 can proceed after PR 4).
