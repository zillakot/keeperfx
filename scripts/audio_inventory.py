#!/usr/bin/env python3
"""Build a source cue inventory without loading or distributing game audio."""

import argparse
import hashlib
import json
import re
import subprocess
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CALLS = {
    "play_non_3d_sample": (0, "effect"),
    "play_non_3d_sample_no_overlap": (0, "effect"),
    "thing_play_sample": (1, "effect"),
    "play_sample": (1, "effect"),
    "play_speech_sample": (0, "speech"),
    "output_message": (0, "speech"),
    "output_message_from_path": (0, "speech"),
    "play_music_track": (0, "music"),
    "play_music": (0, "music"),
    "play_music_fgroup": (1, "music"),
    "output_custom_message": (0, "speech"),
    "play_streamed_sample": (0, "speech"),
}
AUDIO_EXTENSIONS = {".wav", ".ogg", ".mp3", ".bmu", ".flac", ".smk"}


def strip_comments(text):
    pattern = r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'|//[^\n]*|/\*[\s\S]*?\*/'
    return re.sub(pattern, lambda m: re.sub(r"[^\n]", " ", m[0])
                  if m[0].startswith(("//", "/*")) else m[0], text)


def config_rows(text):
    section = ""
    for line, raw in enumerate(text.splitlines(), 1):
        value = re.split(r"[;#](?=(?:[^\"]*\"[^\"]*\")*[^\"]*$)", raw, maxsplit=1)[0].strip()
        if value.startswith("[") and value.endswith("]"):
            section = value.strip("[]").lower()
        elif "=" in value:
            name, rhs = value.split("=", 1)
            if name.strip() and rhs.strip():
                yield section, name.strip(), rhs.strip(), line


def variant_ids(value):
    match = re.fullmatch(r"(\d+)(?:\s+(\d+))?(?:\s+STACK=\S+)?", value, re.I)
    if not match:
        return [], None
    first, count = int(match[1]), int(match[2] or 1)
    if first == 0 or count == 0:
        return [], count
    if count > 4096:
        raise ValueError("variant count exceeds inventory safety bound")
    return list(range(first, first + count)), count


def source(path, line):
    return {"path": path, "line": line}


def cue(key, kind, path, line, value, override, ids=None, count=None):
    return {
        "cue_key": key, "category": kind, "trigger": key.rsplit(":", 1)[-1],
        "source": source(path, line), "definition": value,
        "legacy_bank": ("speech" if kind == "speech" else "sound") if ids else None,
        "legacy_ids": ids or [], "variant_count": count,
        "duration_seconds": None, "loop_points_frames": None,
        "source_format": None, "source_format_hint": None, "channels": None, "language": None,
        "override_rule": override, "production_decision": "unassessed",
        "provenance": {"audio_source": None, "creator": None, "license": None,
                       "permission": None, "redistribution": "unknown", "editable_master": None, "export_recipe": None},
    }


def calls(text, names):
    text = strip_comments(text)
    pattern = r"\b(" + "|".join(map(re.escape, names)) + r")\s*\("
    search_text = re.sub(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'', lambda m: " " * len(m[0]), text)
    for match in re.finditer(pattern, search_text):
        args, start, depth, quote, escaped = [], match.end(), 0, None, False
        for pos in range(start, len(text)):
            char = text[pos]
            if quote:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == quote:
                    quote = None
            elif char in "\"'":
                quote = char
            elif char == "(":
                depth += 1
            elif char == ")":
                if depth == 0:
                    args.append(text[start:pos].strip())
                    if not text[pos + 1:].lstrip().startswith("{"):
                        yield match[1], args, text.count("\n", 0, match.start()) + 1
                    break
                depth -= 1
            elif char == "," and depth == 0:
                args.append(text[start:pos].strip())
                start = pos + 1


def literal_ids(expression):
    expression = " ".join(expression.split())
    match = re.fullmatch(r"(\d+)(?:\s*\+\s*SOUND_RANDOM\(\s*(\d+)\s*\))?", expression)
    return variant_ids(f"{match[1]} {match[2] or 1}") if match else ([], None)


def build(root):
    paths = subprocess.check_output(["git", "ls-files", "-z"], cwd=root).decode().split("\0")
    paths = sorted(p for p in paths if p and p.startswith(("src/", "config/", "campgns/", "lang/")))
    entries, references, input_hashes, media = [], [], {}, []
    for path in paths:
        file = root / path
        if file.suffix.lower() in AUDIO_EXTENSIONS:
            media.append({"path": path, "format_hint": file.suffix[1:], "redistribution": "unknown"})
        if file.suffix.lower() not in {".cfg", ".toml", ".c", ".cpp", ".h", ".txt", ".lua", ".po", ".pot"}:
            continue
        data = file.read_bytes()
        text = data.decode("utf-8", errors="replace")
        before = len(entries) + len(references)
        if file.suffix in {".cfg", ".toml"}:
            for section, name, value, line in config_rows(text):
                is_registry = path.endswith("sounds.cfg")
                if section == "sounds":
                    ids, count = variant_ids(value)
                    key = f"named:{name}" if path == "config/fxdata/sounds.cfg" else f"config:{path}:{section}:{name}"
                    kind = "effect" if is_registry else "creature"
                    item = cue(key, kind, path, line, value,
                               "named_custom" if is_registry else "creature_custom", ids, count)
                    item["stack_policy"] = next((part for part in value.split() if part.upper().startswith("STACK=")), "default_once_per_audio_tick") if is_registry else None
                    if count is None:
                        tokens = re.findall(r'"[^"\n]*"|\S+', value)
                        if len(tokens) > 1 and tokens[1].isdecimal():
                            item["variant_count"] = int(tokens[1])
                        item["source_format_hint"] = Path(tokens[0].strip('"')).suffix.lstrip(".") or None
                    if name.isdecimal():
                        item["redirect_from"] = int(name)
                    entries.append(item)
                elif section == "speech" and is_registry:
                    entries.append(cue(f"config:{path}:{section}:{name}", "speech", path, line, value, "speech_path"))
                elif "sound" in name.lower() or name.lower().startswith("messages"):
                    references.append({"kind": "config_field", "source": source(path, line),
                                       "section": section, "field": name, "value": value})
        if path == "src/gui_soundmsgs.h":
            clean = strip_comments(text)
            block = re.search(r"enum TbSpeechMessages\s*\{(.*?)\};", clean, re.S)
            number, named, maximum = -1, {}, None
            for match in re.finditer(r"\b(SMsg_\w+)\s*(?:=\s*(\d+))?\s*,", block[1]):
                number = int(match[2]) if match[2] else number + 1
                line = clean.count("\n", 0, block.start(1) + match.start()) + 1
                if match[1] == "SMsg_MAX":
                    maximum = number
                elif number:
                    named[number] = (match[1][5:], line)
            for number in range(1, maximum):
                name, line = named.get(number, (f"bank_{number}", clean.count("\n", 0, block.start()) + 1))
                item = cue(f"speech:{name}", "speech", path, line, str(number), "speech_bank_or_override", [number], 1)
                item["named_message"] = number in named
                entries.append(item)
        if path.startswith("src/") and file.suffix in {".c", ".cpp"}:
            for name, args, line in calls(text, CALLS):
                index, kind = CALLS[name]
                if len(args) <= index:
                    continue
                expr = " ".join(args[index].split())
                ids, count = literal_ids(expr)
                references.append({"kind": "source_call", "source": source(path, line),
                                   "function": name, "category": kind, "expression": expr,
                                   "literal_legacy_ids": ids, "variant_count": count,
                                   "resolution": "literal" if count is not None else "unresolved_expression"})
        if path.startswith("campgns/") and file.suffix == ".txt":
            for name, args, line in calls(re.sub(r"(?im)^\s*REM\b[^\n]*", "", text),
                                         ["PLAY_MESSAGE", "PLAY_MUSIC"]):
                references.append({"kind": "campaign_script", "source": source(path, line),
                                   "function": name, "arguments": args})
        if path == "src/bflib_sndlib.cpp":
            for function, key in [("resolve_track_music_path", "music:numbered_track"), ("play_music", "music:filename")]:
                match = re.search(r"\b" + function + r"\s*\(", text)
                item = cue(key, "music", path, text.count("\n", 0, match.start()) + 1,
                           "runtime-selected", "music_resolution")
                item["legacy_bank"] = None
                entries.append(item)
        if len(entries) + len(references) > before or path in {"src/sound_manager.cpp", "src/bflib_sndlib.cpp", "src/config.c"}:
            input_hashes[path] = hashlib.sha256(data).hexdigest()
    keys = [entry["cue_key"] for entry in entries]
    if len(keys) != len(set(keys)):
        duplicates = [key for key, count in Counter(keys).items() if count > 1]
        raise ValueError(f"duplicate cue keys: {duplicates}")
    return {
        "schema_version": 1, "scope": "tracked source definitions and selected call-site references; not runtime asset coverage",
        "unknown_fields": "null means not measured or established; empty legacy_ids means no statically resolved bank mapping",
        "override_rules_document": "inventory.md#resolution-rules",
        "counts": {"cues_by_category": dict(sorted(Counter(e["category"] for e in entries).items())),
                   "references_by_kind": dict(sorted(Counter(r["kind"] for r in references).items())),
                   "tracked_media_files": len(media)},
        "input_sha256": input_hashes,
        "speech_translation_sources": [p for p in paths if re.match(r"lang/speech_.*\.(po|pot)$", p)],
        "tracked_media": media, "cues": entries, "references": references,
    }


def render(data):
    parts = []
    for key, value in data.items():
        if key in {"cues", "references"}:
            body = ",\n".join("    " + json.dumps(item, ensure_ascii=False) for item in value)
            parts.append(f'  "{key}": [\n{body}\n  ]')
        else:
            parts.append("  " + json.dumps(key) + ": " + json.dumps(value, ensure_ascii=False, indent=2).replace("\n", "\n  "))
    return "{\n" + ",\n".join(parts) + "\n}\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT)
    parser.add_argument("--output", type=Path, default=Path("docs/audio/cue-inventory.json"))
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    result = render(build(args.root))
    output = args.output if args.output.is_absolute() else args.root / args.output
    if args.check:
        if not output.exists() or output.read_text(encoding="utf-8") != result:
            parser.exit(1, f"Inventory is stale: regenerate {output}\n")
    else:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(result, encoding="utf-8")
    print(json.dumps(json.loads(result)["counts"], sort_keys=True))


if __name__ == "__main__":
    main()
