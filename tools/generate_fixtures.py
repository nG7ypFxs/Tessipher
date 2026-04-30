#!/usr/bin/env python3
"""Generate immutable independent vectors with vendored Nayuki (MIT), no pip required."""
from pathlib import Path
import sys, struct, hashlib
sys.path.insert(0, str(Path(__file__).parent/'vendor'))
from qrcodegen import QrCode, QrSegment
ROOT=Path(__file__).resolve().parents[1]
def packed(q):
    n=q.get_size(); data=bytearray((n*n+7)//8)
    for y in range(n):
        for x in range(n):
            if q.get_module(x,y):
                i=y*n+x;data[i//8]|=128>>(i%8)
    return data
levels=[QrCode.Ecc.LOW,QrCode.Ecc.MEDIUM,QrCode.Ecc.QUARTILE,QrCode.Ecc.HIGH]
corpus=bytearray(b'TQR1')
for v in range(1,41):
    for ec,level in enumerate(levels):
        for mask in range(8):
            message=f'{v:02}{"LMQH"[ec]}{mask}'.encode()
            q=QrCode.encode_segments([QrSegment.make_bytes(message)],level,v,v,mask,False)
            bits=packed(q)
            corpus.extend(struct.pack('<BBBH',v,ec,mask,len(bits)));corpus.extend(bits)
(ROOT/'fixtures/all-model2.bin').write_bytes(corpus)
for v in [1,2,7,14,32,40]:
    q=QrCode.encode_segments([QrSegment.make_bytes(b'TEST')],QrCode.Ecc.HIGH,v,v,3,False)
    (ROOT/f'fixtures/v{v}.bin').write_bytes(packed(q))
    scale=3 if v>=32 else 5
    n=q.get_size();w=(n+8)*scale
    pgm=bytearray(f'P5\n{w} {w}\n255\n'.encode())
    pgm.extend(0 if 4<=x//scale<n+4 and 4<=y//scale<n+4 and q.get_module(x//scale-4,y//scale-4) else 255 for y in range(w) for x in range(w))
    (ROOT/f'fixtures/v{v}.pgm').write_bytes(pgm)
print(f'1280 vectors, {len(corpus)} bytes, sha256 {hashlib.sha256(corpus).hexdigest()}')
# Explicit segment/control vectors and maximum legal payloads.
def control(mode, bits): return QrSegment(QrSegment.Mode(mode,(0,0,0)),0,bits)
def ub(v,n): return [(v>>i)&1 for i in reversed(range(n))]
sa=control(3,ub(0,4)+ub(1,4)+ub(0xa7,8))
fnc=control(9,ub(42,8))
kanji=QrSegment(QrSegment.Mode.KANJI,2,ub(1855,13)+ub(2586,13)) # 漢字: 8ABF 8E9A
segments=[sa,fnc,QrSegment.make_eci(26),QrSegment.make_numeric('0012345'),QrSegment.make_alphanumeric('A%B%%C'),QrSegment.make_bytes(bytes([0,255,37])),kanji]
q=QrCode.encode_segments(segments,QrCode.Ecc.QUARTILE,5,5,6,False)
(ROOT/'fixtures/modes.bin').write_bytes(packed(q))
# FNC1 can follow ECI and optional leading Structured Append before data.
for second in [False, True]:
    for append in [False, True]:
        controls = ([sa] if append else []) + [QrSegment.make_eci(26), control(9,ub(42,8)) if second else control(5,[])]
        data = [QrSegment.make_alphanumeric('A%B%%C'), QrSegment.make_bytes(bytes([37,255]))]
        q=QrCode.encode_segments(controls+data,QrCode.Ecc.LOW,1,1,2,False)
        name=f'eci-fnc{2 if second else 1}{"-sa" if append else ""}'
        (ROOT/f'fixtures/{name}.bin').write_bytes(packed(q))

for name,segment in [('numeric-max',QrSegment.make_numeric('1'*7089)),('byte-max',QrSegment.make_bytes(bytes(i%256 for i in range(2953)))),('kanji-max',QrSegment(QrSegment.Mode.KANJI,1817,ub(1855,13)*1817))]:
    q=QrCode.encode_segments([segment],QrCode.Ecc.LOW,40,40,2,False)
    (ROOT/f'fixtures/{name}.bin').write_bytes(packed(q))
# Warps are inverse mapped without image libraries; matrices come from the independent encoder.
def solve(a):
    n=len(a)
    for i in range(n):
        pivot=max(range(i,n),key=lambda k:abs(a[k][i]));a[i],a[pivot]=a[pivot],a[i]
        d=a[i][i];a[i]=[x/d for x in a[i]]
        for k in range(n):
            if k!=i:
                d=a[k][i];a[k]=[x-d*y for x,y in zip(a[k],a[i])]
    return [row[-1] for row in a]
def warp(name,version,quad,lighting=False):
    q=QrCode.encode_segments([QrSegment.make_bytes(b'TEST')],QrCode.Ecc.HIGH,version,version,3,False)
    n=q.get_size();source=[(0,0),(n,0),(0,n),(n,n)];a=[]
    for (x,y),(u,v) in zip(quad,source):
        a.extend([[x,y,1,0,0,0,-u*x,-u*y,u],[0,0,0,x,y,1,-v*x,-v*y,v]])
    h=solve(a);w=360;pixels=bytearray()
    for y in range(w):
        for x in range(w):
            xx=x+.5;yy=y+.5;d=h[6]*xx+h[7]*yy+1
            u=(h[0]*xx+h[1]*yy+h[2])/d;v=(h[3]*xx+h[4]*yy+h[5])/d
            dark=0<=u<n and 0<=v<n and q.get_module(int(u),int(v))
            value=25 if dark else 230
            if lighting:value=int((35 if dark else 130)+x*.30+y*.04)
            pixels.append(min(255,value))
    (ROOT/f'fixtures/{name}.pgm').write_bytes(f'P5\n{w} {w}\n255\n'.encode()+pixels)
warp('rotated-v2',2,[(100,30),(330,100),(30,260),(260,330)])
warp('perspective-v1',1,[(45,35),(280,55),(55,285),(305,300)])
warp('perspective-v7',7,[(40,35),(290,50),(65,310),(315,285)])
warp('lighting-v2',2,[(50,50),(300,50),(50,300),(300,300)],True)
warp('rotated45-v2',2,[(180,20),(340,180),(20,180),(180,340)])
warp('perspective-strong-v1',1,[(45,35),(285,65),(55,290),(325,320)])
