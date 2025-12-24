#!/usr/bin/env python3
"""Debug script to understand how asammdf finds data in unfinished files."""

from pathlib import Path
import struct

try:
    from asammdf import MDF
    from asammdf.blocks import v4_constants as v4c
except ImportError:
    print("asammdf not installed")
    exit(1)

def debug_asammdf(path):
    """Debug how asammdf handles this file."""
    mdf = MDF(path)
    
    print(f"Version: {mdf.version}")
    print(f"Groups: {len(mdf.groups)}")
    
    for i, group in enumerate(mdf.groups):
        print(f"\nGroup {i}:")
        print(f"  Channel group: {group.channel_group}")
        print(f"  cycles_nr: {group.channel_group.cycles_nr}")
        print(f"  samples_byte_nr: {group.channel_group.samples_byte_nr}")
        print(f"  flags: {group.channel_group.flags}")
        
        # Look at data_block property
        dg = group.data_group
        print(f"  data_group.data_block_addr: {dg.data_block_addr}")
        
        # Check the record attribute
        if hasattr(group, 'record'):
            print(f"  record: {group.record}")
        
        # Try to get data  
        if len(group.channels) > 0:
            try:
                sig = mdf.get(group=i, index=0, raw=True)
                print(f"  Channel 0 samples: {len(sig.samples)}")
            except Exception as e:
                print(f"  Error getting channel: {e}")
    
    # Look at internal structure
    print("\n\nMDF internal structure:")
    print(f"  _master_channel_cache: {len(mdf._master_channel_cache) if hasattr(mdf, '_master_channel_cache') else 'N/A'}")
    
    # Check for HL or DL blocks
    print("\n\nChecking data block types...")
    raw_data = Path(path).read_bytes()
    
    # Read DG block
    hd_offset = 64
    dg_first = struct.unpack('<Q', raw_data[hd_offset+24:hd_offset+32])[0]
    
    dg_offset = dg_first
    dg_links_start = dg_offset + 24
    data_link = struct.unpack('<Q', raw_data[dg_links_start+16:dg_links_start+24])[0]
    
    print(f"Data link from DG: {data_link}")
    
    if data_link > 0:
        block_id = raw_data[data_link:data_link+4]
        print(f"Block ID: {block_id}")
        
        if block_id == b'##HL':
            print("Header List block found!")
            hl_links_start = data_link + 24
            dl_first = struct.unpack('<Q', raw_data[hl_links_start:hl_links_start+8])[0]
            print(f"DL first: {dl_first}")
        elif block_id == b'##DL':
            print("Data List block found!")
        elif block_id == b'##DT':
            print("Data block found - checking if data is actually after the empty DT block...")
            dt_len = struct.unpack('<Q', raw_data[data_link+8:data_link+16])[0]
            print(f"DT length: {dt_len}")
            
            # Check if there's data right after the DT block
            after_dt = data_link + dt_len
            print(f"After DT: {after_dt}")
            print(f"File size: {len(raw_data)}")
            
            if after_dt < len(raw_data):
                next_block_id = raw_data[after_dt:after_dt+4]
                print(f"Next block ID: {next_block_id}")
                
                # Maybe the data is embedded differently
                # In unfinished files, data may be at end of file without proper DT wrapping
                print("\nChecking raw bytes after DT...")
                print(f"Bytes after DT: {raw_data[after_dt:after_dt+100].hex()}")
    
    # Check what's at the end of file (where data typically is in unfinished files)
    print(f"\n\nLast 100 bytes of file:")
    print(raw_data[-100:].hex())
    
    # Look for raw data by checking known record size
    # CG0: rec_id=1, record_size=23 (1 + 22 + 0)
    # Search for record ID 0x01 at regular intervals
    print("\n\nSearching for record ID 0x01 pattern...")
    matches = 0
    for i in range(7240, min(len(raw_data), 10000)):
        if raw_data[i] == 0x01:
            matches += 1
    print(f"Found 0x01 bytes in range 7240-10000: {matches}")
    
    mdf.close()

if __name__ == '__main__':
    import sys
    if len(sys.argv) > 1:
        debug_asammdf(sys.argv[1])
    else:
        test_file = "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4"
        debug_asammdf(test_file)
