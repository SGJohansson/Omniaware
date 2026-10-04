# Omniaware

A lightweight, keyboard-first home for everything I don't want to lose: clipboard snippets, quick notes, links, screenshots and journal entries — captured in a keystroke and laid out on a timeline.

I built Omniaware to replace a patchwork of AutoHotkey pop-ups, a separate Markdown editor and browser-based note tools. The goals are simple: it should be there the instant I need it, stay out of the way when I don't, and never lose a single character — not even on a power cut.

> **Status:** early development (v0.2). Windows 10/11 only. The interface is currently in Swedish.

## Features

- **Silent capture** — press `Ctrl+C` twice in quick succession and the clipboard (text or image) is saved straight to today's journal. The tray icon briefly turns green; nothing else interrupts you.
- **Capture popup** — `Win+O` opens a small window pre-filled with the clipboard. Type, paste images, give it a name, press `Esc`. Done.
- **Timeline and calendar** — `Ctrl+Alt+O` opens the main window: a day-by-day timeline, a month calendar marking days with content, named entries and a recycle bin.
- **Full-text search** — `Ctrl+K` searches everything. `Shift+Enter` pastes the result straight into the window you came from.
- **Markdown** — entries are plain Markdown with a rendered preview (`Ctrl+E`), including images and task lists.
- **Named snippets** — give an entry a name (`Ctrl+S`) and it becomes a reusable snippet, much like an AutoHotkey text store.
- **Crash-safe by design** — see [Data and durability](#data-and-durability).

## Keyboard shortcuts

**Global**

| Shortcut | Action |
|---|---|
| `Ctrl+C` `C` | Save the clipboard silently (hold `Ctrl`, tap `C` twice) |
| `Win+O` / tray click | Capture popup, pre-filled with the clipboard |
| `Ctrl+Alt+O` (or `AltGr+O`) | Open / close the main window |

**Capture popup**

| Shortcut | Action |
|---|---|
| `Esc` | Save and close (empty entries are discarded) |
| `Shift+Esc` | Move to the recycle bin |
| `Ctrl+S` | Name the entry. If the name is taken, `Enter` again moves the name here; the previous entry is kept, unnamed |
| `Ctrl+V` | Paste text or an image |

**Main window**

| Shortcut | Action |
|---|---|
| `←` `→` / `T` | Previous / next day / today |
| `Ctrl+K` | Search. `Enter` opens, `Shift+Enter` pastes into the previous window |
| `Ctrl+N` | New entry |
| `Ctrl+E` | Toggle edit / preview |
| `Esc` | Back, then close |

All global shortcuts can be changed in `config.toml`.

## Installation

### Download

Pre-built binaries are published on the [Releases](https://github.com/SGJohansson/Omniaware/releases) page. Download `omniaware.exe` and run it — no installer, no dependencies. To start it with Windows, place a shortcut in `shell:startup`.

### Build from source

Requires the Rust toolchain and the MSVC build tools:

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

git clone https://github.com/SGJohansson/Omniaware.git
cd Omniaware
cargo build --release
.\target\release\omniaware.exe
```

## Data and durability

Everything lives locally in `%APPDATA%\Omniaware\` (override with the `OMNIAWARE_DATA` environment variable). Nothing is sent anywhere.

| File | Contents |
|---|---|
| `omniaware.db` | All entries, revision history and the search index (SQLite) |
| `blobs\` | Images, stored content-addressed by their BLAKE3 hash |
| `config.toml` | Settings; created with defaults on first start |
| `omniaware.log` | Errors, if any |

How it avoids losing data:

- The clipboard is written to the database **before** the capture window is even shown.
- Edits are saved 300 ms after the last keystroke, in SQLite WAL mode with `synchronous=FULL` (every commit is flushed to disk).
- A revision snapshot is kept each time an entry is closed.
- Images are written to a temporary file, flushed and then atomically renamed.
- Deleting moves entries to a recycle bin; permanent removal requires a second confirmation.

## Configuration

```toml
renderer = "wgpu"            # "glow" switches to OpenGL if the GPU driver misbehaves

[hotkeys]
capture = "super+KeyO"       # modifiers: ctrl / alt / shift / super
main = "ctrl+alt+KeyO"

[window]
width = 640.0                # capture popup
height = 420.0
font_size = 14.0
main_width = 1040.0
main_height = 700.0
```

## Roadmap

- Events with natural-language dates in Swedish (`imorgon 14 tandläkare`), a full calendar view and reminders
- Desktop post-it notes
- An aggregated to-do view built from Markdown task lists
- Optional encrypted backup to a self-hosted server

## Built with

[Rust](https://www.rust-lang.org/), [egui](https://github.com/emilk/egui), [SQLite](https://sqlite.org/) via rusqlite, and the [JetBrains Mono](https://www.jetbrains.com/lp/mono/) typeface.

## License

Omniaware is released under the [MIT License](LICENSE). JetBrains Mono is bundled under the [SIL Open Font License 1.1](assets/fonts/LICENSE-JetBrainsMono.txt).
