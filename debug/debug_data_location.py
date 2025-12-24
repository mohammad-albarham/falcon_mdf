#!/usr/bin/env python3
"""Debug script to find actual data location in MF4 file."""

import struct
from pathlib import Path

def find_data(path):
    """Find actual data in an MF4 file."""
    data = Path(path).read_bytes()
    print(f"File size: {len(data)} bytes")
    
    # Get data offset from DG block
    dg_offset = struct.unpack('<Q', data[88:96])[0]  # dg_first from HD
    data_offset = struct.unpack('<Q', data[dg_offset+24+16:dg_offset+24+24])[0]
    
    print(f"\nDG at offset: {dg_offset}")
    print(f"Data block link: {data_offset}")
    
    if data_offset == 0:
        print("No data block!")
        return
    
    # Read block header at data offset
    block_id = data[data_offset:data_offset+4].decode('latin-1')
    block_len = struct.unpack('<Q', data[data_offset+8:data_offset+16])[0]
    link_count = struct.unpack('<Q', data[data_offset+16:data_offset+24])[0]
    
    print(f"\nBlock at {data_offset}:")
    print(f"  ID: {block_id}")
    print(f"  Length: {block_len}")
    print(f"  Link count: {link_count}")
    
    if block_id == '##DT':
        actual_data_size = block_len - 24
        print(f"  Data size (DT): {actual_data_size}")
        if actual_data_size > 0:
            print(f"  First bytes: {data[data_offset+24:data_offset+24+min(32, actual_data_size)].hex()}")
    
    elif block_id == '##DL':
        # Data List block - follow links
        print("  Data List block - following links...")
        links_start = data_offset + 24
        dl_next = struct.unpack('<Q', data[links_start:links_start+8])[0]
        
        # Data links start after dl_next
        total_data = 0
        for i in range(link_count - 1):
            link_offset = links_start + 8 + i * 8
            data_link = struct.unpack('<Q', data[link_offset:link_offset+8])[0]
            if data_link == 0:
                continue
            
            sub_id = data[data_link:data_link+4].decode('latin-1')
            sub_len = struct.unpack('<Q', data[data_link+8:data_link+16])[0]
            
            print(f"  Link {i}: offset={data_link}, type={sub_id}, length={sub_len}")
            if sub_id == '##DT':
                total_data += sub_len - 24
                
        print(f"  Total data in DL: {total_data} bytes")
        
    elif block_id == '##HL':
        # Header List - get dl_first
        hl_data_start = data_offset + 24 + link_count * 8
        dl_first = struct.unpack('<Q', data[data_offset+24:data_offset+32])[0]
        print(f"  HL block - dl_first: {dl_first}")
        
        if dl_first != 0:
            # Follow DL chain
            current_dl = dl_first
            total_data = 0
            dl_count = 0
            while current_dl != 0:
                dl_id = data[current_dl:current_dl+4].decode('latin-1')
                dl_len = struct.unpack('<Q', data[current_dl+8:current_dl+16])[0]
                dl_link_count = struct.unpack('<Q', data[current_dl+16:current_dl+24])[0]
                
                print(f"\n  DL {dl_count} at {current_dl}: length={dl_len}, links={dl_link_count}")
                
                dl_next = struct.unpack('<Q', data[current_dl+24:current_dl+32])[0]
                
                # Data links 
                for i in range(dl_link_count - 1):
                    link_offset = current_dl + 24 + 8 + i * 8
                    data_link = struct.unpack('<Q', data[link_offset:link_offset+8])[0]
                    if data_link == 0:
                        continue
                    
                    sub_id = data[data_link:data_link+4].decode('latin-1')
                    sub_len = struct.unpack('<Q', data[data_link+8:data_link+16])[0]
                    
                    if sub_id in ('##DT', '##DZ'):
                        actual = sub_len - 24
                        total_data += actual
                        print(f"    Link {i}: {sub_id} at {data_link}, data={actual}")
                
                current_dl = dl_next
                dl_count += 1
                
            print(f"\n  Total data in HL/DL chain: {total_data} bytes")
    
    # Also scan for ##DT blocks in the file
    print("\n\nScanning for ##DT blocks in file...")
    pos = 0
    dt_blocks = []
    while pos < len(data) - 4:
        if data[pos:pos+4] == b'##DT':
            block_len = struct.unpack('<Q', data[pos+8:pos+16])[0]
            actual_size = block_len - 24
            dt_blocks.append((pos, block_len, actual_size))
            print(f"  Found ##DT at {pos}, length={block_len}, data={actual_size}")
            pos += block_len
        else:
            pos += 1
    
    print(f"\nTotal ##DT blocks: {len(dt_blocks)}")
    total = sum(b[2] for b in dt_blocks)
    print(f"Total data: {total} bytes")

if __name__ == '__main__':
    import sys
    if len(sys.argv) > 1:
        find_data(sys.argv[1])
    else:
        test_file = "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4"
        find_data(test_file)
