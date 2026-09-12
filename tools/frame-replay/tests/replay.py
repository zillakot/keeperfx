#!/usr/bin/env python3
"""Exercise the replay CLI and exact GPU comparisons without game assets."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("replay", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)

    def run(*arguments, passed=True):
        result = subprocess.run([str(args.replay), *map(str, arguments)], check=False)
        assert result.returncode == (0 if passed else 1), result.returncode

    def read(path):
        return json.loads(path.read_text())

    fixture = args.output / "fixture"
    run("--fixture", fixture)
    for scale in (1, 2, 3):
        output = args.output / f"single-{scale}"
        run(fixture / "frame.kfx", "--out", output, "--scale", scale)
        report = read(output / "report.json")
        assert report["passed"]
        assert report["different_pixels"] == 0
        assert report["capture_reference_different_pixels"] == 0

    frame = fixture / "frame.kfx"
    data = bytearray(frame.read_bytes())
    data[-1] ^= 1
    frame.write_bytes(data)
    output = args.output / "single-index-mismatch"
    run(frame, "--out", output, passed=False)
    report = read(output / "report.json")
    assert not report["passed"]
    assert report["different_pixels"] == 1
    assert report["capture_reference_different_pixels"] == 1

    sequence = args.output / "sequence"
    run("--sequence-fixture", sequence)
    for scale in range(1, 9):
        output = args.output / f"sequence-{scale}"
        run("--sequence", sequence / "sequence.json", "--out", output, "--scale", scale)
        report = read(output / "sequence-report.json")
        assert report["passed"]
        assert report["frame_count"] == 7
        assert [entry["index"] for entry in report["frames"]] == list(range(7))
        assert all(entry["different_pixels"] == 0 for entry in report["frames"])
        assert all(entry["capture_reference_different_pixels"] == 0 for entry in report["frames"])

    for name, palette_index, channel in (
        ("palette-rgb", 17, 0),
        ("alpha-only", 128, 3),
        ("transparent-rgb", 128, 0),
    ):
        corrupted = args.output / f"corrupted-{name}"
        shutil.copytree(sequence, corrupted)
        frame = corrupted / "frame-0002.kfx"
        data = bytearray(frame.read_bytes())
        data[16 + palette_index * 4 + channel] ^= 1
        expected = data[16 + 256 * 4:].count(palette_index)
        assert expected > 0
        frame.write_bytes(data)
        output = args.output / f"sequence-mismatch-{name}"
        run("--sequence", corrupted / "sequence.json", "--out", output, "--scale", 2, passed=False)
        report = read(output / "sequence-report.json")
        assert not report["passed"]
        assert report["frame_count"] == 7
        assert [entry["index"] for entry in report["frames"] if not entry["passed"]] == [2]
        changed = report["frames"][2]
        assert changed["capture_reference_different_pixels"] == expected
        assert changed["different_pixels"] == expected * 4
        detail = read(output / "frame-0002" / "report.json")
        assert detail["max_channel_error"] == 1

    print(f"All synthetic replay checks passed: {args.output}")


if __name__ == "__main__":
    main()
