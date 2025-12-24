#!/usr/bin/env python3
"""Compare falcon_mdf output with asammdf for verification."""

import time
import subprocess
from pathlib import Path

try:
    from asammdf import MDF
    HAS_ASAMMDF = True
except ImportError:
    HAS_ASAMMDF = False
    print("Warning: asammdf not installed")

def analyze_with_asammdf(path):
    """Analyze file with asammdf."""
    if not HAS_ASAMMDF:
        return None
    
    start = time.perf_counter()
    mdf = MDF(path)
    open_time = (time.perf_counter() - start) * 1000
    
    result = {
        'open_time_ms': open_time,
        'version': mdf.version,
        'groups': len(mdf.groups),
        'channels': {},
        'samples': {}
    }
    
    total_channels = 0
    for i, group in enumerate(mdf.groups):
        channels_in_group = len(group.channels)
        total_channels += channels_in_group
        result['channels'][i] = channels_in_group
        
        # Get sample count from first channel
        if channels_in_group > 0:
            try:
                sig = mdf.get(group=i, index=0)
                result['samples'][i] = len(sig.samples)
            except:
                result['samples'][i] = 0
    
    result['total_channels'] = total_channels
    result['total_samples'] = sum(result['samples'].values())
    
    # Get channel names
    result['channel_names'] = list(mdf.channels_db.keys())
    
    mdf.close()
    return result

def analyze_with_falcon(path):
    """Analyze file with falcon_mdf (via cargo example)."""
    try:
        result = subprocess.run(
            ['cargo', 'run', '--release', '--example', 'analyze_mf4'],
            capture_output=True,
            text=True,
            timeout=30
        )
        return {
            'stdout': result.stdout,
            'stderr': result.stderr,
            'returncode': result.returncode
        }
    except Exception as e:
        return {'error': str(e)}

def compare(path):
    """Compare both implementations."""
    print(f"Analyzing: {path}")
    print("=" * 60)
    
    # asammdf
    print("\n=== asammdf Results ===")
    asammdf_result = analyze_with_asammdf(path)
    if asammdf_result:
        print(f"Open time: {asammdf_result['open_time_ms']:.2f} ms")
        print(f"Version: {asammdf_result['version']}")
        print(f"Groups: {asammdf_result['groups']}")
        print(f"Total channels: {asammdf_result['total_channels']}")
        print(f"Total samples: {asammdf_result['total_samples']}")
        print(f"\nSamples per group:")
        for i, count in asammdf_result['samples'].items():
            print(f"  Group {i}: {count}")
        print(f"\nChannel names ({len(asammdf_result['channel_names'])}):")
        for name in sorted(asammdf_result['channel_names'])[:20]:
            print(f"  - {name}")
        if len(asammdf_result['channel_names']) > 20:
            print(f"  ... and {len(asammdf_result['channel_names']) - 20} more")
    
    # falcon_mdf
    print("\n=== falcon_mdf Results ===")
    falcon_result = analyze_with_falcon(path)
    if 'error' in falcon_result:
        print(f"Error: {falcon_result['error']}")
    else:
        print(falcon_result['stdout'])
        if falcon_result['stderr']:
            print(f"Stderr: {falcon_result['stderr']}")

if __name__ == '__main__':
    import sys
    if len(sys.argv) > 1:
        compare(sys.argv[1])
    else:
        test_file = "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4"
        compare(test_file)
