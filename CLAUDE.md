# Aaru: notes for the assistant

This file is read at the start of a session in this repository. It says what the project is and how to work on it. Read it, then look at the code; do not trust it over the code.

## What this is

**Aaru** (internally the crate `aaru`, file `aaru.exe`; it was called **Irminsul** until v0.5.0) is a Windows program for Genshin Impact. It watches the game's network traffic (packet capture; no memory reading, no injection, no game files touched) and turns it into: a Genshin Optimizer (GOOD) export, tabs for characters/materials/planner/achievements, an interactive map of the world with markers (what is collected, chests opened, enemies killed), and overlays over the game (map, a list of what is near the player, markers drawn over the game's own big map).

It is a fork of `konkers/irminsul` (MIT). The fork lives at `https://github.com/Neiroromo/Aaru` (public; the old name redirects). Releases are made by hand with `gh release create` and one asset, `aaru.exe`.

The owner speaks Russian: answer in Russian, keep code, comments and commit messages in English, and give UI text as an English/Russian pair (`tr("English", "Русский")` in the UI code). They do not like surprises: do not commit, push, release, delete or rename without being asked.

## Build and test

- Windows only for real use. `rust-toolchain.toml` pins `nightly` (edition 2024, let-chains). Needs the MSVC build tools; `cargo build --release` takes about 3 minutes (thin LTO, one codegen unit) and **`build.rs` downloads game data from gitlab.com**, so the first build for a profile needs the network (it failed once when gitlab did not answer: retry).
- `cargo test --release` (about 190 tests, all offline; two tests of the vendored crate `vendor/auto-artifactarium` about the player id fail and are old and unrelated: run them with `cd vendor/auto-artifactarium && cargo test`). Tests marked `#[ignore]` are tools that need real files or a running game: their doc comments say which environment variables to set.
- `cargo build --profile fast` is a quicker, less optimized build for trying things (it has to run `build.rs` too).
- The build puts the exe in `target/release/aaru.exe`, **which cannot be replaced while the program runs** (the error is "failed to remove file ... os error 5"): the exe it just built is then in `target/release/build/aaru/<hash>/out/aaru.exe`.
- The program needs administrator rights (packet capture with pktmon). Run it with `--no-admin` for looking at the UI, and from a non-elevated shell you cannot click it or read its threads' names reliably. To look at it without touching real data set `APPDATA` to a scratch folder before starting it (the data folder is `%APPDATA%\Aaru\data`). Other debug variables: `IRMINSUL_START_TAB` (achievements, characters, materials, planner, chests, map, events), `IRMINSUL_PROFILES_DIR`, `IRMINSUL_MAP_DIR`, `IRMINSUL_MARKERS_DIR`, `IRMINSUL_MAP_OVERLAY`, `IRMINSUL_FAKE_PLAYER`, `IRMINSUL_LIVE_DATA`, `IRMINSUL_NO_LIVE_DATA` (the prefix is old on purpose).
- Threads are named (`crate::spawn_named`); the window draws are counted by `src/frame_stats.rs` (Power Tools shows them): look there first when the program uses much processor.
- The shell tool of the assistant mangles long here-documents with quotes: write files with the file tools and run small scripts from a file.

## Layout

- `src/main.rs` (starts everything, `APP_ID = "Aaru"`), `src/app.rs` (the window, tabs, Power Tools), `src/monitor.rs` (the capture loop: packets to the sniffer, commands to the matchers, results to the UI through `AppState`), `src/capture/` (pktmon backend, pcap optional).
- `vendor/auto-artifactarium` (vendored fork of the packet decoder): KCP reassembly, XOR key search (the key comes from the server's seed in the token response and the client's clock; search is on all cores), protobuf matching by content (`unk_util.rs`: items, avatars, achievements, gadgets, monsters, quests, positions). After a game patch only command ids and a few field numbers may need updating (see below).
- Data tabs: `roster_tab.rs`, `planner_tab.rs`, `achievement_tab.rs`, `upgrade_data.rs`/`upgrade_store.rs` (static game data `src/data/upgrade_data.json.gz`, made by `cargo run --release --example gen_upgrade_data`), `live_data.rs` (upcoming characters from the lunaris.moe API), `profiles.rs` (one file per account in `profiles/`), `events.rs`/`events_tab.rs` (official announcements).
- Map: `map_tab.rs` (the tab), `markers.rs` (labels/points of the official HoYoLAB interactive map, `where_is`, `routable`), `atlas.rs`, `map_tiles.rs`, `marker_icons.rs`, `official_chests.rs`, `chests.rs`/`chest_watch.rs`/`object_watch.rs`/`puzzle_watch.rs`/`quest_watch.rs`/`coverage.rs` (what the traffic says is opened/done), `mob_watch.rs` (live enemies and which markers are defeated), `heights.rs`, `scenes.rs`, `levels.rs`/`minimap.rs` (floors), `route.rs`/`route_ui.rs` (farming route planner).
- Over the game: `map_overlay.rs` (key = Caps Lock by default: tap shows the map, hold edits it; the list of what is near; the game-map mode), `overlay_window.rs` and `game_map.rs` (layered click-through windows), `game_map_panel.rs` (drawn pictures), `map_render.rs` (CPU rasterizer), `map_sync.rs` (finds which part of the map the game shows in its big map by matching a screenshot with the tiles), `camera_dir.rs` (camera direction from the wedge on the minimap), `window_capture.rs` (Windows Graphics Capture of the game window, with GDI as the fallback), `hotkey.rs`, `overlay.rs` (floating window of materials).
- Housekeeping: `migrate.rs` (copies the old `%APPDATA%\Irminsul` to `%APPDATA%\Aaru`), `autostart.rs` (scheduled task `Aaru`), `session_log.rs` (background log, kept only when the user marks it), `housekeeping.rs` (old logs removed, built-in data preloaded), `frame_stats.rs`.
- `scripts/*.py` generate `src/data/material_sources.json` and `monster_labels.json` from Project Amber (`gi.yatta.moe`) and the Fandom wiki; run them from the repo root after a patch (`gen_material_sources.py` first, then `gen_artifact_domains.py`, `gen_story_domains.py`, `gen_monster_labels.py`; `gen_story_domains.py` reads and rewrites the same JSON). `examples/` has the generators of the map data (`gen_map_markers`) and chests.
- `docs/tracking-logic.md` and `docs/trackable-objects.md` are older notes on what can be tracked.

## Facts about the game's traffic that took work to find

- Entity ids: avatar `0x08xxxxxx`, monster `0x10xxxxxx`, gadget `0x20xxxxxx`. Monster ids are Project Amber's ids; the last two digits are variants. The scene entity message carries gadgets (chests, plants, oculi) and monsters with positions (game x, height, z); one that is dead or collected is **not sent**, which is how "opened"/"killed" is inferred (with a radius and a few reports in a row). A killed monster's drops come as gadgets whose group id is the monster's entity id.
- Map coordinates: `map_x = -z - 750`, `map_y = -x + 2321` for the world (scene 3); other scenes have their own offset and scale (`scenes.rs`, learned or pinned). North is up.
- The game does not send the camera direction: it is read from the minimap (a lighter wedge a quarter of the circle wide, north up). The map markers have no height: heights are learned from what the game sends.
- Session key: the server's token response (command 27808) is sometimes sent **compressed in a way not known** (about 20 KB declared as ~25.7 KB); then nothing of that game session can be read and the game has to be restarted. A sample of it is kept in `debug/packed-login-*.bin` (contains the account token: never print it, delete after looking). The key is kept in `session_key.bin` so the program can be restarted without restarting the game.
- Per patch: command ids and field numbers for items/avatars in the vendored decoder; `keys/gi.json` has not changed since 2025. See `vendor/auto-artifactarium/src/lib.rs` and `protos/protos.proto`.

## State and what is unverified

- Released: v0.5.0 (the renaming, the near list, camera arrow, Graphics Capture, speed-ups). Newer work in the working tree may not be pushed: look at `git status`.
- The camera arrow in the near list and the drawing of markers over the game's big map were written against a few screenshots and a short live test; they may need tuning on a running game. The big map's dark underground view is not recognized.
- A known puzzle (open when this was written): a running copy of the program used a whole core in the thread that draws the window; the cause is not found, `frame_stats.rs` was added to name it (look for "the window is drawn N times a second" in the log) and it holds an unused window to 20 draws a second.
- Ideas not done: finish a better reading of the compressed login response; tile cache and overlay renderer are heavy on the processor; DXGI duplication is not needed now that Graphics Capture is in; routes use straight lines (no path finding); weekly-boss domains for a few materials are not on the map.

## Data of the user (not in this repository)

`%APPDATA%\Aaru\data` has the profiles, what is collected on the map, planner goals, settings, calibrations and caches (tiles, icons). The collected marks and the goals cannot be got back from the game: copy that folder to a new computer (the `map_tiles`, `icons`, `map_floors`, `event_covers` and `log` folders can be left out; they are made again). The exports of the program are `genshin_export_*.json` files wherever the user saved them.
