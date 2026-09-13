# KeeperFX fork guidance

## Workflow

- This is `zillakot/keeperfx`. `origin` is the personal fork; `upstream` is `dkfans/keeperfx`.
- Work on a branch from current `origin/master`. Open PRs against the fork's `master`; never push directly to the protected default branch.
- The user authorizes creating PRs and merging once checks are green. Verify CI for the exact head and verify the final merge state. Follow any task-specific review instruction without adding a new human-approval gate.
- Issues are currently disabled and no project is attached. Use fork PRs as delivery records; check live tracker state before relying on that assumption.
- For substantial work, the parent orchestrates fresh, bounded subagents and reads their final reports and artifact summaries rather than full transcripts. Keep responsibilities separate and avoid concurrent benchmark workloads.
- Default to no comments and minimal docstrings sufficient for API documentation. Preserve only non-obvious constraints; keep touched documentation concise.

## Starting points

- [Documentation index](docs/README.md) and [project overview](docs/architecture/project-overview.md): current architecture and canonical guides.
- [Rust port plan](docs/product/rust-port-plan.md): delivered foundations, graphics next-session task and migration gates. Keep plans here rather than creating competing roadmaps.
- [Mac development](docs/macos.md): native build and assets. [Live Rust presentation](docs/live-rust-presentation.md): optional build, ownership and fallback.
- [Frame feedback](docs/frame-feedback.md), [performance baselines](docs/performance-baselines.md) and [native game control](docs/native-game-control.md): comparison, measurement and repeatable checks.
- [Audio modernization plan](docs/product/audio-modernization-plan.md): separate engine/audio-asset track.

## Graphics and validation

- Preserve original pixel art, palette behavior, nearest sampling and agreed exact comparisons. CPU world drawing remains in C/C++; optional Rust/wgpu presents its indexed framebuffer. SDL is the default and fallback.
- GPU world rendering needs scene/command input before rasterization. Do not describe framebuffer replay or Rust integration as GPU world drawing or a proven optimization.
- Distinguish simulation, CPU drawing, presentation, process CPU and GPU measurements. Report actual settings, source/binary identities, absolute costs and frame-time tails. Capped FPS, readback timings and scoped Rust allocations do not establish uncapped FPS or whole-process/GPU memory gains.
- Prefer `scripts/game-control.py` for repeatable isolated gameplay checks before one-off OS input helpers. State predicates plus screenshots verify outcomes; native in-process input is distinct from physical OS keyboard/mouse delivery.
- Use fresh isolated assets/settings/saves for tests. Preserve personal assets, saves and unrelated working-tree files. Keep original artwork, captures and private control session descriptors under ignored `out/`; do not publish secrets or private host details.
- Match validation to the change. Use existing tests and CI; add tests for meaningful behavioral risks. Documentation-only work needs content/link checks, not a new game build or benchmark. Keep live pixel/control checks separate from performance runs with control/API, audio and readback disabled.
