#!/usr/bin/env python3
"""Drive one control session per drawing-family scene and summarize the counters each scene reaches."""
import argparse
import fnmatch
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import time
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    loaded = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(loaded)
    return loaded


CONTROL = module("game_control", "game-control.py")
PROFILE = module("profile_game", "profile-game.py")

GATE_COUNTERS = ("failures", "invalid_frames", "frame_flagged_invalid", "verification_flagged_shades",
                 "rejected_commands", "rejected_spans", "rejected_triangles", "frame_rejected_checkpoints",
                 "missing_cpu_barriers")
REPORTED_COUNTERS = ("frames", "gpu_batches", "verified_batches", "verified_triangles", "cpu_barriers",
                     "target_alias_barriers", "bridge_solo_batches", "arena_overflows", "arena_evictions",
                     "shadow_prior_divergence")

# Every family row of the drawing coverage spec, section 1, with the drawing.json counters that
# witness it. "exclusive" means no other family in this table writes those counters, so a non-zero
# value identifies the family on its own; otherwise the scene that produced it carries the meaning.
FAMILIES = (
    dict(name="Dungeon terrain (gpoly)", counters=("gpu_triangles",), exclusive=True),
    dict(name="Legacy gpoly span sink", counters=("gpu_spans", "cpu_gpoly_spans", "cpu_replayed_spans"),
         exclusive=True, expect="zero", note="dead sink; a non-zero value is a regression, not coverage"),
    dict(name="Front view (display_fast_drawlist)", counters=("gpu_sprite_commands", "gpu_batches"),
         exclusive=False, note="shares the sprite and quad sinks; only the front-view scene attributes it"),
    dict(name="General triangles, modes 0-26", counters=("arena_trig_misses", "arena_trig_bytes"), exclusive=True),
    dict(name="Creature shadows", counters=("gpu_shadow_commands",), exclusive=True),
    dict(name="Shadow mode10 target triangles", counters=("arena_target_trig_geometry_misses",), exclusive=True),
    dict(name="World/HUD sprites", counters=("gpu_sprite_commands",), exclusive=True),
    dict(name="Ordered sprite runs", counters=("gpu_ordered_sprites",), exclusive=True),
    dict(name="Pixels, boxes, HV lines, circles", counters=("gpu_batches",), exclusive=False,
         note="no per-primitive counter exists; a per-kind command counter would be needed"),
    dict(name="General lines", counters=(), exclusive=False, expect="none",
         note="no kfx_wgpu hook on the general line path, so no counter can move"),
    dict(name="Text / GUI sprites", counters=("gpu_sprite_commands",), exclusive=False,
         note="drawn through the sprite wrappers; not separable from world sprites"),
    dict(name="DBC (Asian) glyph bitmaps", counters=("arena_bitmap_misses", "arena_bitmap_bytes"), exclusive=False,
         note="shares the bitmap arena kind with huge bitmaps; the DBC scene attributes it"),
    dict(name="Huge bitmaps", counters=("arena_bitmap_misses", "arena_bitmap_bytes"), exclusive=False,
         note="shares the bitmap arena kind with DBC glyphs; the landview scene attributes it"),
    dict(name="Raw / tiled images, backgrounds",
         counters=("arena_raw_image_misses", "arena_tiled_image_misses", "arena_image_misses"), exclusive=True),
    dict(name="Landview zoom", counters=("arena_map_view_misses",), exclusive=False,
         note="shares the map-view arena kind with the parchment; the landview scene attributes it"),
    dict(name="Parchment / overhead map", counters=("arena_map_view_misses",), exclusive=False,
         note="shares the map-view arena kind with the landview zoom"),
    dict(name="Minimap", counters=("arena_minimap_misses",), exclusive=True),
    dict(name="Movies", counters=("arena_movie_bytes", "arena_movie_misses"), exclusive=True),
    dict(name="Built-in lenses", counters=("arena_lens_bytes", "arena_lens_misses"), exclusive=True),
    dict(name="Lua lenses, Lua pixel/batch API", counters=("cpu_barriers",), exclusive=False,
         note="CPU path with no arena kind; a Lua-lens command counter would be needed to separate it"),
    dict(name="Possession lens offscreen target", counters=("target_alias_barriers",), exclusive=True),
    dict(name="Map fades / transitions",
         counters=("transition_checkpoint_bytes", "transition_snapshot_copy_bytes", "transition_commands"),
         exclusive=False, note="shares transition_commands with smoothing; the checkpoint bytes are fade-only"),
    dict(name="Smoothing", counters=("transition_commands",), exclusive=False,
         note="shares transition_commands with map fades; the smoothing scene attributes it"),
    dict(name="Cursor", counters=("arena_cursor_misses",), exclusive=True),
    dict(name="Full-surface clear", counters=("gpu_batches",), exclusive=False,
         note="no clear counter exists; every scene clears"),
    dict(name="Screenshots / recording", counters=(), exclusive=False, expect="none",
         note="reads the target with no hook; exercised by the snapshot operations"),
    dict(name="Direct unhooked screen write", counters=(), exclusive=False, expect="none",
         note="frontend.cpp font-test loop has no hook by definition"),
    dict(name="Palette effects", counters=(), exclusive=False, expect="none", note="no pixel loop"),
)

BUSY_LEVEL = 20
SCENES = (
    dict(name="dungeon-busy", launch=dict(level=BUSY_LEVEL), steps=(
        ("wait", dict(frames=120, until=["frontend=0", "view=1", "presenter=wgpu"])),
        ("snapshot", {}),
        ("key", dict(key="Up", frames=30)),
        ("key", dict(key="Right", frames=30)),
        ("key", dict(key="P", until=["paused=true"])),
        ("wait", dict(frames=20, until=["paused=true"])),
        ("key", dict(key="P", until=["paused=false"])),
        ("key", dict(key="M", until=["view=4"])),
        ("snapshot", {}),
        ("key", dict(key="M", until=["view=1"])),
        ("resize", dict(width=800, height=600, until=["width=800"])),
        ("resize", dict(width=640, height=480, until=["width=640"])),
        ("snapshot", {}),
    ), families=("Dungeon terrain (gpoly)", "Creature shadows", "Shadow mode10 target triangles",
                 "World/HUD sprites", "Ordered sprite runs", "Pixels, boxes, HV lines, circles",
                 "Text / GUI sprites", "Raw / tiled images, backgrounds", "Parchment / overhead map",
                 "Minimap", "Cursor", "Full-surface clear", "Legacy gpoly span sink",
                 "Screenshots / recording")),
    dict(name="possession-lens", launch=dict(level=BUSY_LEVEL, cheats=True), steps=(
        ("wait", dict(frames=120, until=["frontend=0", "view=1", "presenter=wgpu"])),
        ("script", dict(command="ADD_CREATURE_TO_LEVEL(PLAYER0,FLY,PLAYER0,1,1,0)")),
        ("wait", dict(frames=30)),
        ("script", dict(command="USE_POWER_ON_CREATURE(PLAYER0,FLY,ANYWHERE,PLAYER0,POWER_POSSESS,1,1)",
                        until=["view=2"])),
        ("wait", dict(frames=40, until=["view=2"])),
        ("snapshot", {}),
        ("key", dict(key="Up", frames=20)),
        ("snapshot", {}),
    ), families=("Built-in lenses", "Possession lens offscreen target")),
    # trig() is reachable in play only as the creature-shadow fallback when the GPU shadow hook
    # declines a sprite, so this scene crowds the camera with the largest shadow casters.
    dict(name="general-triangles", launch=dict(level=BUSY_LEVEL, cheats=True), steps=(
        ("wait", dict(frames=120, until=["frontend=0", "view=1", "presenter=wgpu"])),
        ("script", dict(command="ADD_CREATURE_TO_LEVEL(PLAYER0,HORNY,PLAYER0,2,10,0)")),
        ("script", dict(command="ADD_CREATURE_TO_LEVEL(PLAYER0,DRAGON,PLAYER0,3,10,0)")),
        ("script", dict(command="ADD_CREATURE_TO_LEVEL(PLAYER0,BILE_DEMON,PLAYER0,3,10,0)")),
        ("script", dict(command="ADD_CREATURE_TO_LEVEL(PLAYER0,TENTACLE,PLAYER0,2,10,0)")),
        ("wait", dict(frames=60)),
        ("snapshot", {}),
        ("key", dict(key="Up", frames=20)),
        ("wait", dict(frames=40)),
        ("snapshot", {}),
    ), families=("General triangles, modes 0-26",)),
    dict(name="lua-lens", launch=dict(level=BUSY_LEVEL, cheats=True), steps=(
        ("wait", dict(frames=120, until=["frontend=0", "view=1", "presenter=wgpu"])),
        ("script", dict(command="ADD_CREATURE_TO_LEVEL(PLAYER0,IMP,PLAYER0,1,1,0)")),
        ("wait", dict(frames=30)),
        ("console", dict(command='lua CreateLens("LENS_COVERAGE")')),
        ("console", dict(command='lua SetLensDrawCallback("LENS_COVERAGE", function(ctx) '
                                 'return CopyBuffer(ctx.srcbuf, ctx.dstbuf) end)')),
        ("script", dict(command="USE_POWER_ON_CREATURE(PLAYER0,IMP,ANYWHERE,PLAYER0,POWER_POSSESS,1,1)",
                        until=["view=2"])),
        ("console", dict(command='lua SetActiveLens("LENS_COVERAGE")')),
        ("wait", dict(frames=40, until=["view=2"])),
        ("snapshot", {}),
    ), families=("Lua lenses, Lua pixel/batch API",)),
    dict(name="front-view", launch=dict(level=BUSY_LEVEL, rotate_mode=2), steps=(
        ("wait", dict(frames=120, until=["frontend=0", "presenter=wgpu"])),
        ("snapshot", {}),
        ("key", dict(key="Up", frames=30)),
        ("key", dict(key="Right", frames=30)),
        ("snapshot", {}),
    ), families=("Front view (display_fast_drawlist)",)),
    dict(name="smoothing", launch=dict(level=BUSY_LEVEL, smoothing=True), steps=(
        ("wait", dict(frames=120, until=["frontend=0", "view=1", "presenter=wgpu"])),
        ("key", dict(key="Up", frames=30)),
        ("snapshot", {}),
    ), families=("Smoothing",)),
    dict(name="map-fade", launch=dict(level=BUSY_LEVEL, ingame_res="320x200w32"), steps=(
        ("wait", dict(frames=120, until=["frontend=0", "view=1", "presenter=wgpu"])),
        ("key", dict(key="M", until=["view=4"])),
        ("snapshot", {}),
        ("key", dict(key="M", until=["view=1"])),
        ("snapshot", {}),
    ), families=("Map fades / transitions",)),
    # Main menu 1, campaign selection 31, land view 3; the campaign list starts at y=167.
    dict(name="landview", launch=dict(), steps=(
        ("wait", dict(frames=60, until=["frontend=1", "presenter=wgpu"])),
        ("snapshot", {}),
        ("click", dict(x=320, y=115, until=["frontend=31"])),
        ("click", dict(x=300, y=178, until=["frontend=3"])),
        ("wait", dict(frames=60, until=["frontend=3"])),
        ("snapshot", {}),
    ), families=("Landview zoom", "Huge bitmaps")),
    # The startup movies run before the API server exists, so the launch wait covers their length.
    dict(name="movies", launch=dict(play_movies=True, startup_timeout=300), steps=(
        ("wait", dict(frames=30)),
        ("snapshot", {}),
        ("key", dict(key="Escape", frames=10)),
        ("key", dict(key="Escape", frames=10, until=["frontend=1"])),
        ("snapshot", {}),
    ), families=("Movies",)),
    dict(name="dbc-text", launch=dict(language="CHI"), steps=(
        ("wait", dict(frames=60, until=["frontend=1", "presenter=wgpu"])),
        ("snapshot", {}),
        ("click", dict(x=320, y=345, until=["frontend=27"])),
        ("snapshot", {}),
        ("key", dict(key="Escape", frames=10, until=["frontend=1"])),
    ), families=("DBC (Asian) glyph bitmaps",)),
)


def binary_identity(engine):
    data = engine.read_bytes()
    return dict(binary=str(engine), sha256=hashlib.sha256(data).hexdigest(), size_bytes=len(data))


def running_games():
    processes = subprocess.run(["ps", "-axo", "pid=,comm="], capture_output=True, text=True, check=True).stdout
    return [line.strip() for line in processes.splitlines()
            if len(line.split(maxsplit=1)) == 2 and Path(line.split(maxsplit=1)[1]).name.lower() == "keeperfx"]


def await_no_game(timeout=600, probe=None, sleep=time.sleep):
    """Two games at once would share the GPU and the host; wait for another session to finish."""
    probe = probe or running_games
    deadline = time.monotonic() + timeout
    while (running := probe()):
        if time.monotonic() >= deadline:
            raise RuntimeError("KeeperFX is already running: " + "; ".join(running))
        sleep(5)


def await_console(timeout=900, probe=PROFILE.console_locked, sleep=time.sleep):
    """A locked console cannot acquire a drawable, so a windowed session must wait rather than skip."""
    deadline = time.monotonic() + timeout
    while probe():
        if time.monotonic() >= deadline:
            raise RuntimeError("the console session stayed locked; a windowed game session cannot start")
        sleep(5)


def read_counters(work, attempts=100, sleep=time.sleep):
    for _ in range(attempts):
        try:
            return json.loads((work / "drawing.json").read_text())
        except (OSError, ValueError):
            sleep(0.05)
    raise RuntimeError("drawing counters were not readable")


def gate_of(counters):
    failed = {name: counters.get(name) for name in GATE_COUNTERS if counters.get(name)}
    reported = {name: counters.get(name, 0) for name in REPORTED_COUNTERS}
    return dict(passed=not failed and counters.get("verified_batches", 0) > 0, failed=failed, **reported)


def scene_value(counters, family):
    """First listed counter of the family with a non-zero value, else the first counter at zero."""
    for name in family["counters"]:
        if counters.get(name):
            return name, counters[name]
    return (family["counters"][0], counters.get(family["counters"][0], 0)) if family["counters"] else (None, None)


def summarize(scenes, results):
    order = [scene["name"] for scene in scenes]
    families = []
    for family in FAMILIES:
        row = dict(family=family["name"], counters=list(family["counters"]), exclusive=family["exclusive"],
                   expect=family.get("expect", "nonzero"), note=family.get("note", ""),
                   targeted_by=[scene["name"] for scene in scenes if family["name"] in scene["families"]],
                   scenes={})
        for name in order:
            counters = results[name].get("counters") or {}
            counter, value = scene_value(counters, family)
            row["scenes"][name] = dict(counter=counter, value=value)
        clean = [name for name in order if results[name].get("gate", {}).get("passed")]
        row["nonzero_in"] = [name for name in clean if row["scenes"][name]["value"]]
        # A shared counter only proves its family in the scene built to reach that family.
        row["measured_in"] = (row["nonzero_in"] if family["exclusive"]
                              else [name for name in row["nonzero_in"] if name in row["targeted_by"]])
        if row["expect"] == "none":
            row["status"] = "no counter"
        elif row["expect"] == "zero":
            row["status"] = "zero as expected" if not row["nonzero_in"] else "unexpectedly non-zero"
        elif row["measured_in"]:
            row["status"] = "measured" if family["exclusive"] else "measured (scene-attributed)"
        elif row["nonzero_in"]:
            row["status"] = "not reached (shared counter moved elsewhere)"
        else:
            row["status"] = "not reached"
        families.append(row)
    binaries = [results[name]["binary"] for name in order if results[name].get("binary")]
    return dict(generated_utc=datetime.now(timezone.utc).isoformat(),
                binary=binaries[0] if binaries else None,
                binaries_agree=len({entry["sha256"] for entry in binaries}) <= 1, scenes=[
        dict(scene=name, status=results[name]["status"], error=results[name].get("error"),
             gate=results[name].get("gate"), targets=list(scene["families"]))
        for name, scene in zip(order, scenes)], families=families)


def markdown(summary):
    order = [scene["scene"] for scene in summary["scenes"]]
    binary = summary.get("binary") or {}
    identity = f" Binary `{binary.get('sha256', 'unknown')}`." if binary else ""
    if not summary.get("binaries_agree", True):
        identity += " **Scenes did not all run the same binary.**"
    lines = ["# Drawing-family scene matrix", "",
             f"Generated {summary['generated_utc']}.{identity}", "",
             "## Scenes", "",
             "| Scene | Status | Frames | Verified batches | Failures | CPU barriers | Target-alias barriers | Gate |",
             "| --- | --- | --- | --- | --- | --- | --- | --- |"]
    for scene in summary["scenes"]:
        gate = scene["gate"] or {}
        failures = gate.get("failed") or {}
        lines.append(f"| {scene['scene']} | {scene['status']} | {gate.get('frames', '-')} | "
                     f"{gate.get('verified_batches', '-')} | {failures.get('failures', 0)} | "
                     f"{gate.get('cpu_barriers', '-')} | {gate.get('target_alias_barriers', '-')} | "
                     f"{'pass' if gate.get('passed') else 'fail'} |")
    lines += ["", "## Families", "",
              "| Family | Counter | " + " | ".join(order) + " | Status |",
              "| --- | --- | " + " | ".join("---" for _ in order) + " | --- |"]
    for row in summary["families"]:
        counter = row["counters"][0] if row["counters"] else "none"
        cells = []
        for name in order:
            cell = row["scenes"][name]
            value = "-" if cell["value"] is None else f"{cell['value']:,}"
            cells.append(f"**{value}**" if cell["value"] and name in row["targeted_by"] else value)
        lines.append(f"| {row['family']} | `{counter}` | " + " | ".join(cells) + f" | {row['status']} |")
    notes = [row for row in summary["families"] if row["note"]]
    if notes:
        lines += ["", "## Counter notes", ""] + [f"- **{row['family']}**: {row['note']}" for row in notes]
    return "\n".join(lines) + "\n"


def run_scene(scene, args, work):
    await_no_game()
    await_console()
    options = dict(out=work, game_dir=args.game_dir, engine=args.engine, backend="wgpu", verify=False,
                   draw_backend="wgpu", draw_verify=True, copy_saves=False, lifetime=args.lifetime,
                   campaign=args.campaign, level=None, cheats=False, play_movies=False, smoothing=False,
                   ingame_res=None, language=None, rotate_mode=None, startup_timeout=None)
    options.update(scene["launch"])
    launch = argparse.Namespace(**options)
    record = dict(scene=scene["name"], status="running", launch_args=None, operations=[])
    started = CONTROL.launch(launch)
    session = CONTROL.read_session(Path(started["session"]))
    record["launch_args"] = session["args"]
    record["engine_sha256"] = session["engine_sha256"]
    try:
        for index, (op, fields) in enumerate(scene["steps"]):
            client = CONTROL.Client(session, timeout=args.step_timeout)
            try:
                output = client.run(op, **dict(fields))
            finally:
                client.close()
            (work / f"op-{index:02}-{op}.json").write_text(json.dumps(output, indent=2) + "\n")
            record["operations"].append(dict(index=index, op=op, presenter=output["after"].get("presenter")))
            if output["after"].get("presenter") not in (None, "wgpu"):
                raise RuntimeError(f"presenter fell back to {output['after']['presenter']}")
    finally:
        record["exit"] = quit_session(session, args.step_timeout)
    return record


def quit_session(session, timeout):
    work = Path(session["work"])
    if not (work / "exit.json").exists():
        try:
            client = CONTROL.Client(session, timeout=timeout)
            try:
                client.run("quit")
            finally:
                client.close()
        except (OSError, RuntimeError, TimeoutError, ValueError):
            deadline = time.monotonic() + 620
            while not (work / "exit.json").exists():
                if time.monotonic() >= deadline:
                    raise RuntimeError("the session supervisor did not stop the game; inspect process.json")
                time.sleep(0.2)
    return json.loads((work / "exit.json").read_text())


def selected(patterns):
    chosen = [scene for scene in SCENES
              if any(fnmatch.fnmatchcase(scene["name"], pattern) for pattern in patterns.split(","))]
    if not chosen:
        raise ValueError(f"no scene matches {patterns}")
    return chosen


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=ROOT / "out/drawing-coverage")
    parser.add_argument("--engine", type=Path, default=ROOT / "out/macos/keeperfx")
    parser.add_argument("--game-dir", type=Path, default=ROOT / "out/game")
    parser.add_argument("--scenes", default="*", help="comma-separated scene name patterns")
    parser.add_argument("--campaign", default="keeporig")
    parser.add_argument("--lifetime", type=int, default=600)
    parser.add_argument("--step-timeout", type=int, default=240)
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--summarize-only", action="store_true", help="rebuild the summary from existing scene directories")
    args = parser.parse_args()
    scenes = selected(args.scenes)
    if args.list:
        for scene in scenes:
            print(f"{scene['name']}: " + ", ".join(scene["families"]))
        return
    args.out.mkdir(parents=True, exist_ok=True)
    results = {}
    if not args.summarize_only:
        with PROFILE.timing_lock() as lock:
            print(f"timing lock acquired after {lock['waited_seconds']:.1f}s", flush=True)
            for scene in scenes:
                work = args.out / scene["name"]
                if (work / "scene.json").exists():
                    print(f"skipping completed scene {scene['name']}", flush=True)
                    continue
                if work.exists():
                    work.rename(work.with_name(work.name + "-incomplete-" +
                                               datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")))
                print(f"scene {scene['name']}", flush=True)
                try:
                    record = run_scene(scene, args, work)
                    record["status"] = "complete"
                except (OSError, RuntimeError, TimeoutError, ValueError, subprocess.SubprocessError) as error:
                    record = dict(scene=scene["name"], status="failed", error=f"{type(error).__name__}: {error}")
                if work.is_dir():
                    record["counters"] = read_counters(work) if (work / "drawing.json").exists() else {}
                    record["gate"] = gate_of(record["counters"])
                    record["binary"] = binary_identity(args.engine.resolve())
                    (work / "scene.json").write_text(json.dumps(record, indent=2) + "\n")
                results[scene["name"]] = record
    for scene in scenes:
        path = args.out / scene["name"] / "scene.json"
        if scene["name"] not in results and path.exists():
            results[scene["name"]] = json.loads(path.read_text())
        results.setdefault(scene["name"], dict(scene=scene["name"], status="not run"))
    summary = summarize(scenes, results)
    (args.out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (args.out / "summary.md").write_text(markdown(summary))
    print(markdown(summary))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, TimeoutError) as error:
        print(json.dumps(dict(error=str(error))), file=sys.stderr)
        sys.exit(1)
