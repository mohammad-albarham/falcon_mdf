#!/usr/bin/env python3
"""Debug script to analyze MF4 channel structure including composition channels."""

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

def read_text_block(data, offset):
    """Read a TX block and return the text."""
    if offset == 0:
        return ""
    header = read_block_header(data, offset)
    if not header or header['id'] != '##TX':
        return ""
    text_start = offset + 24
    text_len = header['length'] - 24
    text_data = data[text_start:text_start+text_len]
    # Remove null terminator and trailing zeros
    try:
        return text_data.rstrip(b'\x00').decode('utf-8')
    except:
        return text_data.rstrip(b'\x00').decode('latin-1')

def analyze_channels(path):
    """Analyze channel structure in an MF4 file."""
    data = Path(path).read_bytes()
    
    # Get to DG and CG
    dg_first = struct.unpack('<Q', data[88:96])[0]
    
    dg_offset = dg_first
    while dg_offset != 0:
        dg = read_block_header(data, dg_offset)
        print(f"\n=== Data Group at {dg_offset} ===")
        
        links_start = dg_offset + 24
        dg_next = struct.unpack('<Q', data[links_start:links_start+8])[0]
        cg_first = struct.unpack('<Q', data[links_start+8:links_start+16])[0]
        
        cg_offset = cg_first
        cg_idx = 0
        while cg_offset != 0:
            cg = read_block_header(data, cg_offset)
            print(f"\n  === Channel Group {cg_idx} at {cg_offset} ===")
            
            cg_links_start = cg_offset + 24
            cg_next = struct.unpack('<Q', data[cg_links_start:cg_links_start+8])[0]
            cn_first = struct.unpack('<Q', data[cg_links_start+8:cg_links_start+16])[0]
            
            cn_offset = cn_first
            cn_idx = 0
            while cn_offset != 0:
                cn = read_block_header(data, cn_offset)
                if not cn or cn['id'] != '##CN':
                    break
                
                cn_links_start = cn_offset + 24
                cn_next = struct.unpack('<Q', data[cn_links_start:cn_links_start+8])[0]
                composition = struct.unpack('<Q', data[cn_links_start+8:cn_links_start+16])[0]
                tx_name = struct.unpack('<Q', data[cn_links_start+16:cn_links_start+24])[0]
                
                name = read_text_block(data, tx_name)
                
                # Data section
                cn_data_start = cn_offset + 24 + cn['link_count'] * 8
                channel_type = data[cn_data_start]
                sync_type = data[cn_data_start+1]
                data_type = data[cn_data_start+2]
                bit_offset = data[cn_data_start+3]
                byte_offset = struct.unpack('<I', data[cn_data_start+4:cn_data_start+8])[0]
                bit_count = struct.unpack('<I', data[cn_data_start+8:cn_data_start+12])[0]
                
                print(f"\n    Channel {cn_idx}: '{name}'")
                print(f"      type={channel_type}, sync={sync_type}, data_type={data_type}")
                print(f"      byte_offset={byte_offset}, bit_offset={bit_offset}, bit_count={bit_count}")
                print(f"      composition link: {composition}")
                
                # If composition link exists, parse it
                if composition != 0:
                    comp_header = read_block_header(data, composition)
                    if comp_header:
                        print(f"      composition block: {comp_header['id']} at {composition}")
                        
                        # If it's a CN block, it's a structure
                        if comp_header['id'] == '##CN':
                            print("      (Structure channel - nested channels follow)")
                            nested_offset = composition
                            nested_idx = 0
                            while nested_offset != 0:
                                nested_cn = read_block_header(data, nested_offset)
                                if not nested_cn or nested_cn['id'] != '##CN':
                                    break
                                
                                nested_links = nested_offset + 24
                                nested_next = struct.unpack('<Q', data[nested_links:nested_links+8])[0]
                                nested_tx = struct.unpack('<Q', data[nested_links+16:nested_links+24])[0]
                                nested_name = read_text_block(data, nested_tx)
                                
                                nested_data = nested_offset + 24 + nested_cn['link_count'] * 8
                                nested_byte_off = struct.unpack('<I', data[nested_data+4:nested_data+8])[0]
                                nested_bit_count = struct.unpack('<I', data[nested_data+8:nested_data+12])[0]
                                
                                print(f"        Nested {nested_idx}: '{nested_name}' (offset={nested_byte_off}, bits={nested_bit_count})")
                                
                                nested_offset = nested_next
                                nested_idx += 1
                        
                        # If it's a CA block, it's an array
                        elif comp_header['id'] == '##CA':
                            print("      (Array channel)")
                
                cn_offset = cn_next
                cn_idx += 1
            
            cg_offset = cg_next
            cg_idx += 1
        
        dg_offset = dg_next

if __name__ == '__main__':
    import sys
    if len(sys.argv) > 1:
        analyze_channels(sys.argv[1])
    else:
        test_file = "test_data/mf4-sample-data-v2.1/OBD2 (Audi A4)/LOG/31CB1F25/00000022/00000002.MF4"
        analyze_channels(test_file)
