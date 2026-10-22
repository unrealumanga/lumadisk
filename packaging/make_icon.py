#!/usr/bin/env python3
"""Deterministic 256px PNG icon. Mirrors app_icon() in src/main.rs (64x64 tiles, scaled 4x
nearest-neighbour). Stdlib only; fixed zlib level and no metadata chunks so output is byte-stable.
Usage: make_icon.py OUT.png"""
import struct
import sys
import zlib

SIZE, SCALE = 64, 4
BG = (10, 14, 20, 255)
TILES = [  # left, top, w, h, rgba  (keep in sync with src/main.rs)
    (7, 8, 31, 27, (44, 178, 140, 255)),
    (35, 8, 22, 27, (112, 91, 218, 255)),
    (7, 39, 19, 18, (218, 139, 55, 255)),
    (30, 39, 27, 18, (47, 147, 201, 255)),
]


def pixel(x, y):
    c = BG
    for left, top, w, h, col in TILES:
        if left <= x < left + w and top <= y < top + h:
            c = col
    return bytes(c)


def chunk(tag, data):
    body = tag + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


rows = bytearray()
n = SIZE * SCALE
for y in range(n):
    rows.append(0)  # filter: none
    for x in range(n):
        rows += pixel(x // SCALE, y // SCALE)
png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(bytes(rows), 9))
    + chunk(b"IEND", b"")
)
with open(sys.argv[1], "wb") as f:
    f.write(png)
