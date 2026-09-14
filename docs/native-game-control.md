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

This is **agent mode**: the session ignores physical keyboard and mouse events, never
grabs or warps the host cursor, and its window does not take keyboard focus from the
desktop, so an agent can drive the game while you keep working in another app.
`state` reports the window's real OS focus in `focused` and SDL's cursor capture in
`grabbed`; in agent mode both stay false. `focus` raises the window without
activating it.

```sh
python3 scripts/game-control.py launch --out out/control-example --backend wgpu --verify
python3 scripts/game-control.py launch --out out/agent-session --level 1   # agent mode
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

Only the isolated control session ignores physical keyboard/mouse events received
by its window and avoids grabbing/warping the host cursor. Native close, focus and
window lifecycle events remain active. Control requests run on the main thread;
there is one bounded command queue and one API client. Commands are never retried.
The CLI may retry only the initial read-only identity handshake while the previous
connection closes. Disconnect, cancellation and a 15-second action deadline release
held keys/buttons. Idle connections expire after three seconds; each frame accepts
at most 4095 bytes, and newline-framed requests cannot exceed the 4096-byte buffer.
Acknowledgements must be 32-bit integers in control mode. The normal API protocol
remains unchanged when control is disabled.

## Verification

```sh
python3 -m unittest discover -s scripts/tests -p 'test_game_control.py'
python3 scripts/game-control.py launch --out out/control-test --level 1
KFX_CONTROL_TEST_SESSION=out/control-test/session.json python3 -m unittest discover -s scripts/tests -p 'test_game_control_integration.py'
python3 scripts/game-control.py quit --session out/control-test/session.json
```

The opt-in integration tests execute the engine's authentication and framing,
input cancellation and subscription cleanup. They require unpaused gameplay and
move the isolated camera during the held-input cancellation check.

Screenshots plus state predicates establish UI outcomes. For rendering experiments,
record the executable hash and final Rust verification counts; run performance
comparisons separately with native control and the API disabled. Available displays
must actually have distinct backing scales to establish backing-scale transition
coverage.
