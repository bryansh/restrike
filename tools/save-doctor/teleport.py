"""Save doctor: SAME-BLOCK position teleport — clone a CotAB save slot with
only the party's position/facing changed.

Unlike doctor.py (v3, the cross-block transplant), this variant does NOT touch
section 5 (resident ECL), the block ids, or the field_200 quest flags: the
source slot's script state rides along unchanged. That makes it the mildest
possible edit — the load resumes the source's own game-written resident bytes
(the §1.2-addendum semantics) and merely finds the party standing elsewhere in
the same GEO block. Built 2026-07-28 to stage the sewer troll-tussock and
otyugh-attack captures, whose map regions are walk-unreachable from the slot-D
entry point (the east strip and the center are separate components, entered
only by script teleports).

Usage: python3 teleport.py SRC DST X Y FACING
  e.g.  python3 teleport.py D E 11 2 2   # trolls: land (11,2) facing E
        python3 teleport.py D F 8 13 4   # otyugh attack: land (8,13) facing S

FACING is half-encoded: 0 N / 2 E / 4 S / 6 W (the map_direction convention).
"""
import glob, os, shutil, struct, sys

SAVE = os.path.expanduser('~/goldbox-data/cotab/SAVE')

SRC, DST = sys.argv[1].upper(), sys.argv[2].upper()
X, Y, FACING = int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
assert SRC != DST and 0 <= X < 16 and 0 <= Y < 16 and FACING in (0, 2, 4, 6)

A1 = 1  # Area1 window base within savgam (save-formats.md §1.1)

d = bytearray(open(f'{SAVE}/SAVGAM{SRC}.DAT', 'rb').read())
assert len(d) == 13149

struct.pack_into('<H', d, A1 + 0x1E0, X)  # Area1.lastXPos
struct.pack_into('<H', d, A1 + 0x1E2, Y)  # Area1.lastYPos
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
print(f'{SRC} -> {DST} @ ({X},{Y}) facing {FACING}; wrote:', ', '.join(sorted(copied)))
