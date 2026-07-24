import sys
from daxdump import parse_dax

def sixbit_stream(b, start):
    bits = 0; nbits = 0; out = []
    for byte in b[start:]:
        bits = (bits << 8) | byte; nbits += 8
        while nbits >= 6:
            nbits -= 6
            code = (bits >> nbits) & 0x3F
            if code == 0:
                out.append('\x00')
            elif code <= 0x1F:
                out.append(chr(code + 0x40))
            else:
                out.append(chr(code))
    return ''.join(out)

blocks = parse_dax(sys.argv[1])
words = sys.argv[2:] or ['SEWER','OTYUGH','TROLL','GUILD','COACH','PRINCESS','KNIFE','MONKEY','TAVERN','TEMPLE']
for bid in sorted(blocks):
    b = blocks[bid]
    hits = set()
    for phase in range(3):
        s = sixbit_stream(b, phase)
        for w in words:
            if w in s:
                hits.add(w)
    print(f'block {bid}: {sorted(hits)}')
