<div align="center">
  <img src="./logo/transom-lockup.svg" alt="Transom" width="420" />

  <p>
    <a href="./README.md"><strong>English</strong></a> ·
    <a href="./README_zh-CN.md"><strong>简体中文</strong></a>
  </p>

  <p>
    <a href="https://github.com/mangerle/transom/blob/main/LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-blue" /></a>
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.92%2B-orange" />
    <img alt="Platform" src="https://img.shields.io/badge/platform-Linux%20(primary)%20%7C%20macOS%20%7C%20Windows-informational" />
    <img alt="Status" src="https://img.shields.io/badge/status-alpha-red" />
  </p>

  <p>A native SSH, serial console and terminal workspace with integrated SFTP, written in Rust on GPUI.</p>
</div>

---

**Transom** is a desktop operations client: one window for shells on remote hosts, serial consoles on boards and switches, and the files that go with them. It renders through [GPUI](https://github.com/zed-industries/zed) for a real GPU-backed terminal grid rather than an embedded web view, and every byte of the connection path is native Rust — [`russh`](https://crates.io/crates/russh) for SSH2, [`russh-sftp`](https://crates.io/crates/russh-sftp) for file transfer, [`alacritty_terminal`](https://crates.io/crates/alacritty_terminal) as the VT emulation core.

The name: a *transom* is the crossbar in a doorway, and a porthole is a transom over water. This tool is the bar you look through when the machine you care about is on the far side of a network.

## Why this exists

- **Native, not a web view.** No Electron, no bundled Chromium. Layout is Taffy/GPUI, text shaping is real, and a 50×120 cell repaint pass costs about 1.7 ms in a debug build.
- **Terminal and files in one pane lifecycle.** SFTP tracks the shell's live working directory, and each tab owns its own remote session state.
- **Credentials stay on the machine.** Passwords and key passphrases live in the OS keyring; session files are written atomically with `0600` permissions and never logged in plaintext.
- **CJK-first typography.** Latin and CJK glyphs render from two independently configurable fonts, with CJK advance width reverse-solved to exactly two cells, so a wide-character table never drifts out of alignment when you change fonts.

## Features

### Sessions and protocols
- Session tree with nested folders, drag-to-organize, live filtering and batch attribute edits.
- Connection types: SSH2 (password, keyboard-interactive, public key, OpenSSH CA user certificates, `ssh-agent` on Unix), local PTY shell, and serial console for embedded boards and switch management ports.
- Modern crypto only: AES-128/256-GCM and ChaCha20-Poly1305 ciphers, SHA2-512 MACs, `zlib` stream compression, RSA / ECDSA / Ed25519 keys.
- Keepalive probes every 30 s with 3-strike disconnect, so cloud firewalls cannot silently drop an idle session.
- Sessions restored and reconnected on launch; catalog exported/imported as plain JSON/YAML or as an Argon2id + XChaCha20-Poly1305 encrypted vault.

### Terminal
- Nested split views (split direction belongs to each cut), draggable dividers, per-pane zoom, and pane detach into a tab.
- Buffer search with a regex toggle plus an overview tick band that shows hits across the entire scrollback and jumps on click.
- Always-on highlighting for `ERROR` / `WARN` / `INFO` / `DEBUG`, IPv4 addresses and UUIDs, and up to 32 custom keywords, all mapped to theme palette slots so they follow your colorscheme.
- Rectangular column select with <kbd>Alt</kbd>+drag that still works inside `vim` and `htop`, configurable double-click word separators, and OSC 133 / OSC 777 desktop notifications when a command crosses a runtime threshold.
- Snippets bar for repeating a saved one-liner across the focused pane.

### Port forwarding
- Local, remote and dynamic (SOCKS5) tunnels started and stopped against a live session, with a panel that lists rules, connection counts and channel state.

### SFTP
- Dual-pane local/remote browser with resume, a transfer queue supporting priority insert, retry-all and clear-completed, and per-task rate and percentage.
- Directory diff and one-way selective sync based on size and mtime, with mtime preserved after transfer so nothing re-copies.
- Optional streamed SHA-256 verification of both endpoints (64 KiB chunks) after transfer, and wildcard exclusion rules that prune `.git/` and `node_modules/` as directories.
- Built-in remote editor with read-only protection, unified-diff preview before saving, and backup-on-write; `chmod` as a 9-cell grid or octal literal; hover previews for images and the first lines of text files; `Ctrl+F` filename filter.

### Interface
- Six built-in themes (One Dark, Dracula, Monokai Pro, Nord, Solarized Dark, Gruvbox Dark), client-side window controls with drag and double-click maximize, remembered window geometry, and full-screen immersive mode.
- Bilingual UI (English / 简体中文) switchable in settings — this README mirrors the app.
- Every shortcut is remappable with conflict detection. `$app` below means <kbd>Cmd</kbd> on macOS and <kbd>Ctrl</kbd>+<kbd>Shift</kbd> elsewhere.

| Action | Default | Action | Default |
| :--- | :--- | :--- | :--- |
| New session | `$app-n` | Split right / down | `$app-d` / `$app-e` |
| New local terminal | `$app-t` | Close pane / zoom pane | `$app-x` / `$app-enter` |
| Quick open | `$app-k` | Focus pane | `$app-alt-←↑→↓` |
| Close tab | `$app-w` | Toggle sidebar / SFTP / forwards | `$app-b` / `$app-s` / `$app-p` |
| Next / previous tab | `ctrl-tab` / `ctrl-shift-tab` | Snippets bar | `$app-j` |
| Reconnect | `$app-r` | Terminal search | `$app-f` |
| Immersive mode | `f11` / `alt-enter` | | |

## Platform support

| Platform | Status |
| :--- | :--- |
| Linux (Wayland / X11) | Primary development and test target. |
| macOS, Windows | Cross-platform code paths exist (modifier keys, keyring backends, reveal-in-file-manager) and compile, but they are **not yet verified by CI or a release artifact**. Treat them as experimental. |

`ssh-agent` authentication is Unix-only today; terminal hyperlinks open via `xdg-open`.

## Build from source

Requirements: Rust **1.92 or newer** (edition 2024) and a desktop session for the GPUI platform layer.

```bash
git clone https://github.com/mangerle/transom.git
cd transom

cargo run --bin transom            # development build
cargo build --release --bin transom  # artifact at target/release/transom
```

Release builds strip symbols and use thin LTO. Logging goes through `tracing` with an `env-filter`, so `RUST_LOG=debug ./target/release/transom` raises verbosity; the default level is `info`.

### Where your data lives

Everything is under the user config directory (`~/.config/transom` on Linux, `~/Library/Application Support/transom` on macOS, the known folder on Windows):

| File | Contents |
| :--- | :--- |
| `sessions.json` | Session tree and connection settings. |
| `settings.json` | Preferences, theme, fonts, shortcuts, snippets. |
| `known_hosts.json` | Host key fingerprints (trust-on-first-use). |
| OS keyring, service `transom` | Passwords and key passphrases. Never on disk. |

Generated key pairs default to `~/.ssh`.

## Project layout

```
apps/transom            binary: wires stores and the async connection controller together
crates/transom-core     storage traits, session/settings models, forward and highlight rules
crates/transom-net      russh client, SFTP, port forwarding, serial ports, PTY plumbing
crates/transom-store    JSON stores, keyring, Argon2id/XChaCha20 backup vault, atomic writes
crates/transom-terminal alacritty_terminal grid, renderer, search, OSC parsing
crates/transom-ui       GPUI app, sidebar, tabs, splits, SFTP panes, settings, i18n
```

Roughly 47k lines of Rust with 260+ unit and integration tests.

## Development

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings   # zero-warning policy
cargo check --all-targets
cargo test
```

`AGENTS.md` at the repository root records the engineering conventions this codebase is held to: no unbounded channels, no `unwrap()` on production paths, no blocking calls inside the async runtime, bounded queues with explicit backpressure, and streaming I/O for large transfers.

## Roadmap

[`docs/FEATURES_ROADMAP.md`](./docs/FEATURES_ROADMAP.md) (Chinese) tracks all 116 evaluated capabilities with per-item completion status. Currently in flight: ProxyJump bastion chaining, broadcast input across panes, a compose bar, and per-environment session colors. Already tracked as gaps: SOCKS5 accepts unauthenticated proxies only, and no signed installers yet.

## Contributing

Issues and pull requests are welcome. Open an issue before large changes so the scope can be agreed first. Please keep commits in Conventional Commits form; Chinese or English descriptions are both fine.

## License

Licensed under the **Apache License, Version 2.0**. See [`LICENSE`](./LICENSE). Third-party dependencies remain under their own licenses; notable ones: GPUI (MIT/Apache-2.0), russh (MIT), russh-sftp (MIT), alacritty_terminal (Apache-2.0).
