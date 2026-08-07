#!/usr/bin/env python3
"""生成 SVN 工具图标（1024x1024 RGBA PNG，纯标准库手写 PNG 编码）。

设计：深蓝渐变圆角方块 + 三个错位堆叠圆环（版本/提交历史意象），
最下方圆环用橙色点缀，与参考项目 Tortoise_git_mac 的 Git 图标区分。
用法：python3 scripts/make_icon.py [输出路径]
"""
import struct
import sys
import zlib

W = H = 1024

# —— 几何判定（带 2x2 超采样抗锯齿，alpha 混合）——

def in_round_rect(px, py, x0, y0, x1, y1, r):
    """点是否在圆角矩形内（含边界）"""
    if px < x0 or px > x1 or py < y0 or py > y1:
        return False
    # 角部：距离圆心判断
    cx = x0 + r if px < x0 + r else (x1 - r if px > x1 - r else px)
    cy = y0 + r if py < y0 + r else (y1 - r if py > y1 - r else py)
    if (px < x0 + r and py < y0 + r) or (px < x0 + r and py > y1 - r) or \
       (px > x1 - r and py < y0 + r) or (px > x1 - r and py > y1 - r):
        return (px - cx) ** 2 + (py - cy) ** 2 <= r * r
    return True

def in_ring(px, py, cx, cy, r_out, r_in):
    d2 = (px - cx) ** 2 + (py - cy) ** 2
    return r_in * r_in <= d2 <= r_out * r_out

def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))

def blend(dst, src):
    """src=(r,g,b,a 0-255) 覆盖到 dst=(r,g,b)"""
    a = src[3] / 255.0
    return tuple(int(dst[i] * (1 - a) + src[i] * a) for i in range(3))

# 圆环：白色 x2 + 橙色 x1
RINGS = [
    ((330, 322), 150, 84, (255, 255, 255, 246)),
    ((508, 500), 150, 84, (255, 255, 255, 246)),
    ((686, 678), 150, 84, (242, 163, 60, 252)),
]

def pixel(x, y):
    """返回该像素 RGBA（2x2 超采样）"""
    r = g = b = a = 0
    for dx in (0.0, 0.5):
        for dy in (0.0, 0.5):
            sx, sy = x + dx, y + dy
            if not in_round_rect(sx, sy, 28, 28, 996, 996, 200):
                continue
            # 背景渐变
            t = (sy - 28) / 968
            col = lerp((46, 95, 163), (22, 50, 92), t)
            # 圆环覆盖
            for ((cx, cy), ro, ri, c) in RINGS:
                if in_ring(sx, sy, cx, cy, ro, ri):
                    col = blend(col, c)
            r += col[0]; g += col[1]; b += col[2]; a += 255
        # endfor
    # endfor
    n = 4
    return (r // n, g // n, b // n, a // n)

def write_png(path, rows):
    def chunk(tag, data):
        c = struct.pack(">I", len(data)) + tag + data
        c += struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        return c

    raw = b"".join(b"\x00" + b"".join(struct.pack("4B", *p) for p in row) for row in rows)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)

def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "assets/icon.png"
    print(f"生成 {out} ({W}x{H})…")
    rows = []
    for y in range(H):
        rows.append([pixel(x, y) for x in range(W)])
        if (y + 1) % 128 == 0:
            print(f"  {y + 1}/{H}")
    write_png(out, rows)
    print("完成。")

if __name__ == "__main__":
    main()
