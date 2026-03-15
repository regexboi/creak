# creak

Fast cursor-level voice dictation for Wayland/Hyprland.

`creak` is a small Rust CLI meant to be bound to one key:

1. First press starts recording through `ffmpeg`.
2. Second press stops recording, sends the audio to Groq Whisper, and pastes the transcript at the focused cursor.

## Requirements

- `ffmpeg`
- `wl-copy`
- `hyprctl` for automatic paste on Hyprland
- `notify-send` for wrapper feedback
- `GROQ_API_KEY` in the environment or `.env`

## Why `wav`

The baseline records mono 16 kHz PCM `wav` instead of `m4a`. That avoids encode time at stop and keeps Groq-side preprocessing simple. For short dictation, the upload size is still small enough to stay practical.

## Install

```bash
cargo build --release
install -Dm755 target/release/creak ~/.local/bin/creak
```

You can keep `GROQ_API_KEY` in a repo-local `.env`. This repo ignores `.env` by default.

## Hyprland Binding

The most reliable setup here was a tiny wrapper script plus a normal modifier+key bind, not a modifier-only chord.

Example wrapper:

```sh
#!/bin/sh
set -eu

CREAK_BIN="$HOME/.local/bin/creak"
CREAK_DIR="$HOME/path/to/creak"
CREAK_ENV_FILE="$CREAK_DIR/.env"
LOG_DIR="${XDG_RUNTIME_DIR:-/tmp}/creak"

mkdir -p "$LOG_DIR"
cd "$CREAK_DIR"
CREAK_DOTENV="$CREAK_ENV_FILE" "$CREAK_BIN"
```

Example Hyprland bind:

```ini
bindd = Super_R, COMMA, Creak dictation, exec, ~/.local/bin/creak-toggle
```

On some systems the physical `Right Alt` key is exposed to Hyprland as `Super_R`. Verify your actual keysyms before assuming `Alt_R`.

## Usage

```bash
creak
creak
```

Optional commands:

```bash
creak start
creak stop
creak status
```

Optional environment variables:

- `CREAK_SOURCE`: override the PulseAudio / PipeWire source name if `pactl get-default-source` is not what you want.
- `CREAK_PASTE_SHORTCUT`: override the Hyprland `sendshortcut` string used for paste, for example `SHIFT, Insert, activewindow`.

## Notes

- `creak` records mono 16 kHz PCM `wav` to avoid stop-time encode overhead.
- Paste currently tries `Shift+Insert` first, then falls back to `Ctrl+V`.
- If a keybind seems flaky, add a wrapper-level debounce instead of trying to solve it inside the transcription binary.
