# Saved folder comparisons

Start `sidemirror` without arguments. The home screen lists saved comparisons and an explicit **New comparison** row. A profile is a complete comparison session—not a connection bookmark.

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

Source/destination labels identify the two columns only: folder scanning is read-only; the file diff supports confirmed copying in either direction. The inherited transfer/connection-bookmark screen remains available through `sidemirror transfer`.

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
| Up/Down or k/j         | Select a shared source/destination row                                      |
| Enter / Right          | Enter a logical directory pair or open text diff                            |
| Left / Backspace       | Parent directory                                                            |
| d                      | Toggle Show: All files & folders / Differences only                         |
| f                      | Direct children / recursive descendant list                                 |
| n / p                  | Next / previous non-equal entry, wrapping within the visible list           |
| Page Up/Down, Home/End | Move through rows                                                           |
| i                      | Ignore the selected file/folder on both sides without restarting comparison |
| r                      | Refresh the recursive scan                                                  |
| c / Escape             | Cancel after the current I/O operation                                      |
| o                      | Return to saved comparisons                                                 |
| s                      | Edit/save the active comparison                                             |
| a                      | Reconnect with separate endpoint password prompts                           |
| F4                     | Settings → Themes (also on the home screen and in an idle text diff)        |
| ?                      | Keyboard help                                                               |
| q / Ctrl-C             | Quit                                                                        |

In text diff, n/p jump between changed hunks and Escape returns to folders.

## Profile storage

Profiles are saved atomically in `comparisons.json` under the sidemirror configuration directory. Version 3 stores typed `source`/`destination`, name, ignore globs and the optional `remember_passwords` boolean (defaults to false in existing profiles). Endpoint kinds are `local`, `remote` (URL shortcut), `bookmark`, and `connection` (validated, secret-free inherited connection settings). Passwords, secret access keys and session/security token values are never written to this JSON file; opt-in remembered passwords/default secrets live only in the OS keychain.

Versions 1 and 2 load automatically without filesystem/network access. Version-1 `left`/`right` entries become local-source comparisons; version-2 source/destination entries are retained. The next successful save/update/delete writes all loaded profiles as version 3. Connection bookmarks remain separate and are not automatically converted into comparison sessions.

Ignore globs match slash-separated relative paths. Matching directories are pruned; `.git` ignores the root `.git`, and `*/.git` also ignores nested ones. Hidden files are included unless ignored. Paths match case-sensitively.

In comparison view, select a file or directory and press **i** to append its exact root-relative path to the ignore patterns for both sides. A directory rule also excludes its descendants. Rules are immediately persisted for saved profiles; unsaved comparisons keep them in memory until saved with `s`. The entry and any descendants disappear immediately, and directory summaries are recalculated from existing results. Other results, the current folder, and any ongoing scan are preserved: no restart, relisting, or repeat verification is triggered. Incoming updates are filtered too, so an active scan cannot bring ignored entries back. The running worker may finish work already scheduled under its original rules; an explicit `r` refresh or reopening uses the updated rules for scanning. No endpoint files are modified. Paths containing `*`, `?`, or `;`, or non-UTF-8 names, cannot be added literally this way; the action reports this instead of creating an overly broad rule. Use `s` to edit globs manually.

## Results and safety

The **Show: All files & folders / Differences only** toggle sits above the comparison columns. The active option is highlighted and bracketed; press `d` to switch instantly without rescanning. It applies to both files and folders in both columns, including nested and recursive views. Differences only hides confirmed-equal entries; pending, unverified and error entries remain visible so incomplete scans cannot look equal. `--differences` starts in this mode.

Both columns share ordering, selection, and filtering. Missing-side directories remain navigable placeholders; browsing never creates them.

Statuses are equal, different, left-only (source only), right-only (destination only), type conflict, pending, unverified, and error. Files with equal sizes are compared by streaming bytes, not timestamps. Different known sizes indicate a difference without downloading content. Remote verification may transfer substantial data from both servers.

Directory statuses aggregate descendants after scanning. Inaccessible/cancelled subtrees are not reported as equal. Symlinks and special files are not followed; paired entries stay unverified. Scans are observations, not snapshots: changes during a scan may require refresh.

Entries appear incrementally as each folder listing returns, first on the source and then on the destination; paired rows merge in place as the other side is discovered. Directories can be browsed while scanning. A side not listed yet is not treated as missing: discovered rows remain pending until inventory is complete, then byte-verification updates stream in batches. Refresh clears stale rows and streams the new inventory. Progress messages identify the current listing or file verification. A single blocked remote listing cannot be displayed until the server returns it.

Worker-owned connections keep keyboard navigation responsive. Cancellation is cooperative: blocked connection/list/read/finalization calls must return before cancellation takes effect. Old job results cannot replace the current session.

Text diff supports UTF-8 and Windows-1252 files up to 5 MiB and 10,000 lines per side. Binary files retain folder byte-comparison results but cannot open as text. `␍` marks CRLF; `[no final newline]` marks an unterminated final line. In file diff, use `n`/`p` to select a changed chunk, `F5` to copy it left → right, or `F6` right → left. Use `F7` / `F8` to copy the entire file left → right / right → left. Confirm with `y`; any other key cancels. Copies write immediately and reload the diff; Escape returns and rescans folders. Both snapshots are checked before writing; stale files must be reopened. Local replacement is atomic and retains permissions; remote writes are not atomic and failures may leave partial contents. Concurrent changes during writing cannot be prevented. Only existing paired regular text files are supported; deletion, automatic synchronization, and arbitrary remote command execution are unavailable. SCP and Kubernetes may run read-only listing/stat/read commands internally through their inherited backends.

Opening a paired file with Enter interrupts an active folder scan; it does not wait for the entire folder comparison. A currently blocked remote I/O operation must return first. Returning from the diff rescans the folders.

Text decoding uses UTF-8 first (preserving BOM), then Windows-1252 as a fallback, not general-purpose encoding detection. ASCII-only files are treated as UTF-8. Each column shows its encoding. Chunks retain the destination encoding and reject unrepresentable characters; whole-file copies retain the source bytes and encoding. Decoded text may be identical even when folder byte comparison reports different encodings.

Both file panes use filename-based syntax highlighting with colors matching the selected theme. Multiline comments and strings retain their syntax state. Unknown file types fall back to plain text. Changed chunks have a tinted background and the selected chunk is highlighted separately; syntax colors do not replace difference indicators. Highlighting is cached when opening or reloading a diff.
