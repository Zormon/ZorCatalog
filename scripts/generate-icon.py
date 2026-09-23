#!/usr/bin/env python3
"""Genera app-icon.png (1024x1024) para ZorCatalog sin dependencias externas.

Uso:
    python3 scripts/generate-icon.py
    pnpm tauri icon app-icon.png
"""
import math
import struct
import zlib

SIZE = 1024
CENTER = (SIZE - 1) / 2.0

# Degradado indigo -> violeta (esquina superior izquierda -> inferior derecha).
GRAD_A = (79, 70, 229)
GRAD_B = (124, 58, 237)


def rounded_rect_sdf(px, py, cx, cy, hw, hh, radius):
    """Distancia con signo a un rectangulo redondeado (negativo = dentro)."""
    qx = abs(px - cx) - (hw - radius)
    qy = abs(py - cy) - (hh - radius)
    outside = math.hypot(max(qx, 0.0), max(qy, 0.0))
    inside = min(max(qx, qy), 0.0)
    return outside + inside - radius


def coverage(sdf):
    """Convierte una distancia con signo en cobertura con antialiasing."""
    return min(max(0.5 - sdf, 0.0), 1.0)


def composite(dst, src, alpha):
    """Alpha compositing (straight alpha) de src sobre dst."""
    if alpha <= 0.0:
        return dst
    dr, dg, db, da = dst
    sr, sg, sb = src
    out_a = alpha + da * (1.0 - alpha)
    if out_a <= 0.0:
        return (0.0, 0.0, 0.0, 0.0)
    out_r = (sr * alpha + dr * da * (1.0 - alpha)) / out_a
    out_g = (sg * alpha + dg * da * (1.0 - alpha)) / out_a
    out_b = (sb * alpha + db * da * (1.0 - alpha)) / out_a
    return (out_r, out_g, out_b, out_a)


# Tarjetas del "catalogo": rejilla 2x2.
CARD_HW = 150.0
CARD_RADIUS = 64.0
CARD_OFFSET = 180.0
CARDS = [
    (CENTER - CARD_OFFSET, CENTER - CARD_OFFSET),
    (CENTER + CARD_OFFSET, CENTER - CARD_OFFSET),
    (CENTER - CARD_OFFSET, CENTER + CARD_OFFSET),
    (CENTER + CARD_OFFSET, CENTER + CARD_OFFSET),
]


def render():
    rows = []
    for y in range(SIZE):
        row = bytearray()
        for x in range(SIZE):
            # Fondo: cuadrado redondeado con degradado diagonal.
            bg_a = coverage(rounded_rect_sdf(x, y, CENTER, CENTER, 512.0, 512.0, 224.0))
            t = (x + y) / (2.0 * (SIZE - 1))
            color = (
                GRAD_A[0] + (GRAD_B[0] - GRAD_A[0]) * t,
                GRAD_A[1] + (GRAD_B[1] - GRAD_A[1]) * t,
                GRAD_A[2] + (GRAD_B[2] - GRAD_A[2]) * t,
            )
            px = composite((0.0, 0.0, 0.0, 0.0), color, bg_a)

            # Tarjetas blancas encima.
            for cx, cy in CARDS:
                d = rounded_rect_sdf(x, y, cx, cy, CARD_HW, CARD_HW, CARD_RADIUS)
                a = coverage(d) * 0.93
                if a > 0.0:
                    px = composite(px, (255.0, 255.0, 255.0), a)

            row += bytes(
                (
                    int(px[0] + 0.5),
                    int(px[1] + 0.5),
                    int(px[2] + 0.5),
                    int(px[3] * 255.0 + 0.5),
                )
            )
        rows.append(row)
    return rows


def write_png(path, rows):
    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = b"".join(b"\x00" + bytes(r) for r in rows)
    ihdr = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as fh:
        fh.write(png)


if __name__ == "__main__":
    write_png("app-icon.png", render())
    print("app-icon.png generado (%dx%d)" % (SIZE, SIZE))
