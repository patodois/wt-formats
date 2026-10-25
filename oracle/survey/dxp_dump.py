# Reads the bytes of the install directly, with the Python standard library only (no wt-formats, no wt_blk).
import sys, struct, mmap
p=sys.argv[1]; n=int(sys.argv[2]) if len(sys.argv)>2 else 3
f=open(p,'rb'); m=mmap.mmap(f.fileno(),0,access=mmap.ACCESS_READ)
magic,ver,cnt,dsz=struct.unpack_from('<4sIII',m,0)
print(p.split('/')[-1],magic,ver,cnt,hex(dsz),'filesize',len(m))
d=0x10
(no,nc),(ho,hc),(ro,rc)=[struct.unpack_from('<II',m,d+o) for o in (0,0x10,0x20)]
print('tables names',hex(no),nc,'hdrs',hex(ho),hc,'recs',hex(ro),rc)
for i in range(min(n,cnt)):
    nofs=struct.unpack_from('<Q',m,d+no+8*i)[0]
    e=m.find(b'\0',d+nofs); name=m[d+nofs:e].decode()
    h=m[d+ho+32*i:d+ho+32*i+32]
    r=m[d+ro+24*i:d+ro+24*i+24]
    lab,fmt,flags,w,hh,lv,hq,dep,bpp,q,sh,mem,pk=struct.unpack('<4sIIHHBBHHBBII',h)
    print(i,name,'hdr',h.hex(),'| fmt',fmt.to_bytes(4,'little'),hex(flags),w,hh,'lv',lv,'hq',hq,'dep',dep,'bpp',bpp,'q',q,'shift',sh,'mem',mem,'pk',pk)
    print('   rec',r.hex(),'off',struct.unpack_from('<I',r,12)[0],'packed',struct.unpack_from('<I',r,16)[0])
