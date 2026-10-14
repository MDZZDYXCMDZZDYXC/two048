# two048

A 2048 mini-game plugin for the [Pumpkin](https://pumpkinmc.org) Minecraft server, played entirely
from the chat bar through the `/2048` command tree.

Every command re-sends the current board, the score, and a row of clickable directional buttons,
so a full game plays out as a back-and-forth conversation in chat. No inventory, no GUI, no
external dependencies.

Built against the official Pumpkin plugin development guide
(`https://docs.pumpkinmc.org/plugin-dev/rust`) and compiled to a `wasm32-wasip2` component.

- Package: `two048` v0.1.0 (Rust edition 2024)
- Artifact: `target/wasm32-wasip2/release/two048.wasm`
- Dependencies: `pumpkin-plugin-api`, `tracing`

## Features

- Playable with zero client-side setup: everything renders as chat text components.
- Clickable control row - each button carries a `click_run_command` event, plus WASD aliases for
  players who prefer typing.
- Colour-coded tiles: each value maps to a Minecraft chat colour (bolted-on bold for values >= 128).
- Undo: up to 24 moves, rolling back the grid, score, and step counter together.
- Win and lose detection, with the win announced exactly once per run.
- Persistent server-wide leaderboard that survives restarts.
- Per-player statistics: best score, biggest tile, games played, average score, total steps.

All in-game text is translated server-side and picked per player from the client's own language
setting. The game logic and the command tree itself are language-independent. See
[Languages](#languages).

## Languages

The plugin is bilingual out of the box: **Simplified Chinese** (`zh_cn`) and **English** (`en_us`).
Text is translated **server-side**, which keeps the plugin's central promise that a player needs
zero client-side setup - no resource pack, no language files, nothing to install.

Selection is by prefix on the locale the client reports:

| Client locale | Language |
| --- | --- |
| `zh_cn`, `zh_tw`, `zh_hk`, anything starting with `zh` | Simplified Chinese |
| everything else, including unknown and empty values | English |

Traditional Chinese deliberately falls back to Simplified rather than English: for a `zh_tw` reader
Simplified is far more readable than English, and a third translation would not change the fact
that the plugin has to pick one.

There is no locale at all on the console, a command block, or RCON, so those fall back to English,
the conventional language for server-side output.

### How it is implemented

Text lives behind a `Strings` trait with one `impl` per language (`ZhCn`, `EnUs`), rather than in a
table of `&'static str` values. The reason is specific: `format!` requires its format string to be
a **literal**, so a runtime-loaded template cannot be used with it at all. Keeping every message as
a method means the interpolation stays a literal and the compiler checks placeholder count and
types - a mismatch becomes a build failure rather than a sentence with a hole in it. The side
benefits are that a translation can be proofread as one contiguous block, and that adding a
language means adding a single `impl`: forgetting one message is a compile error.

The language is resolved once per command by `sender_lang`, then threaded down as a `Lang` value
through every renderer (`board_message`, `ranking_message`, `me_message`, `help_message`, and the
button/status builders). That means exactly one host locale lookup per command, and no renderer can
accidentally fall back to a hard-coded language.

What is deliberately **not** translated:

- **Command names and arguments.** `start`, `up`, `undo`, `best` and the `w` / `a` / `s` / `d`
  aliases are identifiers, as is `/2048` itself. Translating them would silently change the command
  syntax whenever a player switches language.
- **Tile values and the board symbols.** The digits, the `-` empty-cell marker, and the arrow glyphs
  on the control buttons (`↑W`, `←A`, ...) are symbols rather than prose, and the grid's alignment
  depends on them being exactly these characters.
- **Server log lines.** `tracing::info!` / `tracing::warn!` output stays in Simplified Chinese; it is
  operator-facing and not part of the player experience.

Two strings are player-visible but still cannot be per-player, because both are registered a single
time in `on_load`, where there is no player context to read a locale from: the **command-tree
description** (shown in the client's command suggestions) and the **plugin metadata description**.
Both carry the two languages in one line separated by `·` instead, so neither audience is left out.
The permission node's description is operator-only and stays in Simplified Chinese.

## Requirements

- A Pumpkin server whose plugin API matches the pinned crate version
  (`pumpkin-plugin-api = "0.2.0+26.3-26.51"`).
- To build from source: a Rust toolchain with the `wasm32-wasip2` target installed.

```sh
rustup target add wasm32-wasip2
```

## Installation

```sh
cargo build --release
```

`.cargo/config.toml` already pins `wasm32-wasip2` as the default build target, so a plain
`cargo build --release` produces the plugin. The `[profile.release]` block enables LTO, which
shrinks the wasm binary considerably.

Then copy the artifact into your server's plugin directory and restart:

```sh
cp target/wasm32-wasip2/release/two048.wasm /path/to/server/plugins/
```

In game, run `/2048 start`. Your first board is pushed straight into chat.

## Commands

The root command `/2048` has the aliases `t2048` and `game2048`.

| Command | Aliases | Description |
| --- | --- | --- |
| `/2048` | - | Re-display the current board, or a prompt to start if none is running |
| `/2048 start` | `new`, `restart`, `reset` | Start a new game (records the abandoned one, if any) |
| `/2048 up` | `w` | Move up |
| `/2048 down` | `s` | Move down |
| `/2048 left` | `a` | Move left |
| `/2048 right` | `d` | Move right |
| `/2048 undo` | `back`, `z` | Undo the previous move |
| `/2048 show` | `board`, `b` | Re-display the board |
| `/2048 best` | `top`, `rank`, `leaderboard` | Server-wide leaderboard (top 10 plus your own rank) |
| `/2048 me` | `stats`, `profile` | Your own statistics and rank |
| `/2048 stop` | `quit`, `end`, `exit` | End the current game and record it |
| `/2048 help` | `h` | Command reference and rules |

Every subcommand literal is a separate node in the Brigadier-style command tree, which is why
clients get working tab-completion and argument highlighting for free. Game responses are sent
only to the player who invoked the command - nothing is broadcast to the rest of the server.

## Rules

Standard 2048:

- A 4x4 grid, seeded with two tiles on a new game.
- A spawned tile is `2` with 90% probability and `4` with 10%.
- Tiles of equal value merge when they collide, and a tile may merge at most once per move.
- A move that produces no change is rejected: the board is left untouched and no tile spawns.
- Reaching 2048 announces the milestone once, and play continues so you can push for higher tiles.
- The game ends when the grid is full and no adjacent pair can merge.

Scoring matches the original game: a merge awards the value of the merged tile.

## Leaderboard

Statistics live in the plugin's private data folder as `leaderboard.tsv`, so they survive server
restarts. The file is a tab-separated table with one row per player:

```
<name>  <best_score>  <best_tile>  <games>  <steps>  <score_sum>
```

Ties are broken in this order:

1. Best score (descending)
2. Biggest tile (descending)
3. Games played (descending)
4. Player name (lexicographic)

A run is recorded when the board locks up, when `/2048 stop` is used, or when a game in progress
is abandoned via `/2048 start`. Runs with zero moves are never recorded, and a `recorded` flag on
each game prevents the same run from being counted twice.

`/2048 best` shows the top 10. Your own row is highlighted in green and bold; if you rank below
the cut-off, your row is appended after an ellipsis so you always see where you stand.
`/2048 me` shows your rank as `#3 / 14 players` along with your full statistics.

The current rank is also appended to the board's status line (`排名 #12` in Simplified Chinese,
`Rank #12` in English) and refreshes on every move, so you can watch it climb without leaving the
game.

## Permissions

The command tree is registered against the permission node `two048:command.play` with
`PermissionDefault::Allow`, so every player can play out of the box. Server operators can revoke
it to lock the game down.

The plugin also requests two plugin-level permissions, used only for the leaderboard file:

| Permission | Why |
| --- | --- |
| `fs.read.data` | Read `leaderboard.tsv` on load |
| `fs.write.data` | Write `leaderboard.tsv` on game completion and unload |

No network permissions are requested, and no files outside the plugin's own data folder are
touched.

## Project layout

The whole plugin is a single file, organised into labelled sections:

| Section | Contents |
| --- | --- |
| Global state | `GAMES`, `PROFILES`, `DATA_FILE` statics and their accessors |
| Leaderboard | Sorting, ranking, accumulation, TSV encode/decode, `Profile` |
| RNG | A `splitmix64` generator seeded from host randomness and the wall clock |
| Game logic | `Direction`, `Game`, `slide`, `read_line`, `write_line` |
| Languages | `Lang`, the `Strings` trait, and the `ZhCn` / `EnUs` implementations |
| Rendering | `TextComponent` builders for tiles, the board, the control row, and the boards |
| Dispatch | The `Action` enum, the `CommandHandler` implementation, `sender_lang`, and `play` |
| Plugin entry | `PluginMetadata`, `on_load`, `on_unload`, `register_plugin!` |

## Implementation notes

- `Plugin::on_load` takes `&self` rather than `&mut self`, so all mutable state lives in
  `static Mutex<...>` values and is accessed through small guard-returning helpers.
- No host API calls happen while a lock is held. State is mutated under the lock, an owned
  snapshot is cloned out, the guard is dropped, and only then is the message rendered and sent.
  The RNG seed is likewise fetched before the game lock is taken, because seeding calls the host
  through `uuid::generate()`. The Pumpkin docs explicitly warn against holding a non-reentrant
  lock across a host call; this layout keeps that invariant easy to audit.
- There is no `rand` dependency. Randomness comes from a `splitmix64` generator whose seed mixes
  a host-provided random UUID with the wall clock, which avoids relying on a `getrandom` backend
  under `wasm32-wasip2`.
- The board is a single text component with newline children. Each cell is padded to a fixed
  six-character width, which stays aligned because Minecraft's default font renders digits and
  spaces at uniform width.
- Randomness is injected into `Game::new`, `spawn_tile`, and `shift` as a `&mut Rng` parameter
  rather than being created internally, which is what makes the "no host calls under a lock"
  rule hold.

## Testing

`cargo test` is not usable for this crate: `crate-type` is `cdylib` and `.cargo/config.toml` pins
the `wasm32-wasip2` target, so no test binary can run on the host.

The game and leaderboard logic was therefore verified with a throwaway native harness that
extracted the pure-logic sections from `src/lib.rs`, compiled them with the host `rustc`, and
exercised them:

- 8 explicit `slide` cases covering the merge edge cases (`[2,2,2,0]`, `[4,4,4,4]`, `[2,0,0,2]`,
  and so on).
- 1600 randomised cross-checks of `shift` (400 boards x 4 directions) against an independently
  written reference implementation that uses a different algorithm path - a physical tile-by-tile
  push plus transpose/reverse for the direction handling. For each case the harness asserts the
  moved board, the awarded score, that exactly one new tile of value 2 or 4 was added, and that
  `over == !can_move()`.
- Dead-board and live-board detection, undo rollback of grid/score/steps, tile spawning on a
  nearly-full and a full board, and new-game invariants.
- Leaderboard accumulation (a lower score must not overwrite the best score, while games, steps,
  and score sum still accumulate), the four-level tie-break ordering, `rank_of` for ranked and
  unranked players, a real on-disk save/load round trip, and tolerance of corrupt rows in the TSV
  file.

The harness is not committed to the repository. The plugin itself builds with zero warnings and a
clean `cargo clippy`.

The language layer needs no separate harness of its own: because every message is a trait method,
a missing or mis-typed translation in either `ZhCn` or `EnUs` is a compile error, and the whole
catalogue is exercised by the same `cargo build --release` / `cargo clippy` run described above.

## Troubleshooting

**`/2048` is not a known command.** The wasm file is not in the server's `plugins/` directory, or
its plugin API version does not match the server. Confirm that the pinned `pumpkin-plugin-api`
version in `Cargo.toml` lines up with your server release.

**The leaderboard resets after a restart.** The plugin needs the `fs.read.data` and
`fs.write.data` permissions to persist its data folder. Check the server log for a line starting
with `two048:`; a failed write is reported there as a warning.

**Cargo prints a warning about semver metadata.** `pumpkin-plugin-api = "0.2.0+26.3-26.51"`
carries build metadata after the `+`, which Cargo ignores for version resolution and warns about.
It is harmless. The version is pinned deliberately so that `cargo update` cannot silently move the
plugin onto an API track your server does not implement.

## Credits

Game and command structure follow the Pumpkin plugin development guide. Plugin API signatures were
verified against the `pumpkin-plugin-api` WIT definitions rather than the guide's snippets, since
several of the guide's examples had drifted from the current API.
