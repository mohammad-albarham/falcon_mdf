#!/usr/bin/env python3
"""Debug script to analyze MF4 file structure."""

import struct
from pathlib import Path

def read_block_header(data, offset):
    """Read a block header at the given offset."""
    if offset + 24 > len(data):
        return None
    block_id = data[offset:offset+4].decode('latin-1')
    reserved = struct.unpack('<I', data[offset+4:offset+8])[0]
    length = struct.unpack('<Q', data[offset+8:offset+16])[0]
    link_count = struct.unpack('<Q', data[offset+16:offset+24])[0]
    return {
        'id': block_id,
        'reserved': reserved,
        'length': length,
        'link_count': link_count,
        'offset': offset
    }

def analyze_mf4(path):
    """Analyze an MF4 file structure."""
    data = Path(path).read_bytes()
    print(f"File size: {len(data)} bytes")
    
    # ID Block (first 64 bytes)
    file_id = data[0:8].decode('latin-1').strip()
    format_id = data[8:16].decode('latin-1').strip()
    program_id = data[16:24].decode('latin-1').strip()
    version = struct.unpack('<H', data[28:30])[0]
    unfinalized_flags = struct.unpack('<H', data[60:62])[0]
    
    print(f"\n=== ID Block ===")
    print(f"File ID: '{file_id}' (Unfinished: {file_id == 'UnFinMF'})")
    print(f"Format: '{format_id}'")
    print(f"Program: '{program_id}'")
    print(f"Version: {version} ({version // 100}.{version % 100})")
    print(f"Unfinalized flags: {unfinalized_flags}")
    
    # HD Block at offset 64
    hd = read_block_header(data, 64)
    if hd and hd['id'] == '##HD':
        print(f"\n=== HD Block ===")
        print(f"Length: {hd['length']}, Links: {hd['link_count']}")
        
        # First link is dg_first
        dg_first = struct.unpack('<Q', data[88:96])[0]
        print(f"DG first offset: {dg_first}")
        
        # Parse DG chain
        dg_offset = dg_first
        dg_index = 0
        while dg_offset != 0:
            dg = read_block_header(data, dg_offset)
            if not dg or dg['id'] != '##DG':
                break
            
            print(f"\n=== DG Block {dg_index} ===")
            print(f"Offset: {dg_offset}, Length: {dg['length']}, Links: {dg['link_count']}")
            
            # Links
            links_start = dg_offset + 24
            dg_next = struct.unpack('<Q', data[links_start:links_start+8])[0]
            cg_first = struct.unpack('<Q', data[links_start+8:links_start+16])[0]
            data_offset = struct.unpack('<Q', data[links_start+16:links_start+24])[0]
            
            # Data section
            data_start = dg_offset + 24 + dg['link_count'] * 8
            rec_id_size = data[data_start]
            
            print(f"DG next: {dg_next}, CG first: {cg_first}")
            print(f"Data offset: {data_offset}, rec_id_size: {rec_id_size}")
            
            # Parse CG chain
            cg_offset = cg_first
            cg_index = 0
            while cg_offset != 0:
                cg = read_block_header(data, cg_offset)
                if not cg or cg['id'] != '##CG':
                    break
                
                print(f"\n  === CG Block {cg_index} ===")
                print(f"  Offset: {cg_offset}, Length: {cg['length']}, Links: {cg['link_count']}")
                
                # Links
                cg_links_start = cg_offset + 24
                cg_next = struct.unpack('<Q', data[cg_links_start:cg_links_start+8])[0]
                cn_first = struct.unpack('<Q', data[cg_links_start+8:cg_links_start+16])[0]
                
                # Data section
                cg_data_start = cg_offset + 24 + cg['link_count'] * 8
                record_id = struct.unpack('<Q', data[cg_data_start:cg_data_start+8])[0]
                cycle_count = struct.unpack('<Q', data[cg_data_start+8:cg_data_start+16])[0]
                flags = struct.unpack('<H', data[cg_data_start+16:cg_data_start+18])[0]
                path_sep = struct.unpack('<H', data[cg_data_start+18:cg_data_start+20])[0]
                data_bytes = struct.unpack('<I', data[cg_data_start+24:cg_data_start+28])[0]
                inval_bytes = struct.unpack('<I', data[cg_data_start+28:cg_data_start+32])[0]
                
                print(f"  CG next: {cg_next}, CN first: {cn_first}")
                print(f"  record_id: {record_id}, cycle_count: {cycle_count}")
                print(f"  flags: {flags}, data_bytes: {data_bytes}, inval_bytes: {inval_bytes}")
                
                # Count channels
                cn_count = 0
                cn_offset = cn_first
                while cn_offset != 0:
                    cn = read_block_header(data, cn_offset)
                    if not cn or cn['id'] != '##CN':
                        break
                    cn_count += 1
                    cn_links = cn_offset + 24
                    cn_offset = struct.unpack('<Q', data[cn_links:cn_links+8])[0]
                
                print(f"  Channel count: {cn_count}")
                
                cg_offset = cg_next
                cg_index += 1
            
            dg_offset = dg_next
            dg_index += 1

if __name__ == '__main__':
    import sys
    if len(sys.argv) > 1:
        analyze_mf4(sys.argv[1])
    else:
        # Default test file
        test_file = "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4"
        analyze_mf4(test_file)
