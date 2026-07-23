#!/usr/bin/env python3
"""Rend docs/assets/demo.gif — animation terminal de la démo trapdoor, 100% local.

Aucune dépendance réseau : Pillow + la police Consolas de Windows. La sortie
reproduit exactement celle, vérifiée, du binaire réel (watch $count = 2, puis
"processed 41 fruits, total=574"). Chaque frame est rendue à la main, donc le
résultat est déterministe et ne peut pas se figer comme un enregistrement live.

Usage :  python docs/assets/render_gif.py
Sortie :  docs/assets/demo.gif  (+ docs/assets/_frame_test.png pour contrôle)
"""
import os
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
OUT_GIF = os.path.join(HERE, "demo.gif")
OUT_TEST = os.path.join(HERE, "_frame_test.png")

# --- Thème (Tokyo Night) ---
BG      = (26, 27, 38)      # #1a1b26
FG      = (169, 177, 214)   # texte par défaut
PROMPT  = (86, 95, 137)     # invites "$ " et "(tdb) "
CMD     = (192, 202, 245)   # commandes tapées (clair)
GREEN   = (158, 206, 106)   # sortie du script / ligne de succès
YELLOW  = (224, 175, 104)   # breakpoint
CYAN    = (125, 207, 255)   # emplacements file:line
DIM     = (86, 95, 137)     # méta (exit status)

FONT_SIZE = 22
PAD = 18
font  = ImageFont.truetype("C:/Windows/Fonts/consola.ttf", FONT_SIZE)
fontb = ImageFont.truetype("C:/Windows/Fonts/consolab.ttf", FONT_SIZE)
CELL_W = round(font.getlength("M"))
ASC, DESC = font.getmetrics()
LINE_H = ASC + DESC + 4

# ------------------------------------------------------------------ terminal
screen = [[]]          # liste de lignes ; chaque ligne = liste de (char, color, bold)
cur = [0, 0]           # (row, col)
frames = []            # liste de (snapshot, duree_ms)

def _ensure(row, col):
    while len(screen) <= row:
        screen.append([])
    line = screen[row]
    while len(line) <= col:
        line.append((" ", FG, False))

def write(text, color=FG, bold=False):
    for ch in text:
        if ch == "\n":
            cur[0] += 1; cur[1] = 0; _ensure(cur[0], 0)
        elif ch == "\r":
            cur[1] = 0
        else:
            _ensure(cur[0], cur[1])
            screen[cur[0]][cur[1]] = (ch, color, bold)
            cur[1] += 1

def snapshot(dur_ms):
    frames.append(([list(l) for l in screen], int(dur_ms)))

def hold(ms):
    if frames:
        snap, d = frames[-1]
        frames[-1] = (snap, d + int(ms))

def emit(text, color=FG, bold=False, dur=90):
    write(text, color, bold)
    snapshot(dur)

def typ(text, color=CMD, cps=26, after=220):
    for ch in text:
        write(ch, color, True)
        snapshot(1000 / cps)
    write("\r\n")
    snapshot(after)

# ------------------------------------------------------------------ scénario
emit("$ ", PROMPT); hold(300)
typ("trapdoor -b 'demo.sh:13 if (( count == 1 ))' -r examples/demo.sh", after=350)

emit("breakpoint #1 at demo.sh:13  if (( count == 1 ))\n", YELLOW, dur=350)
emit("hello, apple\n\n", GREEN, dur=220)
emit("examples/demo.sh:13 [depth 1] ", CYAN, dur=60)
emit("● breakpoint #1\n", YELLOW, dur=120)
emit("  → count=$((count + 1))\n", FG, dur=80)
emit("(tdb) ", PROMPT, dur=60)

typ("p count fruit", after=200)
emit('declare -- count="1"\n', FG, dur=260)
emit('declare -- fruit="banana"\n', FG, dur=40)
emit("(tdb) ", PROMPT, dur=60)

typ("w $count", after=200)
emit("watch #1: $count\n", FG, dur=220)
emit("(tdb) ", PROMPT, dur=60)

typ("s", after=200)
emit("examples/demo.sh:14 [depth 1]\n", CYAN, dur=320)
emit('  → greet "$fruit"\n', FG, dur=50)
emit("  watch #1: $count = 2\n", YELLOW, dur=60)
emit("(tdb) ", PROMPT, dur=60)

typ("!count=40", after=320)
emit("(tdb) ", PROMPT, dur=180)

typ("c", after=260)
emit("hello, banana\n", GREEN, dur=280)
emit("hello, cherry\n", GREEN, dur=130)
emit("processed 41 fruits, total=574\n\n", GREEN, bold=True, dur=140)
emit("trapdoor: script exited with status 0\n", DIM, dur=160)
emit("$ ", PROMPT, dur=60); hold(2000)

# ------------------------------------------------------------------ rendu
ncols = max(len(l) for snap, _ in frames for l in snap) if frames else 1
nrows = max(len(snap) for snap, _ in frames)
W = PAD * 2 + ncols * CELL_W
H = PAD * 2 + nrows * LINE_H

def render(snap):
    img = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(img)
    for r, line in enumerate(snap):
        y = PAD + r * LINE_H
        for c, (ch, color, bold) in enumerate(line):
            if ch == " ":
                continue
            d.text((PAD + c * CELL_W, y), ch, font=(fontb if bold else font), fill=color)
    return img

imgs = [render(snap) for snap, _ in frames]
durs = [d for _, d in frames]

# frame de contrôle : celle qui contient ● et → (juste après le 1er arrêt)
test_idx = next((i for i, (snap, _) in enumerate(frames)
                 if any("●" in "".join(ch for ch, *_ in l) for l in snap)), len(frames) // 2)
imgs[test_idx].save(OUT_TEST)

imgs[0].save(OUT_GIF, save_all=True, append_images=imgs[1:],
             duration=durs, loop=0, optimize=True, disposal=2)

kb = os.path.getsize(OUT_GIF) / 1024
print(f"OK -> {OUT_GIF}  ({kb:.0f} KB, {len(imgs)} frames, {W}x{H})")
print(f"frame de contrôle -> {OUT_TEST} (index {test_idx})")
