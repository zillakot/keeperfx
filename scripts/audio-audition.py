#!/usr/bin/env python3
import argparse
import hashlib
import io
import json
import math
from pathlib import Path
import random
import struct
import wave

RATE = 48000
FAMILIES = [
    ('TAB_CLICK', 60, 1, 'tab', .12, 640, 'wooden detent with a short muted resonance'),
    ('BUTTON_CLICK2', 61, 1, 'button', .04, 880, 'compact tactile latch'),
    ('REFUSAL', 119, 1, 'refusal', .04, 220, 'low double detent, without a musical error beep'),
    ('ROOM_BUILD', 77, 1, 'build', .70, 90, 'stone seating impact with settling grit'),
    ('TILE_SELL', 115, 1, 'sell', .44, 180, 'inward granular sweep and wooden release'),
    ('DIG_IMPACT', 72, 3, 'dig', .85, 330, 'mineral strike with short metal ring and falling grit'),
    ('STRIKE_WALL', 128, 3, 'wall', .50, 145, 'dense stone impact with a dull metal overtone'),
]
LOCAL_FAMILIES = [('HEART_ENGINE', 93, 1, 'heart', True),
                  ('IMP_FOOT', 9, 4, 'imp_foot', False),
                  ('IMP_HIT', 490, 3, 'imp_hit', False),
                  ('IMP_DIE', 493, 2, 'imp_die', False)]


def digest(data):
    return hashlib.sha256(data).hexdigest()


def rms(samples):
    return math.sqrt(sum(x * x for x in samples) / len(samples)) if samples else 0.0


def write_wav(path, samples, rate, width=2):
    if not samples or any(not math.isfinite(x) or abs(x) > 1 for x in samples):
        raise ValueError('WAV export requires finite nonempty signal within full scale')
    path.parent.mkdir(parents=True, exist_ok=True)
    scale = (1 << (8 * width - 1)) - 1
    pcm = b''.join(max(-scale, min(scale, round(x * scale))).to_bytes(width, 'little', signed=True)
                   for x in samples)
    with wave.open(str(path), 'wb') as wav:
        wav.setparams((1, width, rate, len(samples), 'NONE', 'not compressed'))
        wav.writeframes(pcm)
    return {'sha256': digest(path.read_bytes()), 'frames': len(samples), 'sample_rate': rate,
            'channels': 1, 'bits': width * 8, 'duration_seconds': len(samples) / rate,
            'peak_dbfs': 20 * math.log10(max(max(map(abs, samples)), 1e-12)),
            'rms_dbfs': 20 * math.log10(max(rms(samples), 1e-12)),
            'boundary_step': abs(samples[-1] - samples[0])}


def synthesize(kind, duration, frequency, variant):
    rng = random.Random(29013 + variant * 173 + sum(map(ord, kind)))
    samples = []
    low = 0.0
    for i in range(round(duration * RATE)):
        t = i / RATE
        noise = rng.uniform(-1, 1)
        low += .09 * (noise - low)
        f = frequency * (1 + .045 * (variant - 1))
        body = math.sin(math.tau * f * t) * math.exp(-t * (35 if duration < .2 else 13))
        ring = math.sin(math.tau * f * 2.713 * t) * math.exp(-t * 19)
        strike = (noise - low) * math.exp(-t * 150)
        grit = low * math.exp(-t * 7) * (.5 + .5 * math.sin(math.tau * 41 * t) ** 2)
        value = .60 * body + .16 * ring + .45 * strike + .40 * grit
        if kind == 'refusal':
            value = .60 * body + .30 * low * (1 + math.cos(math.tau * 48 * t))
        elif kind == 'sell':
            value = .6 * low * math.sin(math.pi * t / duration) ** 2 + .12 * body
        elif kind in ('build', 'wall'):
            value = .90 * body + .8 * grit + .3 * strike + .07 * ring
        elif kind == 'dig':
            value = .3 * body + .28 * ring + .65 * strike + 1.6 * grit
        edge = min(1.0, t / .001, (duration - t - 1 / RATE) / .008)
        samples.append(value * max(0.0, edge))
    gain = 10 ** (-9 / 20) / max(map(abs, samples))
    return [x * gain for x in samples]


def bank_sample(path, sample_id):
    data = path.read_bytes()
    if len(data) < 4:
        raise ValueError('Truncated sound bank')
    header = struct.unpack_from('<I', data, len(data) - 4)[0]
    if header + 18 + 9 * 16 > len(data) - 4:
        raise ValueError('Sound bank directory outside file')
    table, base, size, _ = struct.unpack_from('<IIII', data, header + 18 + 2 * 16)
    if size % 32 or table + size > len(data) or not 0 <= sample_id < size // 32:
        raise ValueError('Invalid sound bank sample table or ID')
    name, offset, _, length, _, _ = struct.unpack_from('<18sIIIBB', data, table + sample_id * 32)
    start = base + offset
    if start + length > len(data) or length < 12:
        raise ValueError('Sound bank sample outside file')
    raw = data[start:start + length]
    if raw[:4] != b'RIFF' or raw[8:12] != b'WAVE':
        raise ValueError('Expected RIFF/WAVE sample')
    riff_length = struct.unpack_from('<I', raw, 4)[0] + 8
    if riff_length > len(raw):
        raise ValueError('Truncated RIFF sample')
    raw = raw[:riff_length]
    with wave.open(io.BytesIO(raw)) as wav:
        if wav.getnchannels() != 1 or wav.getsampwidth() not in (1, 2):
            raise ValueError('Local recipe requires mono 8/16-bit PCM; no implicit ADPCM conversion')
        rate, width, frames = wav.getframerate(), wav.getsampwidth(), wav.getnframes()
        pcm = wav.readframes(frames)
    if not frames or rate <= 0:
        raise ValueError('Empty PCM sample or invalid sample rate')
    if len(pcm) != frames * width:
        raise ValueError('Truncated PCM sample')
    samples = ([(v - 128) / 128 for v in pcm] if width == 1
               else [v[0] / 32768 for v in struct.iter_unpack('<h', pcm)])
    return samples, rate, raw, {'bank': path.name, 'bank_sha256': digest(data),
                              'index': sample_id, 'source_name': name.split(b'\0')[0].decode('ascii'),
                              'source_sha256': digest(raw), 'bits': width * 8}


def restore(samples, rate, loop=False):
    mean = sum(samples) / len(samples)
    low = samples[0] - mean
    output = []
    alpha = 1 - math.exp(-math.tau * 4500 / rate)
    for sample in samples:
        dry = sample - mean
        low += alpha * (dry - low)
        output.append(.8 * dry + .2 * low)
    if loop:
        width = min(round(rate * .025), len(output) // 4)
        boundary = (output[0] + output[-1]) / 2
        start_delta, end_delta = boundary - output[0], boundary - output[-1]
        for i in range(width):
            weight = .5 + .5 * math.cos(math.pi * i / width)
            output[i] += start_delta * weight
            output[-1 - i] += end_delta * weight
    else:
        width = min(round(rate * .002), len(output) // 4)
        for i in range(width):
            output[i] *= i / width
            output[-1 - i] *= i / width
    return output


def resample(samples, source_rate, target_rate=RATE):
    if source_rate == target_rate:
        return list(samples)
    result = []
    for i in range(round(len(samples) * target_rate / source_rate)):
        at = i * source_rate / target_rate
        lo = min(int(at), len(samples) - 1)
        hi = min(lo + 1, len(samples) - 1)
        result.append(samples[lo] + (samples[hi] - samples[lo]) * (at - lo))
    return result


def match_pair(original, candidate):
    length = max(len(original), len(candidate))
    original = original + [0.0] * (length - len(original))
    candidate = candidate + [0.0] * (length - len(candidate))
    if not rms(original) or not rms(candidate):
        raise ValueError('Cannot level-match silence')
    gain = rms(original) / rms(candidate)
    candidate = [x * gain for x in candidate]
    common = min(1.0, .8 / max(max(map(abs, original)), max(map(abs, candidate))))
    return [x * common for x in original], [x * common for x in candidate], gain, common


def make_pair(root, key, original, original_rate, candidate, candidate_rate):
    a, b, gain, common = match_pair(resample(original, original_rate), resample(candidate, candidate_rate))
    gap = [0.0] * round(.5 * RATE)
    write_wav(root / 'ab' / (key + '.wav'), a + gap + b, RATE)
    return {'file': 'ab/' + key + '.wav', 'order': 'Original, 0.5 seconds silence, candidate',
            'comparison': 'equal RMS over equal padded event windows; not perceptual loudness certification',
            'candidate_gain_db': 20 * math.log10(gain), 'common_gain_db': 20 * math.log10(common)}, (a, b)


def write_json(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2, sort_keys=True) + '\n')


def build(output, original_bank=None, speech_bank=None):
    if output.exists() and any(output.iterdir()):
        raise ValueError('Output must be empty; use a fresh directory to preserve previous auditions')
    mod = output / 'distributable' / 'mods' / 'audio_audition'
    rows, config, pairs = [], ['[sounds]'], {}
    for cue, legacy, count, kind, duration, frequency, description in FAMILIES:
        base = 'audition_' + kind
        first = base + ('1' if count > 1 else '') + '.wav'
        config.append(f'{cue} = {first} {count}')
        for variant in range(1, count + 1):
            filename = base + (str(variant) if count > 1 else '') + '.wav'
            samples = synthesize(kind, duration, frequency, variant)
            stats = write_wav(mod / 'sound' / filename, samples, RATE)
            master = write_wav(mod / 'masters' / filename, samples, RATE, 3)
            row = {'cue': cue, 'variant': variant, 'legacy_index_reference': legacy + variant - 1,
                   'file': 'sound/' + filename, 'master': 'masters/' + filename,
                   'source': 'New deterministic procedural synthesis by Codex for this repository; no sampled input',
                   'license': 'GPL-2.0-or-later', 'status': 'experimental; not listening-approved',
                   'intent': description, 'export': stats, 'master_export': master,
                   'recipe': {'kind': kind, 'duration': duration, 'frequency': frequency, 'variant': variant,
                              'trim': '1 ms attack and 8 ms end envelope; no leading delay',
                              'gain': 'peak -9 dBFS per variant, provisional; final mix unapproved',
                              'loop': False, 'dither': 'none'}}
            if original_bank:
                source, rate, raw, metadata = bank_sample(original_bank, legacy + variant - 1)
                key = cue.lower() + str(variant)
                (output / 'local' / 'originals').mkdir(parents=True, exist_ok=True)
                (output / 'local' / 'originals' / (key + '.wav')).write_bytes(raw)
                comparison, pair = make_pair(output / 'local', key, source, rate, samples, RATE)
                pairs[key] = pair
                write_json(output / 'local' / 'comparisons' / (key + '.json'),
                           {'source': metadata, **comparison, 'redistribution': 'unresolved; local only'})
            rows.append(row)
    (mod / 'fxdata').mkdir(parents=True, exist_ok=True)
    (mod / 'fxdata' / 'sounds.cfg').write_text('\n'.join(config) + '\n')
    write_json(mod / 'manifest.json', {'schema': 1, 'families': len(FAMILIES), 'assets': rows,
                                     'acceptance': 'pending in-game and headphone/speaker listening'})
    local_rows, local_config = [], ['[sounds]']
    for cue, legacy, count, kind, loop in LOCAL_FAMILIES if original_bank else []:
        for variant in range(1, count + 1):
            source, rate, raw, metadata = bank_sample(original_bank, legacy + variant - 1)
            key = kind + str(variant)
            samples = restore(source, rate, loop)
            (output / 'local' / 'originals' / (key + '.wav')).write_bytes(raw)
            stats = write_wav(output / 'local' / 'masters' / (key + '.wav'), samples, rate, 3)
            write_wav(output / 'local' / 'mods' / 'audio_audition_local' / 'sound' / (key + '.wav'), samples, rate)
            comparison, pair = make_pair(output / 'local', key, source, rate, samples, rate)
            pairs[key] = pair
            if loop:
                make_pair(output / 'local', 'heart_loop_three_cycles', source * 3, rate, samples * 3, rate)
            local_rows.append({'cue': cue, 'variant': variant, 'source': metadata, 'export': stats,
                               'comparison': comparison, 'loop_frames': [0, len(samples)] if loop else None,
                               'redistribution': 'unresolved; original and all derivatives local only'})
        if cue == 'HEART_ENGINE':
            local_config.append('HEART_ENGINE = heart1.wav')
    if original_bank:
        crtr = output / 'local' / 'mods' / 'audio_audition_local' / 'creatrs'
        crtr.mkdir(parents=True, exist_ok=True)
        (crtr / 'imp.cfg').write_text('[sounds]\nFoot = imp_foot1.wav 4\nHit = imp_hit1.wav 3\nDie = imp_die1.wav 2\n')
        path = output / 'local' / 'mods' / 'audio_audition_local' / 'fxdata'
        path.mkdir(parents=True, exist_ok=True)
        (path / 'sounds.cfg').write_text('\n'.join(local_config) + '\n')
    if speech_bank:
        source, rate, raw, metadata = bank_sample(speech_bank, 1)
        (output / 'local' / 'originals').mkdir(parents=True, exist_ok=True)
        (output / 'local' / 'originals' / 'mentor_angry.wav').write_bytes(raw)
        samples = restore(source, rate)
        stats = write_wav(output / 'local' / 'masters' / 'mentor_angry.wav', samples, rate, 3)
        comparison, pair = make_pair(output / 'local', 'mentor_angry', source, rate, samples, rate)
        pairs['mentor_angry'] = pair
        local_rows.append({'cue': 'SMsg_CreatrAngryAnyReason', 'source': metadata, 'export': stats,
                           'comparison': comparison, 'integration': 'offline only; preserve language lookup',
                           'redistribution': 'unresolved; original and all derivatives local only'})
    if local_rows:
        write_json(output / 'local' / 'manifest.json', {'assets': local_rows,
                   'recipe': 'Remove whole-clip DC, 80% dry + 20% one-pole 4.5 kHz lowpass; 2 ms boundary fade for one-shots, 25 ms boundary correction for heart loop; source rate and duration retained',
                   'limits': 'No denoising, bandwidth recovery, voice replacement, music source or quality approval'})
    if pairs:
        mixes(output / 'local', pairs)
    return mod


def mixes(root, pairs):
    scenarios = {'quiet': [('tab_click1', .2, .5), ('room_build1', 1.0, .65), ('dig_impact1', 2.0, .55)],
                 'crowded': [('dig_impact' + str(i % 3 + 1), .1 + i * .10, .25) for i in range(12)]}
    scenarios['crowded'] += [('strike_wall' + str(i % 3 + 1), .13 + i * .15, .25) for i in range(8)]
    if 'mentor_angry' in pairs:
        scenarios['crowded'].append(('mentor_angry', .3, .5))
    for name, events in scenarios.items():
        events = [e for e in events if e[0] in pairs]
        if not events:
            continue
        length = max(round(t * RATE) + len(pairs[key][0]) for key, t, gain in events) + RATE // 2
        result = [[0.] * length, [0.] * length]
        for key, t, gain in events:
            start = round(t * RATE)
            for side in (0, 1):
                for i, value in enumerate(pairs[key][side]):
                    result[side][start + i] += value * gain
        a, b, gain, common = match_pair(*result)
        write_wav(root / 'ab' / (name + '_arranged.wav'), a + [0.] * RATE + b, RATE)
        write_json(root / 'ab' / (name + '_arranged.json'),
                   {'events': events, 'order': 'Original, 1 second silence, candidate',
                    'candidate_gain': gain, 'common_gain': common,
                    'evidence': 'offline arrangement; not a game capture, spatial/voice-budget or possession test'})


def main():
    parser = argparse.ArgumentParser(description='Build an opt-in procedural audition pack and optional private A/B artifacts')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--original-bank', type=Path)
    parser.add_argument('--speech-bank', type=Path)
    args = parser.parse_args()
    try:
        print(build(args.output, args.original_bank, args.speech_bank))
    except (ValueError, OSError, wave.Error, struct.error) as exc:
        parser.exit(1, f'{exc}\n')


if __name__ == '__main__':
    main()
