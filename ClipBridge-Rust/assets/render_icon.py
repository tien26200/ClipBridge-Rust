"""Build-time artwork only; Pillow is not needed by the application."""
from pathlib import Path
from PIL import Image, ImageDraw

root=Path(__file__).parent
size=768
scale=size/128
def pt(p): return tuple(round(v*scale) for v in p)
image=Image.new('RGBA',(size,size))
pixels=image.load()
for y in range(size):
    for x in range(size):
        t=(x/size*0.63+(1-y/size)*0.37)
        a=(117,62,230); b=(28,208,188)
        pixels[x,y]=tuple(round(a[i]*(1-t)+b[i]*t) for i in range(3))+(255,)
mask=Image.new('L',(size,size));ImageDraw.Draw(mask).rounded_rectangle(pt((3,3,125,125)),radius=round(30*scale),fill=255)
image.putalpha(mask)
d=ImageDraw.Draw(image)
# Rear purple card, foreground photograph and a mint "send/copy" arrow.
d.rounded_rectangle(pt((20,31,91,102)),radius=round(13*scale),fill='#322667')
d.rounded_rectangle(pt((28,22,102,94)),radius=round(12*scale),fill='#f5fffe')
d.rounded_rectangle(pt((35,29,95,80)),radius=round(7*scale),fill='#233247')
d.ellipse(pt((72,37,84,49)),fill='#72f2d2')
d.polygon([pt(p) for p in [(36,76),(55,52),(71,70),(82,59),(94,76)]],fill='#a999ff')
d.line([pt((41,86)),pt((66,86))],fill='#cbc8e0',width=round(4*scale))
d.rounded_rectangle(pt((69,72,113,115)),radius=round(13*scale),fill='#202b45')
d.line([pt((80,104)),pt((101,83))],fill='#73ffda',width=round(7*scale))
d.line([pt((82,83)),pt((101,83)),pt((101,102))],fill='#73ffda',width=round(7*scale))
d.polygon([pt(p) for p in [(104,13),(107,20),(114,23),(107,26),(104,33),(101,26),(94,23),(101,20)]],fill='#ffffff')
image.resize((512,512),Image.Resampling.LANCZOS).save(root/'icon-preview.png')
image.save(root/'app.ico',format='ICO',sizes=[(s,s) for s in (16,20,24,32,40,48,64,128,256)])
(root/'app.svg').write_text('''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
<defs><linearGradient id="g" x1="0" y1="1" x2="1" y2="0"><stop stop-color="#753ee6"/><stop offset="1" stop-color="#1cd0bc"/></linearGradient></defs>
<rect x="3" y="3" width="122" height="122" rx="30" fill="url(#g)"/>
<rect x="20" y="31" width="71" height="71" rx="13" fill="#322667"/>
<rect x="28" y="22" width="74" height="72" rx="12" fill="#f5fffe"/>
<rect x="35" y="29" width="60" height="51" rx="7" fill="#233247"/>
<circle cx="78" cy="43" r="6" fill="#72f2d2"/>
<path d="M36 76L55 52L71 70L82 59L94 76Z" fill="#a999ff"/>
<path d="M41 86H66" stroke="#cbc8e0" stroke-width="4"/>
<rect x="69" y="72" width="44" height="43" rx="13" fill="#202b45"/>
<path d="M80 104L101 83M82 83H101V102" fill="none" stroke="#73ffda" stroke-width="7"/>
<path d="M104 13L107 20L114 23L107 26L104 33L101 26L94 23L101 20Z" fill="white"/>
</svg>''',encoding='utf-8')
print('Built app.ico with nine sizes and an editable SVG')
