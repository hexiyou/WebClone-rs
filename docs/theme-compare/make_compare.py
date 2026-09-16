"""把改造前/后的两张主界面截图拼成一张对比图，附局部放大。

布局：
  上半区 —— 两张主界面整图左右并排
  下半区 —— 若干局部区域，每个区域内"上=改造前、下=改造后"纵向对照，区域之间横向并排
"""
from PIL import Image, ImageDraw, ImageFont

FONT = "C:/Windows/Fonts/msyh.ttc"
V = "C:/Users/Administrator/WorkBuddy/workBuddy-Space/webclone-rs/_viztest/"

before = Image.open(V + "shot-before.png").convert("RGB")
after = Image.open(V + "shot-after.png").convert("RGB")
W, H = before.size

f_title = ImageFont.truetype(FONT, 25)
f_sub = ImageFont.truetype(FONT, 17)
f_tag = ImageFont.truetype(FONT, 16)
f_label = ImageFont.truetype(FONT, 18)

MARGIN = 26
GAP = 22
TITLE_H = 44
RED, GREEN = "#B3261E", "#1E7B34"

crops = [
    ("按钮", (25, 530, 300, 592)),
    ("复选框 / 单选框", (405, 296, 660, 450)),
    ("页签与标题", (28, 33, 210, 128)),
]
SCALE = 2.2
PAIR_GAP = 8  # 每个 crop 内 before/after 之间的间距

# 先算出下半区尺寸
crop_boxes = []
for label, box in crops:
    w, h = box[2] - box[0], box[3] - box[1]
    sw, sh = int(w * SCALE), int(h * SCALE)
    crop_boxes.append((label, box, sw, sh))

band_h = max(sh * 2 + PAIR_GAP for _, _, _, sh in crop_boxes)
band_w = sum(sw for _, _, sw, _ in crop_boxes) + GAP * (len(crop_boxes) - 1)

canvas_w = max(MARGIN * 2 + W * 2 + GAP, MARGIN * 2 + band_w)
canvas_h = MARGIN + TITLE_H + H + 54 + 34 + band_h + MARGIN

canvas = Image.new("RGB", (canvas_w, canvas_h), "#FFFFFF")
d = ImageDraw.Draw(canvas)

# ---------------- 上半区：整图并排 ----------------
for i, (img, title, note) in enumerate([
    (before, "改造前", "无 manifest —— 原生控件走经典渲染路径"),
    (after, "改造后", "+ comctl32 v6 manifest（只增 1.5 KB，0 行业务代码）"),
]):
    x = MARGIN + i * (W + GAP)
    y = MARGIN
    color = RED if i == 0 else GREEN
    d.text((x, y), title, font=f_title, fill=color)
    d.text((x + d.textlength(title, font=f_title) + 12, y + 8), note, font=f_sub, fill="#5F6368")
    canvas.paste(img, (x, y + TITLE_H))
    d.rectangle([x - 1, y + TITLE_H - 1, x + W, y + TITLE_H + H], outline="#DADCE0", width=1)

# ---------------- 下半区：局部放大 ----------------
band_y = MARGIN + TITLE_H + H + 54
d.text((MARGIN, band_y - 32), "局部放大", font=f_title, fill="#202124")

cx = MARGIN
for label, box, sw, sh in crop_boxes:
    d.text((cx, band_y), label, font=f_label, fill="#202124")

    pair_h = sh * 2 + PAIR_GAP
    top = band_y + 28 + (band_h - pair_h) // 2  # 组内垂直居中，避免矮的 crop 偏上

    for i, img in enumerate([before, after]):
        py = top + i * (sh + PAIR_GAP)
        piece = img.crop(box).resize((sw, sh), Image.LANCZOS)
        canvas.paste(piece, (cx, py))
        d.rectangle([cx - 1, py - 1, cx + sw, py + sh],
                    outline=RED if i == 0 else GREEN, width=2)
        d.text((cx + 4, py + 3), "前" if i == 0 else "后",
               font=f_tag, fill="#FFFFFF",
               stroke_width=2, stroke_fill=RED if i == 0 else GREEN)

    cx += sw + GAP

canvas.save(V + "compare-before-after.png")
print("saved:", canvas.size)
