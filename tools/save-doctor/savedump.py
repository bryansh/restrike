import struct, sys
p = sys.argv[1]
d = open(p,'rb').read()
assert len(d) == 13149, len(d)
A1 = 1            # Area1 window base (file offset)
A2 = 0x801        # Area2 window base
POS = 0x3201      # position block
def w(base, off): return struct.unpack_from('<H', d, base+off)[0]
print('game_area (sec1):', d[0])
print('Area1.current_3DMap_block_id @0x18A:', w(A1,0x18A))
print('Area1.inDungeon @0x1CC:', w(A1,0x1CC))
print('Area1.lastXPos @0x1E0:', w(A1,0x1E0), ' lastYPos @0x1E2:', w(A1,0x1E2))
print('Area1.LastEclBlockId @0x1E4:', w(A1,0x1E4))
print('Area1.current_city @0x342:', w(A1,0x342))
print('Area1.field_200 flags:', d[A1+0x200:A1+0x242].hex())
print('Area2.game_area @0x624:', w(A2,0x624), ' party_size @0x67C:', w(A2,0x67C))
print('pos block: x=%d y=%d dir=%d wallType=%d wallRoof=%d' % tuple(d[POS:POS+5]))
print('last_game_state:', d[0x3206], ' game_state:', d[0x3207])
print('setBlocks:', struct.unpack_from('<6h', d, 0x3208))
print('party_count:', d[0x3214])
names = d[0x3215:0x3215+0x148]
print('names:', [names[i*41:i*41+41].split(b'\0')[0] for i in range(8)])
