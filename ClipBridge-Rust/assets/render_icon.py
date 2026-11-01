"""Optional local preview renderer. The release EXE icon is generated directly by build.rs."""
from pathlib import Path
from PIL import Image
from math import atan2, hypot, pi

root = Path(__file__).resolve().parent
size = 1024
radius = size * 0.463
colours = [(248, 48, 107), (27, 117, 244), (255, 194, 35)]

image = Image.new("RGBA", (size, size), (0, 0, 0, 0))
pixels = image.load()
for y in range(size):
    dy = (y + 0.5) - size / 2
    for x in range(size):
        dx = (x + 0.5) - size / 2
        r = hypot(dx, dy)
        if r >= radius + 0.5:
            continue
        theta = (atan2(dy, dx) + pi / 2 + 0.46 * (1 - r / radius)) % (2 * pi)
        colour = colours[min(2, int(theta / (2 * pi / 3)))]
        opacity = round(255 * max(0, min(1, radius - r + 0.5)))
        pixels[x, y] = (*colour, opacity)

image.save(root / "icon-preview.png")
image.save(root / "app.ico", format="ICO",
           sizes=[(n, n) for n in (16, 20, 24, 32, 40, 48, 64, 128, 256)])
print("Rendered the three-colour circular icon; release builds use build.rs.")
