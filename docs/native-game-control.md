---
type: guide
description: Drive isolated development sessions through native game input handlers and verify UI, state and SDL window outcomes.
---

# Native game control

`scripts/game-control.py` drives an explicitly enabled development session through
KeeperFX's existing loopback API. It uses native game event injection: keyboard and
pointer events pass through the SDL event dispatcher and the normal game input
handlers. This proves game input handling, not physical keyboard/mouse delivery or
OS accessibility permissions.

Every control-session launch is **agent mode**: the session ignores physical
keyboard and mouse events, never grabs or warps the host cursor, and its window
does not take keyboard focus from the desktop, so an agent can drive the game
while you keep working in another app. `state` reports the window's real OS focus
in `focused` and SDL's cursor capture in `grabbed`; in agent mode both stay false.
`focus` raises the window without activating it. The game cursor starts at the
window centre rather than following the host pointer.

```sh
python3 scripts/game-control.py launch --out out/control-example --backend wgpu --verify
python3 scripts/game-control.py launch --out out/agent-session --level 1
python3 scripts/game-control.py snapshot --session out/control-example/session.json
python3 scripts/game-control.py click 320 345 --until frontend=27 --session out/control-example/session.json
python3 scripts/game-control.py key Escape --until frontend=1 --session out/control-example/session.json
python3 scripts/game-control.py quit --session out/control-example/session.json
```

The launcher clones game assets and creates fresh settings, saves and screenshots.
`--game-dir` selects the source assets; `--copy-saves` explicitly copies its saves.
It never writes to the source directory. The default lifetime is 1,200 seconds;
a supervisor terminates the launched child at the limit. On macOS, launching from
a restricted shell requires desktop access for SDL; no input-synthesis permission
is needed. The default presenter is SDL; `--backend wgpu --verify` selects Rust and
its live GPU comparison. Drawing defaults to software; `--draw-backend wgpu` selects the
GPU drawing path and `--draw-verify` adds its CPU oracle comparison, which is a parity
check rather than a performance run. `--level 1 --campaign keeporig` starts gameplay directly.

Further launch options exist to reach drawing paths a default session never enters:
`--cheats` passes `-alex`, which the `lua` and `dbc` console commands require; `--play-movies`
keeps the startup movies and forces the short Bullfrog logo movie; `--smoothing` passes
`-vidsmooth`; `--ingame-res` replaces the in-game video mode list (the parchment fade state is only
entered below 321 pixels wide); `--language` writes a three-letter language code, and a
double-byte language switches all text to the Asian glyph path; `--rotate-mode 2` writes the
isolated `save/settings.toml` so the level starts in front view.

The descriptor is private (0600) inside the new session directory (0700). Keep it
private: its random token authorizes all API actions in that process, including
legacy Lua/console actions. The engine opts in only with valid
`KFX_DEV_CONTROL_TOKEN` (64 lowercase hex characters) and
`KFX_DEV_CONTROL_SESSION` (32 lowercase hex characters), and the usual API setting
must also be enabled. The token is not returned or logged. The separate session
identifier prevents an accidental connection to another game. Normal launches do
not enable control or change their desktop input behavior.

Use this CLI as the repeatable control path for graphics checks before creating
one-off OS input helpers or accessibility automation. The final-source validation
in [PR #9](https://github.com/zillakot/keeperfx/pull/9) exercised menu/load navigation,
window changes, pause and quit with 833 exact acquired-frame comparisons. See the
[Rust port plan](product/rust-port-plan.md#delivered-milestone-optional-live-rust-presentation)
for what that evidence establishes and the remaining coverage.

## Operations and outcomes

All commands print JSON. `state` returns frontend state, game view, turn, pause,
OS focus, cursor grab, actual window/pixel dimensions, mouse position, active
presenter and the control sequence counters. Frontend values include 1 (main
menu), 2 (load menu), 27 (options), and 0 during gameplay. `presenter` reports
the actual active SDL or wgpu instance, including a fallback.

- `move X Y`, `click X Y --button 1`, and `drag X Y TO_X TO_Y` use absolute window
  coordinates. The dispatcher applies the same window-to-game coordinate mapping
  as ordinary pointer events. Buttons are 1 left, 2 middle, 3 right.
- `key Escape`, `chord "Left Alt" R`, and `cycle-mode` use SDL key names. Mode
  cycling injects the standard Alt+R hotkey; custom game bindings still apply.
- `--frames N` holds input for 2–120 input-processing iterations, default 3.
  Movement is processed before pressing, and releases happen in later iterations.
  `wait --frames N` waits through processing iterations even when simulation is
  paused.
- `resize WIDTH HEIGHT`, `minimize`, `restore`, and `focus` call SDL's native
  window operations on the game's own window. Resize is limited to windowed mode,
  320–4096 by 200–2160. These operations exercise real window lifecycle events.
- `snapshot` schedules the existing renderer screenshot path and returns the PNG
  path. This is the game frame; live GPU comparison requires `--verify` separately.
- `script "COMMAND"` and `console "COMMAND"` send a level-script line or a console
  command through the game API instead of the input queue, which is how a check
  possesses a creature or activates a Lua lens. Both are refused while the game is
  paused, and `lua`, `dbc` and the other cheat commands need `--cheats`.
- `cancel` releases held inputs; `quit` dispatches the standard quit event and
  waits for the supervised process to exit normally.

A completed sequence means its inputs have been dispatched and released. It does
not alone prove a UI action succeeded. Use repeated `--until FIELD=VALUE` predicates
for actual results; values use JSON syntax, except `presenter=sdl`/`presenter=wgpu`.
Supported fields are frontend, view, width, height, fullscreen, minimized, focused,
grabbed, paused and presenter. A predicate timeout is an error. For the launcher's
configured game modes, starting at 640×480 windowed:

```sh
python3 scripts/game-control.py cycle-mode --until fullscreen=true --session out/control-example/session.json
python3 scripts/game-control.py cycle-mode --until fullscreen=false --until width=800 --session out/control-example/session.json
python3 scripts/game-control.py resize 937 613 --until width=937 --until height=613 --session out/control-example/session.json
```

After PR #41 the API server survives the video-mode switch, but `cycle-mode` and the
following operation both time out while the game keeps running in desktop mode — the
measurement schedule logged `{"error": "timed out"}` for each — so drawing-oracle sessions
exclude the mode round trip until that is fixed.

Only the isolated control session ignores physical keyboard/mouse events received
by its window and avoids grabbing/warping the host cursor. Native close, focus and
window lifecycle events remain active. Control requests run on the main thread;
there is one bounded command queue and one API client. Commands are never retried.
In control mode, a new connection replaces the previous client, even if its EOF
has not been read yet. Replacement clears subscriptions and partial requests and
cancels held input; do not run concurrent control clients. Socket writes suppress
SIGPIPE, and a failed write drops the client. This also covers reconnects after a
video-mode switch delays API polling. Window recreation does not restart the API
listener or change the port, token or session identifier; reuse the same descriptor.
The CLI may retry only the initial read-only identity handshake while the previous
connection closes. Disconnect, cancellation and a 15-second action deadline release
held keys/buttons. Idle connections expire after three seconds; each frame accepts
at most 4095 bytes, and newline-framed requests cannot exceed the 4096-byte buffer.
Acknowledgements must be 32-bit integers in control mode. The normal API protocol
remains unchanged when control is disabled.

## Verification

```sh
python3 -m unittest discover -s scripts/tests -p 'test_game_control.py'
python3 -m unittest discover -s scripts/tests -p 'test_drawing_coverage.py'
cmake -S tests/api -B out/api-tests
cmake --build out/api-tests
ctest --test-dir out/api-tests --output-on-failure
python3 scripts/game-control.py launch --out out/control-test --level 1
KFX_CONTROL_TEST_SESSION=out/control-test/session.json python3 -m unittest discover -s scripts/tests -p 'test_game_control_integration.py'
python3 scripts/game-control.py quit --session out/control-test/session.json
```

The opt-in integration tests execute the engine's authentication and framing,
input cancellation and subscription cleanup. They require unpaused gameplay and
move the isolated camera during the held-input cancellation check.

For the mode-switch reconnect regression, launch a fresh session with the rebuilt
engine, then issue separate commands without delays:

```sh
python3 scripts/game-control.py launch --out out/control-reconnect --level 1 --backend wgpu --draw-backend wgpu --verify
python3 scripts/game-control.py cycle-mode --until fullscreen=true --session out/control-reconnect/session.json
python3 scripts/game-control.py state --session out/control-reconnect/session.json
python3 scripts/game-control.py state --session out/control-reconnect/session.json
python3 scripts/game-control.py state --session out/control-reconnect/session.json
python3 scripts/game-control.py quit --session out/control-reconnect/session.json
```

Every command must succeed; `quit` must report `exit.returncode: 0` and
`exit.timed_out: false`. This regression currently fails at `cycle-mode` and the following
operation, which both time out after the switch. The C socket fixture covers
closed-peer writes with the default SIGPIPE disposition and replacement cleanup without
launching the game.

Screenshots plus state predicates establish UI outcomes. For rendering experiments,
record the executable hash and final Rust verification counts; run performance
comparisons separately with native control and the API disabled. Available displays
must actually have distinct backing scales to establish backing-scale transition
coverage.

## Drawing-family scene matrix

`scripts/drawing-coverage.py` runs one control session per drawing-family scene with
`KFX_DRAW_BACKEND=wgpu`, `KFX_WGPU_DRAW_VERIFY=1` and `KFX_WGPU_DRAW_STATS`, then writes the
table of which drawing family each scene reached. It exists because batch volume in a
dungeon session says nothing about families that session never enters.

```sh
python3 scripts/drawing-coverage.py --list
python3 scripts/drawing-coverage.py --engine out/macos/keeperfx --game-dir out/game
python3 scripts/drawing-coverage.py --scenes possession-lens --summarize-only
```

The run holds `/private/tmp/keeperfx-timing.lock` throughout, waits for the console to
unlock and for any other game to exit, and never starts two sessions at once. Each scene
keeps its isolated session directory under `out/drawing-coverage/<scene>/` with
`drawing.json`, the per-operation replies, screenshots and `scene.json`; `summary.json` and
`summary.md` hold the matrix. A scene directory that already has `scene.json` is skipped, so
an interrupted run resumes, and a failed scene does not stop the others.

Counters are not one per family. DBC glyphs and huge bitmaps share the bitmap arena kind,
the landview zoom shares the map-view kind with the parchment, smoothing shares
`transition_commands` with map fades, and a Lua lens has no counter of its own. Those rows
are reported as scene-attributed: the value only proves the family in the scene built to
reach it, and a non-zero value anywhere else is reported but does not count. Families with
no hook at all — general lines, screenshots, the unhooked frontend write, palette effects —
are reported as having no counter rather than as covered.
