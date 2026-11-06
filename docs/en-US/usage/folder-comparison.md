# Saved folder comparisons

Full folder comparisons and text diffs ignore CRLF versus LF line-ending differences. Other whitespace and missing final newlines remain significant. Original file bytes are preserved when copying files.

Start `sidemirror` without arguments. The home screen lists saved comparisons and an explicit **New comparison** row. A profile is a complete comparison session—not a connection bookmark.

## Quick comparison (default) and full comparison

Opening a comparison runs a **quick, metadata-only scan by default**. Unchecking both quick-check boxes in a profile instead runs full content verification on open and refresh. In quick mode, both endpoints are listed recursively, but file contents are not downloaded, hashed or opened. By default, paired regular files are compared using both their listed size and modification time. Each profile can instead use **size only** or **timestamp only**. Timestamp checks use **whole-second UTC precision** (subseconds are discarded, with no extra tolerance or timezone adjustment).

- **≈ quick match**: all enabled metadata checks match. This is **not proof that contents are identical**; size-only checks miss same-size edits, and timestamp-only checks miss edits retaining the timestamp even if sizes differ.
- **≠ different**: a known, enabled metadata check differs. Identical contents with different timestamps (when enabled), or equivalent CRLF/LF contents with different raw sizes (when enabled), may appear different.
- **? unverified**: metadata required by an enabled check is missing and the available enabled checks do not establish a mismatch. Disabled criteria are ignored, including missing metadata. No automatic content download is attempted.

### Per-profile quick checks

In the profile editor (`s` in comparison view, `e` on the home screen, or `n` for a new profile), Tab to the **Quick checks** checkboxes:

- **[x] Size  [x] Timestamp** — both checks (**default** for new and existing profiles).
- **[x] Size  [ ] Timestamp** — size-only; differing timestamps are ignored.
- **[ ] Size  [x] Timestamp** — timestamp-only; differing sizes are ignored.
- **[ ] Size  [ ] Timestamp** — quick checks disabled; **full normalized-content comparison** on opening, reconnecting and refreshing (`r`). This can transfer substantial data.

Press Space or Left/Right to toggle the focused checkbox. Both may be unchecked to disable quick checks; this never marks files equal without content verification. Ctrl-U on either checkbox restores both. Enter/Ctrl-S saves and opens the profile. **Size-only is off by default**; enable it explicitly by unchecking Timestamp. The choice is retained on reopening, duplicating and renaming a profile. The comparison header shows the active criteria or Full mode. Post-copy checks use the same mode/criteria without restarting the comparison. No general setting is needed and other profiles are unchanged.

Size-only avoids timestamp-only false differences, but cannot detect changed contents of the same length. Matching line endings removes CRLF/LF size mismatches; quick checks still cannot prove content equality. FTP listings may expose coarse or unrelated modification dates, so timestamp-only checks are also approximate. **F5 always performs full normalized-content verification regardless of these checkboxes.**

Press **F5 in the folder comparison view** for a **full comparison of all non-ignored files**, not just the selection or quick differences. It obtains a fresh inventory and runs normalized-content verification even for quick matches. Only full content matches are labeled **= equal**. Existing strong-version-validated hash-cache reuse remains available; matching sizes/times alone never bypass full verification. Full comparison can transfer substantial data.

Press **r** to refresh using the profile's quick-check selection (full comparison if both boxes are unchecked). Opens, reconnects and editor saves use that selection too. Copies recheck only the copied paths without restarting the comparison or changing its mode. Returning from a text diff also preserves the existing comparison. F5/r can replace a running scan; active I/O is drained first, and stale results cannot overwrite the newer scan. The current mode stays visible beside the elapsed time. Existing profiles keep the previous both-checks behavior without migration. **F5 inside a text diff retains its existing copy-chunk action**, requiring confirmation.

## Create, open, and update a comparison

1. Select **New comparison** and press Enter (or press `n`).
2. Enter a comparison name.
3. In **Source settings**, select Local or a remote protocol and fill its fields.
4. Independently select and configure **Destination settings**.
5. Enter the folder on each endpoint, plus optional semicolon-separated ignore globs.
6. Press Enter or Ctrl-S to **save and open** the comparison.

Use Tab/Shift-Tab or Up/Down to move through fields. F1 selects the name, F2 the source, and F3 the destination. Each endpoint panel scrolls independently to its selected field. Space or Left/Right cycles the type or radio options. On the type selector, `l` chooses Local, `r` SFTP, `u` a URL shortcut, and `b` a connection bookmark. Ctrl-U clears a text field. Switching connection types clears session passwords/tokens to avoid carrying them into another protocol. Local roots must exist when saving; remote settings are validated offline and connect only when opened.

Text fields show a terminal cursor at the insertion point and scroll horizontally when necessary, including masked passwords and wide Unicode characters.

Passwords and S3 secret access keys entered for either endpoint are always saved directly in the native OS credential store after a successful connection and reused on reopening. Native storage is mandatory; there is no F4 opt-out, session-only password mode, or file fallback. Temporary S3 session/security tokens remain ephemeral. Native storage is always compiled, even with `--no-default-features`.

Both endpoints offer the inherited termscp settings:

| Type                    | Settings                                                                                                                                          |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Local                   | Folder                                                                                                                                            |
| SFTP / SCP / FTP / FTPS | Host or SSH alias, port, username, masked password, folder                                                                                        |
| S3                      | Bucket, region, endpoint, AWS profile, access key ID, masked secret access key/security token/session token, path-style addressing, folder prefix |
| GCS                     | Bucket, endpoint, service-account JSON file (blank uses ADC), folder prefix                                                                       |
| Kubernetes              | Namespace, cluster URL, username, certificate/key file paths, folder (`/pod/container/path`, or `/` to list pods)                                 |
| WebDAV                  | HTTP(S) URI, username, masked password, folder                                                                                                    |
| SMB                     | Host, username, masked password, share, folder; POSIX also offers port, workgroup and Auto/SMB1/SMB2/SMB3 dialect                                 |
| URL shortcut            | Standard SFTP/SCP/FTP/FTPS or HTTP(S) WebDAV URL, optional session password                                                                       |
| Connection bookmark     | Saved connection name (optionally `bookmark:NAME`), optional session password/default secret override                                             |

SMB is offered only in builds with the `smb` feature. Windows manages SMB dialect negotiation; POSIX-only settings are retained but not editable there. SMB1 is insecure and should be limited to isolated legacy devices. URL-based saved profiles expand into full protocol-specific fields when edited.

Source and destination each support local or remote folders, including **remote↔remote**. All connection-bookmark protocols use the same inherited clients as explicit settings. Folder paths select existing roots, not copy destinations; saving never creates endpoint folders.

Select a saved comparison and press Enter to reopen it. Press `e` in the home screen, or `s` inside a comparison, to edit settings. Saving **updates that same profile and immediately reopens it** with the changed endpoints/rules. Renaming updates the profile atomically rather than creating a duplicate. Names cannot collide with another saved comparison. `d` in the home screen deletes a profile after confirmation; it never deletes endpoint files.

Source/destination labels identify the two columns only: folder scanning is read-only; the folder browser and file diff support confirmed copying in either direction. The inherited transfer/connection-bookmark screen remains available through `sidemirror transfer`.

Press `o` in the comparison screen to return to the saved comparison list. At the root list, Escape quits; it does not reopen the previous comparison. Inside a profile folder, Escape returns to the root list.

## Organize saved profiles

On the comparison home, select a profile and press `c` to duplicate it. The editor opens with copied settings and a unique name; Enter saves and opens the copy, while Escape cancels without saving. Session passwords/tokens are not copied.

Press `f` to create a single-level organizational folder. Enter opens a folder; Backspace or Escape returns to the root list. Select a profile and press `m` to list available folders, including **Root (no folder)**. The current folder is marked and selected. Use Up/Down (or j/k), Home/End, and Enter to move the profile; Escape cancels. New and duplicated profiles are saved in the currently open folder. Renaming retains membership; deleting a profile leaves its folder intact. Select a folder and press `d` to delete it after confirmation (`y` confirms; `n`/Escape cancels). Its profiles return to the root list; no profiles or endpoint files are deleted. Profile names remain globally unique, so CLI `--profile NAME` is unchanged.

These folders organize profiles only and never create, move, or modify endpoint files. Folder names and membership are persisted alongside profiles in `comparisons.json`; older files without folders still load.

## Appearance

The comparison UI defaults to **Catppuccin Mocha**. From the home screen, folder
browser, or idle text diff, press **F4** → **Themes**, select a theme with Up/Down
or j/k, then Enter to apply and save. **Tokyo Night** (terminalcolors.com Default palette) and
**Terminal default** are also available.
Escape returns to Settings, then to the previous screen without losing its
selection. This preference is separate from comparison profiles and the legacy
transfer theme. See [Themes](../configuration/themes.md) for storage details.

## Parallel comparisons

For **full comparison (F5)**, the built-in default is **4 simultaneous file pairs**. Quick comparison only lists the two endpoints and does not open extra content-verification connections. To change the general default, press **F4** → select **Parallel comparisons** with Up/Down or j/k → use Left/Right (or `-`/`+`) to decrease/increase the value. Values range from **1 to 16**, and each change is saved immediately. Escape returns to the previous screen.

In a comparison's settings (`n` for new, `e` from the home screen, or `s` from the browser), Tab to **Parallel comparisons**. Left/Right or Space cycles between **Use default (N)** and explicit counts from 1 to 16; **Ctrl-U** restores Use default. Enter/Ctrl-S saves and opens the comparison. New and older profiles inherit the general default; an explicit profile count stays unchanged when the general default changes, even if it originally equaled that default. Editing or duplicating a profile preserves its override.

The count means file **pairs**, not total requests: each pair has one connection per endpoint and can read both sides concurrently. A limit of N permits up to N connections per endpoint and 2N active filesystem operations. Additional connections are opened only when there are enough files to use them; server connection limits may reduce actual parallelism. Higher counts can help many small remote files but may overload a server or provide little benefit when bandwidth is already saturated.

Changes apply on the **next open or refresh (`r`)**, not midway through an active scan. Inheriting profiles resolve the current general default on every refresh. Reducing the limit releases excess worker connections before the next scan.

The general default is stored as `parallel_comparisons` in `comparison-ui.json`, alongside `theme`. A missing or invalid general value falls back to 4. A profile's optional `parallel_comparisons` field in `comparisons.json` stores its override; omitted or `null` means inherit. Explicit profile values outside 1–16 are rejected. Theme and general-default changes preserve each other.

## CLI shortcuts

From a source checkout:

```sh
cargo run --no-default-features --features keyring
cargo run --no-default-features --features keyring -- compare ./source ./destination
cargo run --no-default-features --features keyring -- compare ./local 'sftp://user@server:22/folder' --save website
cargo run --no-default-features --features keyring -- compare --profile website
```

Installed/release binary:

```sh
sidemirror compare 'sftp://user@server:22/source' ./local --save download-review
sidemirror compare 'sftp://user@server-a/source' 'ftps://user@server-b/destination' --save server-review
sidemirror compare bookmark:source-server bookmark:destination-server
sidemirror compare ./source ./destination --ignore '.git' --ignore '*.tmp' --differences
```

`compare` without endpoints is an alias for the home screen. Comparison URLs use **standard URL paths**, unlike the inherited transfer CLI's colon-separated folder syntax.

SSH agent, configured SSH keys/config files, AWS profiles/default credentials, GCS ADC, kube configuration, and connection-bookmark credentials are reused independently for each endpoint. A blank SSH port follows SSH configuration; FTP defaults to 21. Passwords and S3 secret keys/tokens entered in settings are masked, passed only to that endpoint's worker client, and never saved in comparison profiles. Access key IDs and credential-file paths are saved, not file contents. Passwords and S3 secret access keys are reused automatically from the native credential store. Re-enter temporary tokens when reopening, or use credential profiles.

Remembered passwords are scoped to the exact endpoint settings (including protocol, user, host and folder), not the comparison name. Renaming a profile retains access; profiles using identical endpoint settings can reuse the same keychain entry. Changing settings requires entering credentials again. A newly entered password overrides the saved one and replaces it only after successful authentication. Leaving a password prompt blank reuses existing authentication. Keychain read/write failures are reported explicitly; this feature never falls back to file storage. Remove saved entries using the operating system's credential manager. `--wno-keyring` is rejected; build flags cannot disable native storage. Older profiles with password remembering disabled now use mandatory native storage too.

Native keychain entries use readable connection names rather than serialized settings, with a short fingerprint to distinguish exact endpoint identities. Older entries are copied to the new name when first read; macOS may show the old name once during that migration. Original entries are left intact.

`--password-prompt` and `a` reconnect with separate masked password/default-secret prompts (the default secret for S3 is its secret access key). Typed GCS/Kubernetes settings do not ask for a generic password. Use `s` to update credential-file paths or all S3 token fields. Never put credentials or signed queries in server URLs. If the native store is unavailable or locked, unlock or configure it before retrying; no fallback is available.

**Security limitation:** the inherited SFTP/SCP clients do not verify SSH server host keys, and the current native-TLS FTPS client accepts invalid certificates/hostnames. Use only trusted endpoints until verification is implemented. Plain FTP is unencrypted.

## Keyboard controls

| Key                    | Action                                                                      |
| ---------------------- | --------------------------------------------------------------------------- |
| Up/Down or k/j         | Select a shared source/destination row; clear any range selection            |
| Shift+Up/Down          | Extend or shrink a range of selected files and folders                      |
| F7 / F8                | Copy selected entries (or focused row) left → right / right → left; confirm with y |
| Enter / Right          | Enter a logical directory pair or open text diff                            |
| Left / Backspace       | Parent directory                                                            |
| d                      | Toggle Show: All files & folders / Differences only                         |
| f                      | Direct children / recursive descendant list                                 |
| n / p                  | Next / previous non-equal entry, wrapping within the visible list           |
| Page Up/Down, Home/End | Move through rows                                                           |
| i                      | Ignore the selected file/folder on both sides without restarting comparison |
| r                      | Refresh using profile checks; full content verification if both unchecked |
| F5                     | Full recursive content comparison of all non-ignored entries (folder view)  |
| c / Escape             | Cancel after the current I/O operation; Escape clears an idle selection     |
| o                      | Return to saved comparisons                                                 |
| s                      | Edit/save the active comparison                                             |
| a                      | Reconnect with separate endpoint password prompts                           |
| F4                     | Settings → Themes (also on the home screen and in an idle text diff)        |
| ?                      | Keyboard help                                                               |
| q / Ctrl-C             | Quit                                                                        |

Selected rows have checkmarks and a highlighted background; the browser title shows the selected count. Shift+Up/Down selects from the initial row through the cursor, including both endpoints. Reversing direction shrinks the range. Plain row navigation clears the range, as does entering/leaving a directory or refreshing. Filtering drops selections that are no longer visible; incoming scan rows preserve selections by path.

In the folder browser, **F7** copies left → right and **F8** copies right → left. Without a range, only the focused row is copied. A confirmation lists the selected paths; `y` confirms and any other key cancels without writing. Folder copies include all non-ignored descendants, even when hidden by the Differences only filter. Existing files are overwritten, missing files and parent directories are created, and destination-only files are retained. Files are copied byte-for-byte, including binary files, with no text-diff size limit. A selected entry missing on the sending side is an error, not a deletion request.

Discovery finishes before writing. Symlinks and special files are not followed or copied; source discovery errors stop the batch before writing. Destination type conflicts and later I/O errors stop the batch without replacing directories or links. Completed entries are **not rolled back**. Files are spooled one at a time through temporary local storage, keeping memory bounded but requiring local free disk space. Local file replacement is atomic and preserves existing destination permissions; remote writes are not atomic and may leave partial files on failure. Copies use live source contents, not scan-time snapshots, and cannot prevent external changes during copying. `c`/Escape requests cooperative cancellation after current I/O; other controls are disabled until copying and its targeted recheck finish. After success or failure, only the selected paths are rechecked in the current comparison mode (a selected folder includes its subtree); unrelated folders are not rescanned. Parent metadata and directory summaries are updated to reflect any created directories. Cancellation or a failed recheck leaves affected old results unverified rather than claiming equality. The copy outcome is reported without restarting the comparison, resetting its elapsed time, or changing its mode. Copying does not guarantee matching modification times; use F5 to verify contents even if a quick result still differs.

In text diff, n/p jump between changed hunks and Escape returns to folders.

## Profile storage

Profiles are saved atomically in `comparisons.json` under the sidemirror configuration directory. Version 3 stores typed `source`/`destination`, name, ignore globs and the optional `remember_passwords` boolean (defaults to false in existing profiles). Endpoint kinds are `local`, `remote` (URL shortcut), `bookmark`, and `connection` (validated, secret-free inherited connection settings). Passwords, secret access keys and session/security token values are never written to this JSON file; opt-in remembered passwords/default secrets live only in the OS keychain.

The optional `quick_check` profile field stores `"size"`, `"timestamp"`, `"both"` or `"none"`; omission means `"both"`. `"none"` disables metadata shortcuts and selects full content verification. The default may be omitted when saving. Unknown, null or non-string values are rejected instead of silently selecting a weaker check. Profile document version remains 3.

Versions 1 and 2 load automatically without filesystem/network access. Version-1 `left`/`right` entries become local-source comparisons; version-2 source/destination entries are retained. The next successful save/update/delete writes all loaded profiles as version 3. Connection bookmarks remain separate and are not automatically converted into comparison sessions.

Ignore globs match slash-separated relative paths. Matching directories are pruned; `.git` ignores the root `.git`, and `*/.git` also ignores nested ones. Hidden files are included unless ignored. Paths match case-sensitively.

In comparison view, select a file or directory and press **i** to append its exact root-relative path to the ignore patterns for both sides. A directory rule also excludes its descendants. Rules are immediately persisted for saved profiles; unsaved comparisons keep them in memory until saved with `s`. The entry and any descendants disappear immediately, and directory summaries are recalculated from existing results. Other results, the current folder, and any ongoing scan are preserved: no restart, relisting, or repeat verification is triggered. Incoming updates are filtered too, so an active scan cannot bring ignored entries back. The running worker may finish work already scheduled under its original rules; an explicit `r` refresh or reopening uses the updated rules for scanning. No endpoint files are modified. Paths containing `*`, `?`, or `;`, or non-UTF-8 names, cannot be added literally this way; the action reports this instead of creating an overly broad rule. Use `s` to edit globs manually.

## Results and safety

The **Show: All files & folders / Differences only** toggle sits above the comparison columns. The active option is highlighted and bracketed; press `d` to switch instantly without rescanning. It applies to both files and folders in both columns, including nested and recursive views. Differences only hides both full-equal entries and quick metadata matches; pending, unverified and error entries remain visible. Hidden quick matches are not content-verified. `--differences` starts in this mode.

Both columns share ordering, selection, and filtering. Missing-side directories remain navigable placeholders; browsing never creates them.

Statuses are quick match, equal, different, left-only (source only), right-only (destination only), type conflict, pending, unverified, and error. Quick comparison uses listing metadata only, as described above. **Full comparison (F5, or both quick checks disabled)** verifies paired regular files using SHA-256 of their line-ending-normalized contents, never timestamps alone. Early normalized-byte mismatches stop verification; otherwise uncached verification reads the complete files. Different sizes still require verification because CRLF and LF files can have equivalent contents. Remote verification may transfer substantial data from both servers.

Directory statuses aggregate descendants after scanning; a directory's own size and modification time are not compared. A file's enabled metadata difference, a missing entry or a type conflict can make its ancestor directories different, even when the directory timestamps match. Unverified, pending and error descendants propagate uncertainty instead of a match. Inaccessible/cancelled subtrees are not reported as equal. Symlinks and special files are not followed; paired entries stay unverified. Scans are observations, not snapshots: changes during a scan may require refresh.

Source and destination inventories run concurrently. Entries appear incrementally as each folder listing returns, in either order; paired rows merge in place as the other side is discovered. Directories can be browsed while scanning. A side not listed yet is not treated as missing: discovered rows remain pending until inventory is complete, then metadata classification (quick) or content-verification (full) updates stream in batches. Refresh clears stale rows and streams the new inventory. Progress messages identify the current listing or file verification. A single blocked remote listing cannot be displayed until the server returns it.

Worker-owned connections keep keyboard navigation responsive. Cancellation is cooperative: blocked connection/list/read/finalization calls must return before cancellation takes effect. Old job results cannot replace the current session.

Unchanged idle screens are not continuously redrawn. Background updates and the live comparison timer repaint at most ten times per second; keyboard input and resizing request an immediate repaint. Status-only updates avoid rebuilding the whole folder list unless they affect filtering or ordering. Content comparison processes line endings in bounded chunks; it still verifies complete uncached equal files and can use multiple CPU cores at higher parallelism settings. Use a release build when measuring performance.

The **Comparison** line above the file filters shows live elapsed time in seconds with millisecond precision (for example, `Comparison: running · 12.345s elapsed`). When the requested quick or full scan finishes, it freezes as `Comparison: completed in 12.345s`, and the completion message also reports the duration. The total measures wall-clock time from starting/opening or requesting a refresh through recursive inventory, content verification when requested, and directory aggregation—not just the time until the first rows appear. Initial connection and cache setup are included, as is any wait for an earlier cancelled operation to finish. Waiting at a password-entry prompt before starting is excluded. The final duration is recorded by the worker, so delayed UI redraws do not inflate it.

Every new comparison, `r` profile-based refresh or `F5` full comparison resets the timer. The final time remains visible during subsequent folder navigation, filtering and status messages. Cancelled, incomplete, failed and interrupted scans are labeled separately rather than reported as completed. Opening a file diff during scanning interrupts that comparison; text diff/copy work is not added to the comparison time. This is elapsed time, not an estimated time remaining.

Text diff supports UTF-8 and Windows-1252 files up to 5 MiB and 10,000 lines per side. Binary files retain folder byte-comparison results but cannot open as text. `␍` marks CRLF; `[no final newline]` marks an unterminated final line. In file diff, use `n`/`p` to select a changed chunk, `F5` to copy it left → right, or `F6` right → left. Use `F7` / `F8` to copy the entire file left → right / right → left. Confirm with `y`; any other key cancels. Copies write immediately and reload the diff. After a copy attempt, only that file is rechecked using full normalized-content verification, and its folder row and ancestor summaries are updated. Escape returns without restarting the folder comparison, including after copying. Both snapshots are checked before writing; stale files must be reopened. Local replacement is atomic and retains permissions; remote writes are not atomic and failures may leave partial contents. Concurrent changes during writing cannot be prevented. Text-diff copying supports only existing paired regular text files; use the folder browser to copy missing files, binary files, or folders. Deletion, automatic synchronization, and arbitrary remote command execution are unavailable. SCP and Kubernetes may run read-only listing/stat/read commands internally through their inherited backends.

Opening a paired file with Enter interrupts an active folder scan; it does not wait for the entire folder comparison. A currently blocked remote I/O operation must return first. Escape or `q` returns from a read-only diff without restarting the comparison, preserving its rows, selected path, current folder, filters, comparison mode and elapsed time. A cancelled copy confirmation does not trigger a refresh either. If opening the diff interrupted an active scan, its partial results and interrupted timing remain; use `r` for a fresh profile-based scan or F5 for full comparison. Confirmed diff-copy attempts update only that file and its ancestor summaries, including on failure; no automatic whole-tree refresh is queued. Possibly partial writes whose result cannot be checked remain unverified. The diff keeps its original file path even if a matching result hides that file from Differences only. Unrelated changes made externally are discovered only when you explicitly refresh with `r` or F5.

Text decoding uses UTF-8 first (preserving BOM), then Windows-1252 as a fallback, not general-purpose encoding detection. ASCII-only files are treated as UTF-8. Each column shows its encoding. Chunks retain the destination encoding and reject unrepresentable characters; whole-file copies retain the source bytes and encoding. Decoded text may be identical even when folder byte comparison reports different encodings.

Both file panes use filename-based syntax highlighting with colors matching the selected theme. Multiline comments and strings retain their syntax state. Unknown file types fall back to plain text. Changed chunks have a tinted background and the selected chunk is highlighted separately; syntax colors do not replace difference indicators. Highlighting is cached when opening or reloading a diff.

## Debugging unexpected quick differences

For a temporary metadata-only diagnostic session, set `SIDEMIRROR_COMPARE_LOG` to a new JSONL filename before launching sidemirror. Reproduce the issue, note an affected relative path, and quit. Optionally open its file diff or run F5 so the same session includes content results. The log records exact sizes/timestamps, compared whole-second values, deltas, mismatch reasons and directory/display aggregation, without changing comparison behavior or downloading extra contents for logging. Filenames and metadata can be sensitive; review before sharing. See [temporary comparison diagnostics](../configuration/logging.md#temporary-folder-comparison-diagnostics) for commands, privacy details and how to disable it.

## Strict verification and hash cache

Full comparison (F5, or both quick checks disabled) verifies up to the configured number of file pairs concurrently (**four by default**), using independent source/destination connections for each pair. The profile override takes precedence over the general default; see [Parallel comparisons](#parallel-comparisons). The first pair lists both sides concurrently; extra pairs connect lazily only when there are multiple files to verify. If additional connections fail, comparison continues with the available pairs. Connections are reused on refresh, but every refresh obtains a fresh recursive inventory, including additions and deletions. Clients and streams stay on their owner threads, with bounded request, result and discovery queues. Completed file results stream independently of slower pairs; the displayed rows retain path ordering. Missing, cancelled or inaccessible subtrees never become equal through cached results.

SFTP reads start with a 64 KiB probe and grow their windows as more data is consumed, up to 16 MiB with bounded prefetch. The next window is not fetched speculatively until the consumer asks beyond the initial probe. This reduces unnecessary downloads when files differ near the start; matching large files still read in full. CRLF normalization can require more raw bytes than the normalized comparison chunk, and sustained reads still read ahead. Ramp-up adds some overhead for fully matching files, so improvements depend on the workload. Pending SFTP read/close failures are reported instead of being hidden during finalization. This behavior is automatic and does not change the parallel-comparison setting.

Cancellation stops assigning new file pairs and drains all active reads and protocol finalizers before the scan finishes or subsequent diff/copy/refresh work starts. Blocked network operations must still return before cancellation can complete. Full-comparison verification and cache validation remain strict; only the explicitly labeled quick mode uses size/timestamp shortcuts. Cache writes are serialized to avoid SQLite writer contention between lanes; file reads and hashing remain concurrent.

Per-endpoint SHA-256 hashes are stored in `comparison-hashes.sqlite3` under the native sidemirror cache directory:

- Linux/BSD: `$XDG_CACHE_HOME/sidemirror` (normally `~/.cache/sidemirror`).
- macOS: `~/Library/Caches/sidemirror`.
- Windows: `%LOCALAPPDATA%\sidemirror`.

Quick comparison is the default and never records metadata matches as content hashes. In full comparison, a cached hash is reused only after a fresh check of a strong content-version identifier. WebDAV currently supports this through strong ETags; weak/missing ETags force live reads, and ETags themselves are never compared as cross-endpoint content hashes. The current Local, SFTP/SCP, FTP/FTPS, S3, GCS, Kubernetes and SMB adapters do not expose a trustworthy content version, so full comparison rereads files even if size and modification time match. Only hashes with reusable strong validators are persisted, avoiding per-file database writes on metadata-only backends. Hashes are recorded after complete successful reads and stream finalization, with metadata checked before and after; detected concurrent changes require refresh. Early mismatches, failed reads and cancelled reads never publish partial hashes.

The database contains hashes, sizes, strong validators and verification times, with hashed endpoint/path keys—not passwords or file contents. Entries older than 30 days are removed and the database is periodically pruned to 100,000 entries. Cache age is housekeeping, not proof of freshness. The cache is disposable: unavailable, locked, corrupt or unsupported databases fall back to live verification without claiming equality. To clear it, close sidemirror and remove `comparison-hashes.sqlite3` and any matching `-wal`/`-shm` files.

Text diffs and pre-copy snapshot checks always read live contents, bypassing the hash cache. The destination cache entry is invalidated before any write attempt, including writes that fail partially. As with uncached scans, absolute consistency during external edits requires storage snapshots/versioned reads; metadata checks do not make a scan atomic.
