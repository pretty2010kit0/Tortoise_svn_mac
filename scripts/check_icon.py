#!/usr/bin/env python3
"""快速校验生成的图标：尺寸、透明占比、主色、与参考项目是否不同（标准库 zlib）。"""
import struct
import sys
import zlib

def load_png(path):
    data = open(path, "rb").read()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "不是 PNG"
    pos = 8
    idat = b""
    w = h = None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos:pos + 4])
        tag = data[pos + 4:pos + 8]
        payload = data[pos + 8:pos + 8 + length]
        if tag == b"IHDR":
            w, h, bit, ctype = struct.unpack(">IIBB", payload[:10])
        elif tag == b"IDAT":
            idat += payload
        pos += 12 + length
    raw = zlib.decompress(idat)
    stride = w * 4 + 1
    rows = []
    for y in range(h):
        off = y * stride
        assert raw[off] == 0, "过滤器非 0"
        rows.append(raw[off + 1:off + 1 + w * 4])
    return w, h, rows

def main():
    path = sys.argv[1]
    ref = sys.argv[2] if len(sys.argv) > 2 and sys.argv[2] != "--preview" else None
    w, h, rows = load_png(path)
    print(f"尺寸: {w}x{h}")
    total = w * h
    transparent = sum(1 for r in rows for i in range(0, len(r), 4) if r[i + 3] < 16)
    opaque = total - transparent
    print(f"透明像素: {transparent / total * 100:.1f}%（不透明 {opaque / total * 100:.1f}%）")
    # 主色统计（不透明像素）
    from collections import Counter
    cnt = Counter()
    for r in rows:
        for i in range(0, len(r), 4):
            if r[i + 3] >= 16:
                cnt[(r[i] // 32, r[i + 1] // 32, r[i + 2] // 32)] += 1
    top = cnt.most_common(4)
    for (cr, cg, cb), n in top:
        print(f"  主色 ~({cr * 32 + 16}, {cg * 32 + 16}, {cb * 32 + 16}) 占比 {n / opaque * 100:.1f}%")
    if ref:
        w2, h2, rows2 = load_png(ref)
        same = w == w2 and h == h2 and all(
            rows[y] == rows2[y] for y in range(h)
        )
        print(f"与参考项目 {ref}: {'字节级相同（有问题！）' if same else '内容不同 ✅'}")

    # ASCII 缩略预览（亮度分级字符画）
    if "--preview" in sys.argv:
        chars = " .:-=+*#%@"
        cw, ch = 24, 48
        print("预览：")
        for y in range(0, h, ch):
            line = ""
            for x in range(0, w, cw):
                lum = 0.0
                cnt = 0
                for yy in range(y, min(y + ch, h), 12):
                    r0 = rows[yy]
                    for xx in range(x, min(x + cw, w), 12):
                        i = xx * 4
                        if r0[i + 3] > 16:
                            lum += 0.299 * r0[i] + 0.587 * r0[i + 1] + 0.114 * r0[i + 2]
                            cnt += 1
                if cnt == 0:
                    line += " "
                else:
                    line += chars[min(int(lum / cnt / 256 * len(chars)), len(chars) - 1)]
            print(line)

if __name__ == "__main__":
    main()
