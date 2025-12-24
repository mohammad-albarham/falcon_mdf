#!/usr/bin/env python3
"""Debug script to analyze MF4 data records and calculate sample counts."""

import struct
from pathlib import Path
from collections import defaultdict

def analyze_records(path):
    """Analyze data records in an MF4 file."""
    data = Path(path).read_bytes()
    
    # Check if unfinished
    file_id = data[0:8].decode('latin-1').strip()
    is_unfinished = file_id == 'UnFinMF'
    print(f"File ID: '{file_id}' (Unfinished: {is_unfinished})")
    
    # Get HD block
    hd_offset = 64
    dg_first = struct.unpack('<Q', data[88:96])[0]
    
    # Get DG block
    dg_offset = dg_first
    links_start = dg_offset + 24
    cg_first = struct.unpack('<Q', data[links_start+8:links_start+16])[0]
    data_offset = struct.unpack('<Q', data[links_start+16:links_start+24])[0]
    
    dg_data_start = dg_offset + 24 + 4 * 8  # 4 links
    rec_id_size = data[dg_data_start]
    
    print(f"\nrec_id_size: {rec_id_size}")
    print(f"Data block offset: {data_offset}")
    
    # Get data block info
    if data_offset > 0:
        block_id = data[data_offset:data_offset+4].decode('latin-1')
        block_len = struct.unpack('<Q', data[data_offset+8:data_offset+16])[0]
        data_start = data_offset + 24  # After block header
        data_size = block_len - 24
        
        print(f"Data block type: {block_id}")
        print(f"Data block length: {block_len}")
        print(f"Actual data size: {data_size}")
    
    # Collect CG info
    cgs = []
    cg_offset = cg_first
    while cg_offset != 0:
        cg_links_start = cg_offset + 24
        cg_next = struct.unpack('<Q', data[cg_links_start:cg_links_start+8])[0]
        
        cg_header = struct.unpack('<Q', data[cg_offset+16:cg_offset+24])[0]
        cg_data_start = cg_offset + 24 + 6 * 8  # 6 links
        
        record_id = struct.unpack('<Q', data[cg_data_start:cg_data_start+8])[0]
        cycle_count = struct.unpack('<Q', data[cg_data_start+8:cg_data_start+16])[0]
        data_bytes = struct.unpack('<I', data[cg_data_start+24:cg_data_start+28])[0]
        inval_bytes = struct.unpack('<I', data[cg_data_start+28:cg_data_start+32])[0]
        
        record_size = rec_id_size + data_bytes + inval_bytes
        
        cgs.append({
            'record_id': record_id,
            'cycle_count': cycle_count,
            'data_bytes': data_bytes,
            'inval_bytes': inval_bytes,
            'record_size': record_size
        })
        
        print(f"\nCG {len(cgs)-1}:")
        print(f"  record_id: {record_id}")
        print(f"  cycle_count (from block): {cycle_count}")
        print(f"  data_bytes: {data_bytes}, inval_bytes: {inval_bytes}")
        print(f"  record_size: {record_size}")
        
        cg_offset = cg_next
    
    # Count records by scanning data
    if rec_id_size > 0 and data_offset > 0:
        print(f"\n=== Scanning data records ===")
        record_counts = defaultdict(int)
        
        pos = data_start
        data_end = data_offset + block_len
        
        while pos < data_end:
            # Read record ID
            if rec_id_size == 1:
                rec_id = data[pos]
            elif rec_id_size == 2:
                rec_id = struct.unpack('<H', data[pos:pos+2])[0]
            elif rec_id_size == 4:
                rec_id = struct.unpack('<I', data[pos:pos+4])[0]
            elif rec_id_size == 8:
                rec_id = struct.unpack('<Q', data[pos:pos+8])[0]
            else:
                break
            
            # Find matching CG
            found = False
            for cg in cgs:
                if cg['record_id'] == rec_id:
                    record_counts[rec_id] += 1
                    pos += cg['record_size']
                    found = True
                    break
            
            if not found:
                print(f"Unknown record ID {rec_id} at position {pos}")
                break
        
        print(f"\nRecord counts by channel group:")
        for i, cg in enumerate(cgs):
            calculated = record_counts[cg['record_id']]
            stored = cg['cycle_count']
            print(f"  CG {i} (rec_id={cg['record_id']}): calculated={calculated}, stored={stored}")

if __name__ == '__main__':
    import sys
    if len(sys.argv) > 1:
        analyze_records(sys.argv[1])
    else:
        test_file = "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4"
        analyze_records(test_file)
