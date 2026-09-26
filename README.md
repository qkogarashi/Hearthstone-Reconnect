# HS Reconnect

A small always-on-top overlay for Windows that forces **Hearthstone** to reconnect to the game server in one click.

It finds the TCP connections owned by `Hearthstone.exe`, lets you drop the match-server connection, and the game immediately reconnects — the same thing that happens after a short network hiccup, but on demand. Handy for skipping long animations or recovering from a stuck game state.

## Requirements

- **Administrator rights.** Windows only lets elevated processes reset TCP connections.

## Usage

1. Launch `HSReconnect.exe` and accept the UAC prompt.
2. Start a match in Hearthstone. The game-server connection shows up highlighted in green.
3. Press **Reconnect**. Hearthstone drops and restores the connection within a couple of seconds.

| Control                  | Action                                                |
| ------------------------ | ----------------------------------------------------- |
| **Reconnect**            | Drop the match-server connection(s)                   |
| **kill**                 | Drop one specific connection                          |
| `show all` / `game only` | Toggle between all connections and match servers only |

## Building from source

Requirements:

- [Rust](https://rustup.rs) (stable, MSVC toolchain)
- Windows SDK: `rc.exe` is used to embed the icon, version info and manifest

```sh
git clone https://github.com/qkogarashi/Hearthstone-Reconnect.git
cd Hearthstone-Reconnect

cargo build --release    # -> target/release/HSReconnect.exe
cargo test
```
