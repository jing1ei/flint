#!/usr/bin/env python3
"""Rebuild Flint's pink-blue glass spark icon. Requires Pillow.

python3 scripts/make_icons.py --review --verify
The small Windows frames use BMP/DIB; larger frames and macOS use PNG.
"""
import argparse
import math
from pathlib import Path
import struct
from io import BytesIO
from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src-tauri/icons"
PNG_SIZES = [16, 32, 64, 128, 256, 512, 1024]
ICO_SIZES = [16, 32, 48, 64, 128, 256]
ICO_BMP_BELOW = 64
ICNS_TYPES = [(b"icp4",16),(b"icp5",32),(b"ic11",32),(b"ic12",64),
              (b"ic07",128),(b"ic13",256),(b"ic08",256),(b"ic14",512),
              (b"ic09",512),(b"ic10",1024)]

def curve(start, control1, control2, end, steps=48):
    return [((1-t)**3*start[0]+3*(1-t)**2*t*control1[0]+3*(1-t)*t*t*control2[0]+t**3*end[0],
             (1-t)**3*start[1]+3*(1-t)**2*t*control1[1]+3*(1-t)*t*t*control2[1]+t**3*end[1])
            for t in [i/steps for i in range(steps+1)]]


# A rising ember with a tapered, curved trail. No facet seams or lettering.
TIP=(0.695,0.257)
TAIL=(0.292,0.767)
LEFT=curve(TAIL,(0.35,0.49),(0.447,0.261),(0.61,0.259))+curve((0.61,0.259),(0.65,0.251),(0.679,0.25),TIP)
RIGHT=curve(TIP,(0.696,0.366),(0.601,0.44),(0.51,0.499))+curve((0.51,0.499),(0.40,0.58),(0.356,0.675),TAIL)
def slim(points):
    """Narrow perpendicular to the tail-to-tip axis without shortening the spark."""
    ax,ay=TAIL
    dx,dy=TIP[0]-ax,TIP[1]-ay
    length2=dx*dx+dy*dy
    result=[]
    for x,y in points:
        t=((x-ax)*dx+(y-ay)*dy)/length2
        px,py=ax+t*dx,ay+t*dy
        result.append((px+(x-px)*0.72,py+(y-py)*0.72))
    return result


LEFT=slim(LEFT)
RIGHT=slim(RIGHT)
SPARK=LEFT+list(reversed(RIGHT))


def color_field():
    """Continuous pastel gradient from pink through lilac to a luminous ice-blue tip."""
    size=256
    field=Image.new('RGBA',(size,size));pixels=field.load()
    stops=[(0.0,(241,155,210,140)),(.3,(250,182,220,206)),
           (.58,(210,193,247,215)),(.82,(164,223,255,236)),(1.0,(235,252,255,255))]
    for y in range(size):
        for x in range(size):
            u,v=x/(size-1),y/(size-1)
            t=max(0,min(1,((u-.29)*.4+(.767-v)*.51)/(.4*.4+.51*.51)))
            for (a,ca),(b,cb) in zip(stops,stops[1:]):
                if a<=t<=b:
                    f=(t-a)/(b-a);f=f*f*(3-2*f)
                    pixels[x,y]=tuple(round(c+(d-c)*f) for c,d in zip(ca,cb));break
    return field


COLOR_FIELD=color_field()


def render(size):
    scale=4 if size<=256 else 2
    n=size*scale
    image=Image.new('RGBA',(n,n))
    mask=Image.new('L',(n,n));points=[]
    margin=0.06 if size<=32 else 0.08
    radius=n*(0.5-margin)
    for i in range(360):
        t=2*math.pi*i/360
        points.append((n/2+radius*math.copysign(abs(math.cos(t))**0.4,math.cos(t)),
                       n/2+radius*math.copysign(abs(math.sin(t))**0.4,math.sin(t))))
    ImageDraw.Draw(mask).polygon(points,fill=255)
    image.paste((0,0,0,255),(0,0,n,n))
    image.putalpha(mask)
    def pts(values):return [(round(x*n),round(y*n)) for x,y in values]
    sm=Image.new('L',(n,n));ImageDraw.Draw(sm).polygon(pts(SPARK),fill=255)
    glass=COLOR_FIELD.resize((n,n),Image.Resampling.BICUBIC)
    glass.putalpha(ImageChops.multiply(glass.getchannel('A'),sm))
    # A small halo gives the ember light without washing out the black tile.
    halo=Image.new('RGBA',(n,n));d=ImageDraw.Draw(halo)
    d.ellipse((n*.51,n*.24,n*.72,n*.42),fill=(148,213,255,42))
    halo=halo.filter(ImageFilter.GaussianBlur(n*.047))
    halo.putalpha(ImageChops.multiply(halo.getchannel('A'),mask))
    image=Image.alpha_composite(image,halo)
    image=Image.alpha_composite(image,glass)
    reflection=Image.new('RGBA',(n,n));d=ImageDraw.Draw(reflection)
    highlight=slim(curve((.33,.68),(.415,.431),(.523,.292),(.674,.271)))
    d.line(pts(highlight),fill=(248,253,255,165),width=max(1,round(n*.014)))
    reflection=reflection.filter(ImageFilter.GaussianBlur(n*.009))
    reflection.putalpha(ImageChops.multiply(reflection.getchannel('A'),sm))
    image=Image.alpha_composite(image,reflection)
    edge=Image.new('RGBA',(n,n));d=ImageDraw.Draw(edge);w=max(1,round(n*.0018))
    d.line(pts(LEFT),fill=(255,219,245,160),width=w)
    d.line(pts(RIGHT[:49]),fill=(204,243,255,195),width=w)
    # Two secondary sparks indicate a strike, not a flame or leaf.
    if size>=32:
        d.polygon(pts([(.412,.245),(.424,.359),(.397,.303)]),fill=(251,197,230,225))
        d.polygon(pts([(.768,.486),(.652,.581),(.701,.518)]),fill=(172,224,255,215))
    image=Image.alpha_composite(image,edge)
    return image.resize((size,size),Image.Resampling.LANCZOS)


def png_bytes(img: Image.Image) -> bytes:
    buf = BytesIO()
    img.save(buf, format="PNG", optimize=True)
    return buf.getvalue()


def build_icns(art: dict[int, Image.Image]) -> bytes:
    """Pack the explicitly rendered sizes into a macOS ICNS container."""
    chunks = b""
    for ost, size in ICNS_TYPES:
        payload = png_bytes(art[size])
        chunks += ost + struct.pack(">I", len(payload) + 8) + payload
    return b"icns" + struct.pack(">I", len(chunks) + 8) + chunks


def dib_bytes(img: Image.Image) -> bytes:
    """32bpp bottom-up BITMAPINFOHEADER DIB + an all-zero AND mask, as .ico expects (the height in
    the header is doubled to cover both planes; alpha lives in the BGRA, the mask is vestigial)."""
    w, h = img.size
    px = img.convert("RGBA").load()
    rows = []
    for y in range(h - 1, -1, -1):  # DIBs are stored bottom-up
        rows.append(bytes(b for x in range(w) for b in (px[x, y][2], px[x, y][1], px[x, y][0], px[x, y][3])))
    and_mask = b"\x00" * ((((w + 31) // 32) * 4) * h)
    header = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    return header + b"".join(rows) + and_mask


def build_ico(art: dict[int, Image.Image]) -> bytes:
    payloads: list[tuple[int, bytes]] = []
    for size in ICO_SIZES:
        # Use the artwork rendered at each target size.
        img = art[size]
        payloads.append((size, dib_bytes(img) if size < ICO_BMP_BELOW else png_bytes(img)))

    offset = 6 + 16 * len(payloads)
    directory = b""
    for size, data in payloads:
        b = 0 if size == 256 else size  # 256 is encoded as 0 in a one-byte field
        directory += struct.pack("<BBBBHHII", b, b, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    return struct.pack("<HHH", 0, 1, len(payloads)) + directory + b"".join(d for _, d in payloads)


def write(art):
    OUT.mkdir(parents=True,exist_ok=True)
    for s in PNG_SIZES: art[s].save(OUT/f"{s}x{s}.png",optimize=True)
    art[256].save(OUT/'128x128@2x.png',optimize=True)
    art[1024].save(OUT/'icon.png',optimize=True)
    (OUT/'icon.icns').write_bytes(build_icns(art))
    (OUT/'icon.ico').write_bytes(build_ico(art))


def verify(art):
    for s in PNG_SIZES:
        with Image.open(OUT/f"{s}x{s}.png") as im:
            assert im.mode=='RGBA' and im.size==(s,s)
            assert im.tobytes()==art[s].tobytes()
            assert im.getpixel((0,0))[3]==0
            assert im.getpixel((s//2,s//2))[3]==255
    with Image.open(OUT/'icon.ico') as im:
        assert im.ico.sizes()=={(s,s) for s in ICO_SIZES}
        for s in ICO_SIZES:
            assert im.ico.getimage((s,s)).convert('RGBA').tobytes()==art[s].tobytes()
    raw=(OUT/'icon.icns').read_bytes()
    assert raw[:4]==b'icns' and struct.unpack('>I',raw[4:8])[0]==len(raw)
    offset=8
    for kind,size in ICNS_TYPES:
        assert raw[offset:offset+4]==kind
        length=struct.unpack('>I',raw[offset+4:offset+8])[0]
        with Image.open(BytesIO(raw[offset+8:offset+length])) as im:
            assert im.convert('RGBA').tobytes()==art[size].tobytes()
        offset+=length
    assert offset==len(raw)
    print('Verified PNG sizes, alpha, and every ICNS/ICO frame against the generated artwork.')


def review(art):
    path=ROOT/'icon-review';path.mkdir(exist_ok=True)
    sheet=Image.new('RGB',(1000,540),'#f1f0ec')
    d=ImageDraw.Draw(sheet);d.rectangle((500,0,1000,540),fill='#141517')
    for x in [0,500]:
        sheet.paste(art[256],(x+122,28),art[256])
        cursor=x+60
        for s in [16,32,48,64,128]:
            sheet.paste(art[s],(cursor,356-s//2),art[s]);cursor+=s+18
    sheet.save(path/'contact-sheet.png')
    print(path/'contact-sheet.png')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--review',action='store_true')
    parser.add_argument('--verify',action='store_true')
    args=parser.parse_args()
    art={s:render(s) for s in sorted(set(PNG_SIZES+ICO_SIZES))}
    write(art)
    if args.verify:verify(art)
    if args.review:review(art)
    print('Wrote Flint icons to',OUT)


if __name__=='__main__':main()
