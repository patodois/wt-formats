# Reads the bytes of the install directly, with the Python standard library only (no wt-formats, no wt_blk).
import sys, struct, mmap, os, collections
GAME=sys.argv[1]
C=collections.Counter
var=C(); vw=C(); zl=C(); n_skin=0; n_models=0; errs=C(); sizeflag=C(); lodskin=C()
def walk(d):
    for r,ds,fs in os.walk(d):
        for f in fs:
            if f.endswith('.grp'): yield os.path.join(r,f)
for p in walk(GAME):
    f=open(p,'rb'); size=os.path.getsize(p)
    m=mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ)
    names_at,nc=struct.unpack_from('<II',m,0x10); res_at,rc=struct.unpack_from('<II',m,0x20)
    rs=[struct.unpack_from('<IIHH',m,res_at+12*i) for i in range(rc)]
    for i,(cls,off,nid,x) in enumerate(rs):
        if cls!=0xB4B7D9C4: continue
        end=rs[i+1][1] if i+1<rc else size
        if end-off<24: continue
        w=struct.unpack_from('<5I',m,off)
        if w[1]!=0xFFFFFFFF: continue
        lods_size=w[0]; vword=w[4]
        pos=off+20
        if vword>>29==6:
            head=struct.unpack_from('<I',m,pos)[0]; pos+=4+(head&0x3FFFFFFF)
        elif vword>>30==0:
            hs=vword; hdr=m[pos:pos+hs]; tab,cnt=struct.unpack_from('<II',hdr,16)
            pay=0
            for k in range(cnt):
                vn,w1,w2,fl=struct.unpack_from('<IIII',hdr,tab+32*k)
                pay+=vn*(w1&0xFF)+ (((w1>>8)|((w2>>28)<<24)) if fl&0x200 else (w2&0x0FFFFFFF))
            pos+=hs+pay
        else: continue
        lods=m[pos:pos+lods_size]; pos+=lods_size
        ns=struct.unpack_from('<I',m,pos)[0]; pos+=4+ns
        tab,lc=struct.unpack_from('<II',lods,0)
        tot=0
        try:
            for l in range(lc):
                ssz=struct.unpack_from('<Q',lods,tab+16*l)[0]
                scene=m[pos:pos+ssz]; pos+=ssz
                ro,rcnt=struct.unpack_from('<II',scene,0); sk=struct.unpack_from('<I',scene,20)[0]
                tot+=sk
                for r in range(rcnt):
                    ms=struct.unpack_from('<Q',scene,ro+32*r)[0]; pos+=ms
        except Exception as e:
            errs[str(e)[:40]]+=1; continue
        n_models+=1
        if tot==0: continue
        n_skin+=1
        variants,total=struct.unpack_from('<II',m,pos)
        pairs=[struct.unpack_from('<II',m,pos+8+8*k) for k in range(variants)]
        var[(variants,tuple(pairs))]+=1
        # next words after the chosen variant's skip
        sk_off=pos+8+8*1+pairs[0][1] if variants>=1 else pos+8
        words=struct.unpack_from('<4I',m,sk_off)
        vw[(hex(words[0]),'packed' if words[3]>>29==6 else 'stored' if words[3]>>30==0 else 'other')]+=1
        # distance to the end of section
        lodskin[(tuple(),)]+=0
print('models walked',n_models,'with skins',n_skin,'errors',dict(errs))
print('== variants (count, [(consts, skip)])')
for k,v in var.most_common(10): print('  ',k,v)
print('== words after chosen variant')
for k,v in vw.most_common(10): print('  ',k,v)
