---
name: next-pr
description: >
  Implement the next Portage design PR only. Use when starting a portage-app
  coding session, executing the PR Plan, or the user runs /next-pr.
---

# Next Portage PR

Do not invent an architecture. The approved plan is `docs/design.md` **PR Plan**.

## Steps

1. Read `PROJECT.md` (`Stopped at`, `Next steps`).
2. Read that numbered PR block in `docs/design.md` and the design sections it names.
3. `git status -sb` and `git log -1 --oneline`. Branch from `main` as `feat/prN-<slug>` if you are not already on that PR's branch.
4. Implement **that PR only**. Follow `tdd` (failing test first) and `incremental-implementation` (one slice, then `cargo test`).
5. Every PR: unit tests + ≥1 integration test on each touched boundary. Planner PRs also need P-space and P-last-copy.
6. Run:

   ```powershell
   cargo test --workspace
   cargo clippy --workspace -- -D warnings
   cargo fmt --all -- --check
   ```

7. Update `PROJECT.md` (`Stopped at`, `Next steps`, `What's implemented`).
8. Open a PR against `main` (`create-pr`). Do not push `main`.

## Now

Next numbered PR after what `PROJECT.md` says is merged. Confirm with `gh pr list --state merged` if `PROJECT.md` looks stale.

## Do not

- Start a PR before every PR on its **Depends on** line is merged (providers need the catalog; apply needs the planner and executor; TUI (15) and NL (16) wait as the design says).
- Add share-link / `anyoneWithLink` APIs.
- Open OneDrive / DriveFS placeholders.
- Apply a plan without a typed plan id. The LLM never applies.
- Commit secrets, tokens, or a real file inventory.
