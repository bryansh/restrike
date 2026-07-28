"""Save doctor: party upgrade — clone a save slot with stronger characters.

Philosophy (2026-07-28, Bryan's call): captures don't need wins, but campaigns
2/3 (cleric casting, buffed-affects) need the party to SURVIVE long enough to
be interesting, and magic weapons drive the cited-deferred item `plus` terms.
Edits are restricted to fields with no derived-state risk:

- stats (stats2 @0x10: 7 cur/full byte pairs Str/Int/Wis/Dex/Con/Cha/Str00,
  save-formats §1.7 order CoabCurFull) and HP (max @0x78, cur @0x1A4) are flat
  fields; str/dex combat bonuses are recomputed LIVE from stats by sub_66023.
- gear is fabricated as CHRDAT<slot><n>.swg files (raw 0x3F Item structs, coab
  Item.cs ctor layout) from REAL template bytes harvested out of MONxITM.DAX
  monster kits — only `plus` (@0x32, sbyte) and `count` (@0x39) are retuned,
  and everything ships UN-readied (@0x34=0): the operator readies each item
  in-game so the real recompute derives the record's weapon profile, then
  re-saves — the game-written result becomes the canonical upgraded base.
- NO class-level edits (thac0/saves/spell slots are level-derived; hand-editing
  those invites inconsistency).

Usage: python3 charup.py SRC DST     (e.g. charup.py D H)
"""
import glob, os, shutil, struct, sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from daxdump import parse_dax

SAVE = os.path.expanduser('~/goldbox-data/cotab/SAVE')
GAME = os.path.expanduser('~/goldbox-data/cotab')
ITEM_SZ = 0x3F

SRC, DST = sys.argv[1].upper(), sys.argv[2].upper()
assert SRC != DST

# --- real item templates: (file, block, index) pinned by the 2026-07-28 scan ---
TEMPLATES = {
    'long_sword': ('MON1ITM.DAX', 1, 3),    # type 36
    'short_bow':  ('MON1ITM.DAX', 1, 1),    # type 44
    'long_bow':   ('MON1ITM.DAX', 16, 1),   # type 43
    'arrows':     ('MON1ITM.DAX', 1, 0),    # type 73, the FK 7-arrow quiver
    'shield':     ('MON1ITM.DAX', 1, 2),    # type 59
    'leather':    ('MON1ITM.DAX', 1, 4),    # type 50
    'chain':      ('MON1ITM.DAX', 32, 4),   # type 55
    'banded+1':   ('MON3ITM.DAX', 20, 2),   # type 57, genuine plus=+1
    'mace':       ('MON1ITM.DAX', 90, 0),   # type 23
    'sling':      ('MON2ITM.DAX', 6, 0),    # type 47 (no ammo needed)
    'dagger':     ('MON2ITM.DAX', 2, 0),    # type 8
}

def fetch(name):
    f, bid, idx = TEMPLATES[name]
    data = parse_dax(f'{GAME}/{f}')[bid]
    body = data[2:] if len(data[2:]) % ITEM_SZ == 0 else data
    it = bytearray(body[idx*ITEM_SZ:(idx+1)*ITEM_SZ])
    assert len(it) == ITEM_SZ
    it[0x34] = 0  # un-readied: the in-game READY does the faithful recompute
    return it

def tuned(name, plus=None, count=None):
    it = fetch(name)
    if plus is not None:
        struct.pack_into('b', it, 0x32, plus)
    if count is not None:
        it[0x39] = count
    return bytes(it)

# --- per-character kits + stat targets, slot-D party order (CHRDAT<X>1..6) ---
# classes (turndiff decode): MATHEW/MARK 3, TRAVIS 14 (F/T), LEDERA 13,
# SHARA 0 (cleric), PHILIPPE 5 (MU). Fighter-classes get Str 18/00.
PARTY = {
    1: ('MATHEW',   dict(str=18, str00=100, dex=18, con=18, hp=65), [
        tuned('long_bow'), tuned('arrows', count=40),
        tuned('long_sword', plus=1), tuned('banded+1')]),
    2: ('MARK',     dict(str=18, str00=100, dex=18, con=18, hp=65), [
        tuned('long_sword', plus=2), tuned('shield'), tuned('banded+1')]),
    3: ('TRAVIS',   dict(str=18, str00=100, dex=18, con=18, hp=60), [
        tuned('short_bow'), tuned('arrows', count=40),
        tuned('long_sword', plus=1), tuned('leather')]),  # leather: thief skills
    4: ('LEDERA',   dict(str=18, str00=100, dex=18, con=18, hp=60), [
        tuned('long_sword', plus=2), tuned('shield'), tuned('banded+1')]),
    5: ('SHARA',    dict(str=18, str00=0,   dex=18, con=18, hp=55), [
        tuned('mace', plus=1), tuned('sling'), tuned('shield'), tuned('chain')]),
    6: ('PHILIPPE', dict(str=16, str00=0,   dex=18, con=18, hp=45), [
        tuned('dagger', plus=1)]),
}

STAT_OFS = {'str': 0x10, 'int': 0x12, 'wis': 0x14, 'dex': 0x16,
            'con': 0x18, 'cha': 0x1A, 'str00': 0x1C}

def patch_char(n, name, stats, items):
    rec = bytearray(open(f'{SAVE}/CHRDAT{SRC}{n}.SAV', 'rb').read())
    assert len(rec) == 0x1A6, (n, len(rec))
    nlen = rec[0]
    disk_name = rec[1:1+nlen].decode('ascii', 'replace')
    assert disk_name == name, f'slot {n}: expected {name}, found {disk_name}'
    for k, v in stats.items():
        if k == 'hp':
            rec[0x78] = v    # hit_point_max
            rec[0x1A4] = v   # hit_point_current
        else:
            o = STAT_OFS[k]
            rec[o] = v       # current  (CoabCurFull order)
            rec[o+1] = v     # full
    open(f'{SAVE}/CHRDAT{DST}{n}.SAV', 'wb').write(bytes(rec))
    open(f'{SAVE}/CHRDAT{DST}{n}.SWG', 'wb').write(b''.join(items))
    fx = f'{SAVE}/CHRDAT{SRC}{n}.FX'
    if os.path.exists(fx):
        shutil.copyfile(fx, f'{SAVE}/CHRDAT{DST}{n}.FX')
    print(f'  {name}: stats {stats}, {len(items)} items')

d = bytearray(open(f'{SAVE}/SAVGAM{SRC}.DAT', 'rb').read())
assert len(d) == 13149
for i in range(8):
    ns = 0x3215 + i * 41
    if d[ns] == 8 and d[ns + 1: ns + 7] == b'CHRDAT' and d[ns + 7] == ord(SRC):
        d[ns + 7] = ord(DST)
open(f'{SAVE}/SAVGAM{DST}.DAT', 'wb').write(bytes(d))
print(f'{SRC} -> {DST}: SAVGAM{DST}.DAT written (position untouched)')
for n, (name, stats, items) in sorted(PARTY.items()):
    patch_char(n, name, stats, items)
