# Reads the bytes of the install directly, with the Python standard library only (no wt-formats, no wt_blk).
import sys, struct, mmap
def resources(path):
    f=open(path,'rb'); m=mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ)
    names_at,nc=struct.unpack_from('<II',m,0x10); res_at,rc=struct.unpack_from('<II',m,0x20)
    names=[]
    for i in range(nc):
        o=struct.unpack_from('<i',m,names_at+4*i)[0]
        if o<0: names.append(''); continue
        e=m.find(b'\0',o); names.append(m[o:e].decode('utf8','replace'))
    rs=[]
    for i in range(rc):
        cls,off,nid,_=struct.unpack_from('<IIHH',m,res_at+12*i)
        rs.append([names[nid],cls,off,None])
    for i in range(rc):
        rs[i][3]=(rs[i+1][2] if i+1<rc else len(m))
    return m,rs
if __name__=='__main__':
    m,rs=resources(sys.argv[1])
    for n,c,o,e in rs: print(f'{c:08X} {o:#x}..{e:#x} ({e-o}) {n}')
