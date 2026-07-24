#!/usr/bin/env python3
"""Dump DAX block index + printable strings per block (ECL identification)."""
import sys, re, struct

def parse_dax(path):
    data = open(path, 'rb').read()
    (hdr,) = struct.unpack_from('<H', data, 0)
    n = hdr // 9
    blocks = {}
    base = 2 + hdr
    for i in range(n):
        bid, off, raw, comp = struct.unpack_from('<BIHH', data, 2 + i * 9)
        blocks[bid] = (off, raw, comp)
    out = {}
    for bid, (off, raw, comp) in blocks.items():
        src = data[base + off: base + off + comp]
        dst = bytearray()
        i = 0
        while i < len(src) and len(dst) < raw:
            c = struct.unpack_from('<b', src, i)[0]
            i += 1
            if c >= 0:
                dst += src[i:i + c + 1]
                i += c + 1
            else:
                dst += src[i:i + 1] * (-c)
                i += 1
        out[bid] = bytes(dst)
    return out

def strings(b, minlen=5):
    # plain ASCII runs
    for m in re.finditer(rb'[\x20-\x7e]{%d,}' % minlen, b):
        yield m.start(), m.group().decode('ascii'), 'plain'
    # high-bit-terminated / high-bit-set text (strip bit 7)
    stripped = bytes(ch & 0x7f for ch in b)
    for m in re.finditer(rb'[\x20-\x7e]{%d,}' % minlen, stripped):
        s = m.group().decode('ascii')
        # skip if identical to a plain run (already reported)
        if b[m.start():m.end()] != m.group():
            yield m.start(), s, 'hb'

if __name__ == '__main__':
    path = sys.argv[1]
    only = int(sys.argv[2]) if len(sys.argv) > 2 else None
    blocks = parse_dax(path)
    for bid in sorted(blocks):
        if only is not None and bid != only:
            continue
        b = blocks[bid]
        print(f'=== block {bid}: {len(b)} bytes ===')
        seen = 0
        for off, s, kind in sorted(strings(b)):
            print(f'  {off:#06x} [{kind}] {s}')
            seen += 1
            if seen > 60:
                print('  ... (truncated)')
                break
