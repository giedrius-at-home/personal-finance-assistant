"""Draw the Finance app icon (1024 px PNG): glassy squircle with rising bars."""
import sys
from PIL import Image, ImageDraw, ImageFilter

S = 1024
out = sys.argv[1]

def squircle_mask(size, inset, radius):
    m = Image.new("L", (size, size), 0)
    ImageDraw.Draw(m).rounded_rectangle((inset, inset, size - inset, size - inset), radius, fill=255)
    return m

# Background gradient: deep teal to indigo.
bg = Image.new("RGBA", (S, S))
top, bottom = (24, 120, 112), (38, 52, 120)
px = bg.load()
for y in range(S):
    t = y / (S - 1)
    for x in range(S):
        u = (x / (S - 1)) * 0.25
        k = min(1, t + u)
        px[x, y] = tuple(int(top[i] + (bottom[i] - top[i]) * k) for i in range(3)) + (255,)

icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
mask = squircle_mask(S, 100, 185)
icon.paste(bg, (0, 0), mask)

# Rising bars, white with slight transparency.
layer = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(layer)
bars = [(300, 600), (420, 500), (540, 410), (660, 300)]
w = 90
for x, top_y in bars:
    d.rounded_rectangle((x, top_y, x + w, 760), 26, fill=(255, 255, 255, 235))
# Baseline.
d.rounded_rectangle((270, 790, 780, 812), 11, fill=(255, 255, 255, 170))
shadow = layer.copy().filter(ImageFilter.GaussianBlur(18))
shadow_alpha = shadow.split()[3].point(lambda a: a * 0.35)
shadow = Image.merge("RGBA", (*[Image.new("L", (S, S), 0)] * 3, shadow_alpha))
icon.alpha_composite(shadow, (0, 14))
icon.alpha_composite(layer)

# Glass highlight across the top half.
gloss = Image.new("RGBA", (S, S), (0, 0, 0, 0))
gd = ImageDraw.Draw(gloss)
gd.ellipse((-200, -620, S + 200, 470), fill=(255, 255, 255, 48))
gloss_mask = Image.composite(gloss, Image.new("RGBA", (S, S), (0, 0, 0, 0)), mask)
icon.alpha_composite(gloss_mask)

# Thin inner rim.
rim = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(rim).rounded_rectangle((102, 102, S - 102, S - 102), 183, outline=(255, 255, 255, 70), width=4)
icon.alpha_composite(rim)
icon.save(out)
