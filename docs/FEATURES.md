# Feature set

Implement from [design.md](design.md). This file is the product checklist, not a substitute for the design.

## Release 1 P0 — no data loss (PRs 1–13)

These are not optional polish. They ship before TUI and before NL compile-to-plan.

- [ ] Last verified copy is never deleted. Suspect ≠ replica.
- [ ] Verify (BLAKE3 local / native digest cloud) before a dest counts and before any evict.
- [ ] No apply without typing the exact plan id. LLM cannot apply.
- [ ] Dual SpaceDrift preflight; trough ≥ `staging_reserve`.
- [ ] `undo` = reverse plan + second typed id; refuse if reverse drops a blob to 0 verified replicas or breaches reserve. Never auto-redownload.
- [ ] Private-only uploads; inherited anyone fails; `we_created` compensation delete.
- [ ] Placeholders / overlay roots never opened or counted as replicas.

## Safety MVP (PRs 1–14) — usable on a 4 GiB-free Windows box

### Inventory

- [ ] `portage init` — measure `C:` free. If `< 8 GiB`, recommend the largest non-overlay volume (`D:\PortageData`). No silent move. Engine rejects `data_dir` on free < 8 GiB or overlay / Cloud Filter.
- [ ] Register local roots (`provider add local`), including **removable / external volumes**. Roles: `shuttle`, `final`, or both. Identity is volume serial, not drive letter. Refuse OneDrive / DriveFS overlay roots and Cloud Filter volumes. Unplugged disks fail closed and do not count as last-copy.
- [ ] Walk local NTFS without following junctions out of root. Never open Files On-Demand / DriveFS streamed files.
- [ ] Incremental BLAKE3 of `LocalFull` files only (`ntfs_file_id + size + mtime` skip).
- [ ] Google Drive list/delta + quota (`'me' in owners`, ignore Docs/Sheets/shortcuts-as-content).
- [ ] OneDrive Graph delta + quota (personal `/me/drive` only). M365/SharePoint is Release 2.
- [x] Unified catalog: files, proto-blobs, verified/suspect replicas, provider checksum bindings. (PR 3; local walk still PR 4)
- [ ] `search`, `list --collection`, `dups` (confirmed `ContentId` groups vs name+size suspects). Cross-provider dups (USB + Drive + OneDrive) group onto one blob; consolidation plans keep one verified copy at the destination and evict redundant local/USB copies (cloud-side duplicate delete is Release 2).
- [ ] `capacity` — used / free / quota / staging reserve.

### Policy

- [ ] YAML collections, first match wins. Archives before Gaming Clips in the example.
- [ ] `keep_local`: `required` | `prefer` | `cloud_only`.
- [ ] `cloud: most_free | specific | any`.
- [ ] `min_replicas`, `pin_local`, `prefer_local`, `dest_subdir` + basename.
- [ ] `prefer` that cannot keep a file local is Satisfiable with a `replica_shortfall` warning.

### Planner (user-confirmed, never auto-destructive)

- [ ] Dry-run `portage plan` / `plan show` with per-op `residual_during` and `residual_after`.
- [ ] Priority: upload-then-evict → evict → upload-keep → **shuttle → download**.
- [ ] Never emit a plan whose trough on any local volume is below `staging_reserve` (default 1 GiB).
- [ ] Last-copy: never schedule delete of the only **verified** replica. Suspect ≠ replica.
- [ ] Cloud-to-cloud is a local shuttle (download to staging, upload, delete staging). Staging may live on an internal volume **or** a connected external disk whose role includes `shuttle`.
- [ ] `--allow-cloud-delete` parsed and rejected in MVP.
- [ ] Unsatisfiable plans print suggestions; they do not apply anything.
- [ ] Property tests: P-space, P-last-copy, P-prefix-safe. YAML-loaded 4 GiB fixture.

### Apply

- [ ] `portage apply PLAN` requires typing the exact plan id (`y` / `yes` / empty rejected).
- [ ] Serial executor. Dual SpaceDrift preflight before each op.
- [ ] tmp + same-volume rename. Dest is not final until hash verify.
- [ ] Upload verify = native digest (MD5 / SHA1 / …) computed on the bytes we sent vs API checksum. Not BLAKE3==MD5.
- [ ] `assert_parent_private` before write; `assert_private` walks ancestors. `we_created` compensation delete.
- [ ] Crash journal + `resume`. Last-copy `Permit` required for any delete.
- [ ] `status`, `flags.max_apply_bytes` soak limiter.

### Safety / ops

- [ ] Structured **JSON-lines logs** (`%data_dir%/logs/portage.YYYY-MM-DD.jsonl`, 10 MiB × 7) with stable fields — scrapeable by a user-run Grafana Alloy/Promtail → Loki. Redaction layer (tokens / `Authorization` / `session_uri`) with merge-blocking unit tests.
- [ ] **Prometheus-text metrics snapshot**: `portage status --format=prom` + atomic `%data_dir%/metrics/portage.prom` for a local textfile collector. No listener, no push, no telemetry.
- [ ] CI **coverage gate**: `cargo llvm-cov` ≥ 80% line coverage on core/catalog/engine. Every PR ships unit tests + ≥1 integration test per touched boundary.
- [ ] Tokens in OS keyring; Windows fallback `%data_dir%/tokens.dpapi`. Never YAML.
- [ ] OAuth UX: open browser → pick Google/Microsoft account → approve → return. BYO `PORTAGE_GOOGLE_CLIENT_ID` / `PORTAGE_MS_CLIENT_ID`. Full Google `drive` scope; consent copy explains why `drive.file` cannot inventory existing clips.
- [ ] `undo` builds a reverse plan and requires a second typed id; refuses last-copy / reserve breaches.
- [ ] `doctor` — catalog integrity, overlay detection, tokens, `NeedsAttention`, last-upload ACL recheck.
- [ ] CI `rg` forbids share-link / `anyoneWithLink` endpoints in provider code.
- [ ] No telemetry. No public-link feature.

## Release 1 after safety MVP (PRs 15–16)

- [ ] **PR 15** `portage-tui` (ratatui): color inventory/plan review, hotkeys, configurable theme. Apply still requires typing the plan id. Does not block PRs 1–13.
- [ ] **PR 16** Clarify-then-plan agent (`portage ask` / `portage-nl`). Desire (what goes where) + priority (what to free or keep first). Up to 3 clarify questions, then a dry-run. **Grok online** and **local** OpenAI-compatible (Ollama / LM Studio). Online sees a redacted catalog digest unless `nl.send_paths`. **Never applies.** Optional read-only stub after PR 5.

## Future releases (not R1)

- [ ] Microsoft 365 / SharePoint site drives (**Release 2**).
- [ ] Dropbox provider.
- [ ] S3-compatible (Backblaze B2, Wasabi, AWS) with block-public-ACLs.
- [ ] SMB / NAS provider.
- [ ] `--allow-cloud-delete` actually implemented, default off, still last-copy gated.
- [ ] USN Journal incremental local index.
- [ ] Optional `ffprobe` for duration/resolution when `PORTAGE_FFPROBE` is set.
- [ ] FTS5 search.
- [ ] Extra LLM providers (OpenAI, Anthropic, local).
- [ ] Android / other OS clients.
- [ ] VM isolation of transfers or the catalog (idea only).
- [ ] Published OAuth client id.

## Later / non-goals for now

- Real-time sync daemon, Cloud Filter provider, FUSE.
- Chunk-level dedup, transcoding, backup versioning / ransomware timelines.
- Encryption-at-rest of user file contents (not a zero-knowledge vault).
- Any "anyone with the link" or third-party cloud-to-cloud SaaS.

## Example policy

Checked-in fixture: [configs/examples/gaming-clips.yaml](../configs/examples/gaming-clips.yaml) (name is historical; treat it as the planner’s YAML fixture, not a product vertical).

| Collection | Match | Placement |
| --- | --- | --- |
| Archives (first) | path contains Archive / Archives / OldClips, or zip/7z/rar | cloud-only Google Drive |
| Gaming Clips | video mime/ext and **not** those archive paths | prefer local D: + most-free of Drive/OneDrive, min 2 replicas |
| Documents | office/pdf/md | prefer local + most-free cloud |
| default | everything else | prefer local + most-free cloud |

Worked 4 GiB case (see design): evict a 3.50 GiB archive clip to Google, then download 2.20 + 2.00 + 1.80 of keep-local clips. Minimum residual on D: is **1.50 GiB** (reserve 1.00).
