#!/usr/bin/env python3
"""Compare preserved release binaries, alternating order to reduce timing drift.

Use the corpus manifest from bench_extended.py. Times exclude process startup
and include open + native channel decoding, exactly as that harness does.
"""

import argparse
import hashlib
import json
import pathlib
import re
import statistics
import subprocess
from datetime import datetime

ROOT = pathlib.Path(__file__).resolve().parents[1]


def measure(binary, path):
    output = subprocess.run(
        [str(binary), str(path)], check=True, capture_output=True, text=True
    ).stdout
    fields = {}
    for name in ('open', 'read_native', 'read_f64', 'samples'):
        match = re.search(rf'{name}=([\d.]+)', output)
        if not match:
            raise ValueError(f'Missing {name} in benchmark output: {output}')
        fields[name] = float(match[1])
    return {'total_ms': fields['open'] + fields['read_native'],
            'samples': int(fields['samples'])}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--before', required=True, type=pathlib.Path)
    parser.add_argument('--after', default='target/release/examples/bench', type=pathlib.Path)
    parser.add_argument('--manifest', default='benchmarks/latest_results.json', type=pathlib.Path)
    parser.add_argument('--output', default='benchmarks/before_after_results.json', type=pathlib.Path)
    parser.add_argument('--runs', type=int, default=5)
    args = parser.parse_args()
    if args.runs < 3:
        parser.error('--runs must be at least 3')
    binaries = {'before': args.before.resolve(), 'after': args.after.resolve()}
    manifest = json.loads(args.manifest.read_text())
    results = []
    for entry in manifest['results']:
        path = ROOT / entry['file_path']
        for binary in binaries.values():
            measure(binary, path)
        samples = {'before': [], 'after': []}
        for iteration in range(args.runs):
            order = ('before', 'after') if iteration % 2 == 0 else ('after', 'before')
            for name in order:
                samples[name].append(measure(binaries[name], path))
        counts = {run['samples'] for runs in samples.values() for run in runs}
        if len(counts) != 1:
            raise ValueError(f'Sample counts changed for {path}: {counts}')
        before = statistics.median(x['total_ms'] for x in samples['before'])
        after = statistics.median(x['total_ms'] for x in samples['after'])
        results.append({
            'file_path': entry['file_path'], 'file_size': entry['file_size'],
            'samples': counts.pop(), 'runs': samples,
            'before_ms': before, 'after_ms': after,
            'speedup': before / after if after else None,
        })
        if entry['file_size'] >= 100_000:
            print(f'{path.name}: {before:.2f} -> {after:.2f} ms', flush=True)
    report = {
        'generated_at': datetime.now().astimezone().isoformat(timespec='seconds'),
        'baseline_machine': manifest['machine'],
        'runs_per_binary': args.runs,
        'protocol': 'Warm cache; alternate binary order each round; median open + native read',
        'binaries': {name: {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}
                     for name, path in binaries.items()},
        'results': results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')


if __name__ == '__main__':
    main()
