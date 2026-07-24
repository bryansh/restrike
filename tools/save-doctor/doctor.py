"""Save doctor: clone a CotAB save slot with the party teleported to another
ECL/GEO block. Offsets per docs/design/save-formats.md §1.1/§1.4 (validated
against the live slot-B save by savedump.py).

v3 — THE LOAD-BEARING PATCH IS THE ECL TRANSPLANT (proven live 2026-07-24,
probe ladder D/E/F/G): the real binary RESUMES THE SAVED RESIDENT-SCRIPT
BYTES (section 5) on load — it does NOT reload a pristine block by
LastEclBlockId the way coab does (coab≠binary, save-formats.md §1.2
addendum). Every id-only probe kept running the source slot's block; the
teleport only took once section 5 carried the destination block's bytes.
The id fields still get patched for bookkeeping consistency (walk-loop
re-stamps, per-block marker gates)."""
import glob, os, shutil, struct

from daxdump import parse_dax

SAVE = os.path.expanduser('~/goldbox-data/cotab/SAVE')
GAME = os.path.expanduser('~/goldbox-data/cotab')
SRC, DST = 'B', 'C'
AREA = 2                           # game_area -> ECL{AREA}.DAX (unchanged here)
BLOCK, X, Y, FACING = 3, 3, 0, 4   # sewers ECL/GEO block 3, entry (3,0), facing S

A1 = 1            # Area1 window base within savgam
ECL_OFF = 0x1401  # section 5: resident ECL block bytes (0x1E00)
ECL_LEN = 0x1E00

d = bytearray(open(f'{SAVE}/SAVGAM{SRC}.DAT', 'rb').read())
assert len(d) == 13149

def put_w(base, off, v): struct.pack_into('<H', d, base + off, v)

# Section 5: transplant the destination block's pristine payload (2-byte
# container prefix stripped), zero-padded — load_ecl_dax Clear()s the buffer
# before SetData, so a fresh-loaded block's tail is zeros.
payload = parse_dax(f'{GAME}/ECL{AREA}.DAX')[BLOCK][2:]
assert len(payload) <= ECL_LEN, len(payload)
d[ECL_OFF:ECL_OFF + ECL_LEN] = payload + bytes(ECL_LEN - len(payload))

put_w(A1, 0x18A, BLOCK)          # current_3DMap_block_id
put_w(A1, 0x1E4, BLOCK)          # LastEclBlockId (bookkeeping; NOT what the
                                 # real binary resumes from — see module doc)
put_w(A1, 0x0F2, BLOCK)          # script-private "current block" marker: every
                                 # ECL2 entry vector opens with COMPARE mem
                                 # 0x4BF2, <own id>; IF = -> EXIT (clean resume
                                 # into the walk loop). Unset -> the block runs
                                 # its first-visit branch instead (benign for
                                 # the sewers: intro text + menu, no reposition).
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
