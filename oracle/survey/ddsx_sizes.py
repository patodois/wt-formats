import sys, struct, mmap, os, collections
GAME=sys.argv[1]
c=collections.Counter(); ex={}
def walk(d):
    for r,ds,fs in os.walk(d):
        for f in fs:
            if f.endswith('.dxp.bin'): yield os.path.join(r,f)
def side(v,l): return max(1,v>>l) if l<64 else 1
for p in walk(GAME):
    f=open(p,'rb'); m=mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ)
    d=0x10
    (ho,hc)=struct.unpack_from('<II',m,d+0x10)
    for i in range(hc):
        h=m[d+ho+32*i:d+ho+32*i+32]
        if h[4:]==bytes(28): continue
        fmt,fl,w,hh,lv,hq,dep,bpp=struct.unpack_from('<IIHHBBHH',h,4)
        sh=h[0x17]&15; mem=struct.unpack_from('<I',h,0x18)[0]
        if fl&0x20000: continue
        gen=bool(fl&0x6000); levels=1 if gen else max(lv,1)
        tot=0
        for l in range(levels):
            ww,h2=side(w,l),side(hh,l)
            s=(-(-ww//4)*-(-h2//4))<<sh if sh else ww*h2*bpp//8
            if fl&0x1000: s*=side(dep,l)
            tot+=s
        if fl&0x800: tot*=6
        k=('genmip' if gen else 'full', 'mem==described' if mem==tot else ('mem>described' if mem>tot else 'mem<described'))
        c[k]+=1
        if k not in ex: ex[k]=(os.path.basename(p),i,mem,tot)
        if mem>tot: c[('extra bytes',gen,mem-tot)]+=1
for k,v in sorted(c.items(), key=lambda x:-x[1])[:14]: print(v,k,ex.get(k,''))
