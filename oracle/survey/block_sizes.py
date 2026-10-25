# Reads the bytes of the install directly, with the Python standard library only (no wt-formats, no wt_blk).
# The largest unpacked size the vertex-data blocks of DynModels (first block of each model) claim.
import sys, struct, mmap, os
GAME=sys.argv[1]; best=(0,''); n=0
for r,ds,fs in os.walk(GAME):
    for f in fs:
        if not f.endswith('.grp'): continue
        p=os.path.join(r,f); fh=open(p,'rb'); m=mmap.mmap(fh.fileno(),0,access=mmap.ACCESS_READ)
        res_at,rc=struct.unpack_from('<II',m,0x20)
        for i in range(rc):
            cls,off,nid,x=struct.unpack_from('<IIHH',m,res_at+12*i)
            end=struct.unpack_from('<I',m,res_at+12*(i+1)+4)[0] if i+1<rc else len(m)
            if cls!=0xB4B7D9C4 or end-off<28: continue
            w=struct.unpack_from('<5I',m,off)
            if w[1]!=0xFFFFFFFF or w[4]>>29!=6: continue
            n+=1; sz=struct.unpack_from('<I',m,off+24)[0]
            if sz>best[0]: best=(sz,p)
print(n,'packed vertex blocks; largest unpacked size',best)
