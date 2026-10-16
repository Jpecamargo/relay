import sys
from PIL import Image, ImageFilter
src = Image.open(sys.argv[1]).convert('RGBA')
a = src.getchannel('A')
# máscara: só o corpo opaco, sem o halo/sombra semitransparente da borda
m = a.point(lambda v: 255 if v >= 250 else 0)
m = m.filter(ImageFilter.MaxFilter(9)).filter(ImageFilter.MinFilter(9))  # fecha furos
m = m.filter(ImageFilter.MinFilter(13))                                   # recua ~6px da franja
m = m.filter(ImageFilter.GaussianBlur(1.6))                               # antialias
body = src.copy(); body.putalpha(m)
bb = m.point(lambda v: 255 if v > 8 else 0).getbbox()
body = body.crop(bb)
side = max(body.size)
sq = Image.new('RGBA', (side, side), (0,0,0,0)); sq.alpha_composite(body, ((side-body.size[0])//2, (side-body.size[1])//2))
def make(canvas, art, out):
    c = Image.new('RGBA', (canvas, canvas), (0,0,0,0))
    off = (canvas-art)//2
    c.alpha_composite(sq.resize((art, art), Image.LANCZOS), (off, off))
    c.save(out)
make(1024, 824, sys.argv[2])  # macOS: grade da Apple (824 em 1024)
make(1024, 984, sys.argv[3])  # Windows: quase sangrado
