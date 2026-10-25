# Reads the bytes of the install directly, with the Python standard library only (no wt-formats, no wt_blk).
import sys, struct, mmap, os, collections
GAME=sys.argv[1]
C=collections.Counter
labels=C(); rel_c=C(); order=C(); dyn_word=C(); dyn_tag=C(); dyn_inner=C(); tree_c=C(); tree_tag=C(); tree_inner=C()
skin_var=C(); align=C(); h4=C(); h8=C(); h30=C()
samples={}
def walk(d):
    for r,ds,fs in os.walk(d):
        for f in fs:
            if f.endswith('.grp'): yield os.path.join(r,f)
n=0
for p in walk(GAME):
    f=open(p,'rb'); size=os.path.getsize(p)
    if size<0x40: continue
    m=mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ)
    n+=1
    labels[m[:4]]+=1
    d4,d8,dC=struct.unpack_from('<III',m,4)
    rel_c['u32@0xC == size-16' if dC==size-16 else 'u32@0xC != size-16']+=1
    names_at,nc=struct.unpack_from('<II',m,0x10); res_at,rc=struct.unpack_from('<II',m,0x20)
    dep_at,dc=struct.unpack_from('<II',m,0x30)
    h30['dep table count==res count' if dc==rc else 'dep count differs']+=1
    h4['u32@4 == names_at+4*nc? '+str(d4==names_at+4*nc)]+=1
    rs=[]
    for i in range(rc):
        cls,off,nid,x=struct.unpack_from('<IIHH',m,res_at+12*i)
        rs.append((cls,off,nid,x))
    asc=all(rs[i][1]<=rs[i+1][1] for i in range(len(rs)-1))
    order['offsets ascending' if asc else 'NOT ascending']+=1
    if rs: align['first resource offset == end of tables? '+str(rs[0][1]==dep_at+ (dc*8) if False else '')]+=0
    for i,(cls,off,nid,x) in enumerate(rs):
        align['offset%16==0' if off%16==0 else 'offset%16!=0']+=1
        if x!=0: align['u16 after name id != 0']+=1
        end=rs[i+1][1] if i+1<len(rs) else size
        if cls==0xB4B7D9C4 and end-off>=24:
            w=struct.unpack_from('<5I',m,off)
            key=(hex(w[1]),hex(w[2]), 'packed' if (w[4]>>29)==6 else ('stored' if (w[4]>>30)==0 else 'other:%x'%(w[4]>>29)))
            dyn_word[key]+=1
            if (w[4]>>29)==6:
                head=struct.unpack_from('<I',m,off+20)[0]
                dyn_tag[head>>30]+=1
                body=m[off+24:off+24+8]
                kind='zstd' if body[:4]==b'\x28\xb5\x2f\xfd' else 'u32+other(%02x)'%body[4]
                dyn_inner[(head>>30,kind)]+=1
        if cls==0x56F81B6D and end-off>=8:
            ds,cw=struct.unpack_from('<II',m,off)
            tree_c['packed' if cw>>31 else 'stored']+=1
            if cw>>31:
                head=struct.unpack_from('<I',m,off+8)[0]
                tree_tag[head>>30]+=1
                body=m[off+12:off+12+8]
                kind='zstd' if body[:4]==b'\x28\xb5\x2f\xfd' else 'u32+other(%02x)'%body[4]
                tree_inner[(head>>30,kind)]+=1
print('grp files',n)
for name,c in [('labels',labels),('0xC rel',rel_c),('ordering',order),('dep',h30),('u32@4',h4),('align',align),('dynmodel tex/mat words, vertex word kind',dyn_word),('dyn block tag',dyn_tag),('dyn block inner',dyn_inner),('tree',tree_c),('tree tag',tree_tag),('tree inner',tree_inner)]:
    print('==',name)
    for k,v in c.most_common(12): print('  ',k,v)
