# Reads the bytes of the install directly, with the Python standard library only (no wt-formats, no wt_blk).
import sys, struct, mmap, os, collections
GAME=sys.argv[1]
c=collections.Counter(); ex={}
def walk(d):
    for r,ds,fs in os.walk(d):
        for f in fs:
            if f.endswith('.dxp.bin'): yield os.path.join(r,f)
for p in walk(GAME):
    f=open(p,'rb'); size=os.path.getsize(p)
    m=mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ)
    d=0x10
    (no,nc),(ho,hc),(ro,rc)=[struct.unpack_from('<II',m,d+o) for o in (0,0x10,0x20)]
    for i in range(rc):
        a,b,cc,off,pk,z=struct.unpack('<IIIIII',m[d+ro+24*i:d+ro+24*i+24])
        h=m[d+ho+32*i:d+ho+32*i+32]
        hpk=struct.unpack_from('<I',h,0x1C)[0]; hmem=struct.unpack_from('<I',h,0x18)[0]
        flags=struct.unpack_from('<I',h,8)[0]
        empty = h[4:]==bytes(28)
        if empty:
            k=('placeholder','rec.pk==0' if pk==0 else 'rec.pk!=0')
        else:
            comp=flags>>29
            k=('real','comp',comp,'hdr.pk==0' if hpk==0 else 'hdr.pk>0','rec.pk==hdr.pk' if pk==hpk else ('rec.pk==mem' if pk==hmem else 'rec.pk other'))
        c[k]+=1
        if k not in ex: ex[k]=(os.path.basename(p),i)
for k,v in sorted(c.items(), key=lambda x:-x[1]): print(v,k,ex[k])
