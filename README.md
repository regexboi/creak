# creak

Fast cursor-level voice dictation for Wayland/Hyprland.

`creak` is a small Rust CLI meant to be bound to one key:

1. First press starts recording through `ffmpeg`.
2. Second press stops recording, sends the audio to Groq Whisper, and pastes the transcript at the focused cursor.

## Requirements

- `ffmpeg`
- `wl-copy`
- `hyprctl` for automatic paste on Hyprland
- `GROQ_API_KEY` in the environment or `.env`

## Why `wav`

The baseline records mono 16 kHz PCM `wav` instead of `m4a`. That avoids encode time at stop and keeps Groq-side preprocessing simple. For short dictation, the upload size is still small enough to stay practical.

## Usage

```bash
cargo run --release
cargo run --release
```

Optional commands:

```bash
cargo run --release -- start
cargo run --release -- stop
cargo run --release -- status
```

Optional environment variables:

- `CREAK_SOURCE`: override the PulseAudio / PipeWire source name if `pactl get-default-source` is not what you want.
- `CREAK_PASTE_SHORTCUT`: override the Hyprland `sendshortcut` string used for paste, for example `SHIFT, Insert, activewindow`.
