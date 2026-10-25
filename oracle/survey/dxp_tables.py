# Reads the bytes of the install directly, with the Python standard library only (no wt-formats, no wt_blk).
import sys, struct, mmap, os, collections
GAME=sys.argv[1]
C=collections.Counter
c=C(); recf=C(); fl=C(); lab=C(); pad=C()
def walk(d):
    for r,ds,fs in os.walk(d):
        for f in fs:
            if f.endswith('.dxp.bin'): yield os.path.join(r,f)
n=0; tot=0
for p in walk(GAME):
    size=os.path.getsize(p)
    f=open(p,'rb')
    if size<16: c['tiny']+=1; continue
    m=mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ)
    n+=1
    magic,ver,cnt,dsz=struct.unpack_from('<4sIII',m,0)
    lab[(magic,ver)]+=1
    d=0x10
    (no,nc),(ho,hc),(ro,rc)=[struct.unpack_from('<II',m,d+o) for o in (0,0x10,0x20)]
    c['count==nc==hc==rc' if cnt==nc==hc==rc else 'counts differ']+=1
    c['file>=16+dump' if size>=16+dsz else 'file shorter than dump']+=1
    pad[(m[d+0x08:d+0x10]==bytes(8), m[d+0x18:d+0x20]==bytes(8),m[d+0x28:d+0x30]==bytes(8))]+=1
    c['first table at 0x30' if no==0x30 else 'names table not at 0x30: %x'%no]+=0
    offs=[]
    for i in range(rc):
        r=m[d+ro+24*i:d+ro+24*i+24]
        a,b,cc,off,pk,z=struct.unpack('<IIIIII',r)
        h=m[d+ho+32*i:d+ho+32*i+32]
        tot+=1
        hpk=struct.unpack_from('<I',h,0x1C)[0]; hmem=struct.unpack_from('<I',h,0x18)[0]
        recf[('rec+0',a==0,'rec+4',b==0,'rec+8 ffffffff',cc==0xFFFFFFFF,'rec+0x14',z==0)]+=1
        c['rec packed == hdr packed' if pk==hpk else 'rec packed != hdr packed']+=1
        empty = h[4:]==bytes(28)
        sz = pk if pk>0 else hmem
        if empty: c['placeholder: offset==filesize' if off==size else 'placeholder offset != filesize']+=1; continue
        c['data inside file' if off+sz<=size else 'data PAST end of file']+=1
        offs.append((off,sz))
    offs.sort()
    ok=all(offs[i][0]+offs[i][1]<=offs[i+1][0] for i in range(len(offs)-1))
    c['entries do not overlap' if ok else 'entries overlap']+=1
    c['data region starts 16-aligned?' + str(all(o%16==0 for o,_ in offs))]+=1
print('dxp files',n,'entries',tot)
for name,cc in [('labels',lab),('counts',c),('record fields',recf),('table head padding zero (8,8,8)',pad)]:
    print('==',name)
    for k,v in cc.most_common(14): print('  ',k,v)
