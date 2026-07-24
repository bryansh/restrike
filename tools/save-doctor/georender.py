import sys
from daxdump import parse_dax

blocks = parse_dax(sys.argv[1])
bid = int(sys.argv[2])
d = blocks[bid][2:]  # strip 2-byte prefix

print("wall map (per cell: NESW nibble/nibble): '#'=fully walled, '.'=open, digits=wall count")
for y in range(16):
    row = []
    for x in range(16):
        n = (d[y*16+x] >> 4) & 0xF
        e = d[y*16+x] & 0xF
        s = (d[0x100+y*16+x] >> 4) & 0xF
        w = d[0x100+y*16+x] & 0xF
        cnt = sum(1 for v in (n,e,s,w) if v != 0)
        row.append('#' if cnt == 4 else ('.' if cnt == 0 else str(cnt)))
    print(f'{y:2d} ' + ''.join(row))
print('    ' + ''.join(f'{x:x}' for x in range(16)))
print()
print("x3 passability (2-bit NESW packed): cell shown as sum of the four 2-bit vals")
for y in range(16):
    row = []
    for x in range(16):
        b = d[0x300+y*16+x]
        tot = (b&3) + ((b>>2)&3) + ((b>>4)&3) + ((b>>6)&3)
        row.append('%x' % tot if tot else '.')
    print(f'{y:2d} ' + ''.join(row))
