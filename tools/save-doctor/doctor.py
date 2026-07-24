"""Save doctor: clone a CotAB save slot with the party teleported to another
ECL/GEO block. Offsets per docs/design/save-formats.md §1.1/§1.4 (validated
against the live slot-B save by savedump.py)."""
import glob, os, shutil, struct, sys

SAVE = os.path.expanduser('~/goldbox-data/cotab/SAVE')
SRC, DST = 'B', 'C'
BLOCK, X, Y, FACING = 3, 3, 0, 4   # sewers ECL/GEO block 3, entry (3,0), facing S

A1 = 1  # Area1 window base within savgam

d = bytearray(open(f'{SAVE}/SAVGAM{SRC}.DAT', 'rb').read())
assert len(d) == 13149

def put_w(base, off, v): struct.pack_into('<H', d, base + off, v)

put_w(A1, 0x18A, BLOCK)          # current_3DMap_block_id
put_w(A1, 0x1E4, BLOCK)          # LastEclBlockId (drives pristine ECL reload)
put_w(A1, 0x1E0, X)              # lastXPos
put_w(A1, 0x1E2, Y)              # lastYPos
d[A1 + 0x200: A1 + 0x242] = bytes(0x42)  # field_200 quest flags: fresh entry
d[0x3201:0x3206] = bytes([X, Y, FACING, 0, 0])  # mapPosX/Y/Direction/wall/roof

# section 11: rewrite CHRDAT<slot> letters (length-prefixed names, letter at +7)
for i in range(8):
    ns = 0x3215 + i * 41
    if d[ns] == 8 and d[ns + 1: ns + 7] == b'CHRDAT' and d[ns + 7] == ord(SRC):
        d[ns + 7] = ord(DST)

open(f'{SAVE}/SAVGAM{DST}.DAT', 'wb').write(bytes(d))
copied = [f'SAVGAM{DST}.DAT']
for f in glob.glob(f'{SAVE}/CHRDAT{SRC}*.*'):
    base = os.path.basename(f)
    dst = base.replace(f'CHRDAT{SRC}', f'CHRDAT{DST}', 1)
    shutil.copyfile(f, f'{SAVE}/{dst}')
    copied.append(dst)
print('wrote:', ', '.join(sorted(copied)))
