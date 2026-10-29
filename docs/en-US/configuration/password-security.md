# Password security

## Comparison profiles

Native credential storage is mandatory and always enabled. Successfully used passwords and S3 secret access keys are stored directly in the OS credential store and reused for the same endpoint settings. Secrets never enter `comparisons.json`. There is no session-only password mode, opt-out, or file-storage fallback: an unavailable or locked store produces an explicit error. `--wno-keyring` is rejected. Native storage is always compiled, including with `--no-default-features`; the legacy `keyring` feature is only a compatibility alias. Older profiles cannot disable this policy.

Temporary S3 session/security tokens are not passwords and remain ephemeral. Use your OS credential manager to remove saved entries. SSH agents and credential profiles remain supported. See [folder comparison](../usage/folder-comparison.md) for details and current transport-verification limitations.

## Connection bookmarks

Connection bookmarks also require native credential storage.

Bookmarks are saved in your configuration directory along with their passwords.
Passwords are not stored in plaintext: they are encrypted with AES.

The key used to encrypt passwords is stored exclusively in the operating
system secret store:

- The Windows Vault on Windows
- The system keyring on Linux
- The Keychain on macOS

This is managed directly by your operating system.

Systems without an available native store fail explicitly; keys are never saved in plaintext files. WSL requires a working Linux Secret Service. Existing file-key bookmarks are not automatically migrated or deleted: keep the old files until you have recovered your credentials and saved them securely again.

## Linux keyring

On Linux there might be no keyring installed on your system. The key storage
requires a service that exposes `org.freedesktop.secrets` on D-Bus, and only a
few services provide it:

- If you use GNOME as your desktop environment (e.g. Ubuntu users), the keyring
  is already provided by `gnome-keyring` and everything should work out of the
  box.
- For other desktop environments, you can use [KeepassXC](https://keepassxc.org/)
  to obtain a keyring. It must be set up to work with sidemirror; see
  [KeepassXC setup](#keepassxc-setup) below.
- A working, unlocked Secret Service and session D-Bus are required. Without them, credential operations fail; no fallback is provided.

### KeepassXC setup

Follow these steps to set up KeepassXC for sidemirror:

1. Install KeepassXC.
2. Go to "Tools" > "Settings" in the toolbar.
3. Select "Secret service integration" and enable "Enable KeepassXC
   freedesktop.org secret service integration".
4. Create a database, if you do not have one yet: from the toolbar, "Database" >
   "New database".
5. From the toolbar, go to "Database" > "Database settings".
6. Select "Secret service integration" and enable "Expose entries under this
   group".
7. Select the group where the sidemirror secret will be kept. Note that any other
   application can read secrets exposed via D-Bus for that group.
