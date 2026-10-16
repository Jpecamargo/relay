"""Gera os mestres do ícone a partir de assets/icon-source.png.

macOS: grade da Apple — corpo 824×824 centrado em 1024, recorte squircle (superelipse n=5, que
aproxima o "continuous corner" do macOS) e sombra suave. O PNG de origem tem cantos mais redondos
que o padrão e um halo semitransparente; recortamos o corpo opaco 26 px para dentro, o mínimo
para ele cobrir o squircle inteiro sem deixar transparência nos cantos.
Windows: quase sangrado (984 em 1024), mesmo recorte.

Uso: python3 scripts/make-icons.py assets/icon-source.png assets/icon-mac.png assets/icon-win.png
"""
import sys

import numpy as np
from PIL import Image, ImageFilter

BODY = (180, 178, 1070, 1068)  # corpo opaco do squircle no PNG de origem (alpha > 200)
CROP = 26


def squircle(size: int, n: float = 5.0, ss: int = 4) -> Image.Image:
    big = size * ss
    yy, xx = np.mgrid[0:big, 0:big]
    u = np.abs((xx + 0.5) / big * 2 - 1)
    v = np.abs((yy + 0.5) / big * 2 - 1)
    m = ((u**n + v**n) <= 1).astype(np.uint8) * 255
    return Image.fromarray(m).resize((size, size), Image.LANCZOS)


def make(src: Image.Image, canvas: int, art: int, shadow: bool, out: str) -> None:
    x0, y0, x1, y1 = BODY
    body = src.crop((x0 + CROP, y0 + CROP, x1 - CROP, y1 - CROP)).resize((art, art), Image.LANCZOS)
    body.putalpha(squircle(art))
    c = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    off = (canvas - art) // 2
    if shadow:
        sh = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
        sil = Image.new("RGBA", (art, art), (0, 0, 0, 90))
        sil.putalpha(squircle(art).point(lambda v: v * 90 // 255))
        sh.alpha_composite(sil, (off, off + 10))
        c.alpha_composite(sh.filter(ImageFilter.GaussianBlur(14)))
    c.alpha_composite(body, (off, off))
    c.save(out)


src = Image.open(sys.argv[1]).convert("RGBA")
make(src, 1024, 824, True, sys.argv[2])
make(src, 1024, 984, False, sys.argv[3])
