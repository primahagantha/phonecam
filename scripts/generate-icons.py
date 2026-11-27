#!/usr/bin/env python3
"""
PhoneCam Precision Optical Studio Icon & Asset Generator.

Mathematically renders 4x-supersampled antialiased brand assets across Desktop
(SVG, 1024px PNG, 512px Play Store PNG, multi-size Windows ICO), Android
(Adaptive VectorDrawables, Material You Monochrome VectorDrawable, legacy
mipmap PNGs), and a visual audit contact sheet.

Zero hardcoded personal paths; uses repository-relative paths and system TEMP.
"""

from __future__ import annotations

import argparse
import io
import math
import struct
import tempfile
from pathlib import Path
from typing import Tuple

import numpy as np
from PIL import Image, ImageDraw, ImageFont

REPO_ROOT = Path(__file__).resolve().parent.parent


def smoothstep(edge0: float, edge1: float, x: np.ndarray) -> np.ndarray:
    t = np.clip((x - edge0) / (edge1 - edge0), 0.0, 1.0)
    return t * t * (3.0 - 2.0 * t)


def mix_rgb(
    base: np.ndarray, overlay: np.ndarray, alpha: np.ndarray
) -> np.ndarray:
    a = np.clip(alpha, 0.0, 1.0)[..., None]
    return base * (1.0 - a) + overlay * a


def hex_rgb(hex_color: str) -> np.ndarray:
    h = hex_color.lstrip("#")
    return np.array(
        [int(h[0:2], 16) / 255.0, int(h[2:4], 16) / 255.0, int(h[4:6], 16) / 255.0],
        dtype=np.float32,
    )


def render_icon(
    size: int,
    *,
    mode: str = "squircle",
    supersample: int = 4,
    small_optical_boost: float = 0.0,
) -> Image.Image:
    """
    Render the PhoneCam optical studio mark at `size x size`.

    mode:
      - "squircle": transparent background with studio obsidian squircle & titanium rim
      - "full_bleed": opaque full-bleed square (Google Play Store 512x512)
      - "round": circular cropped legacy Android launcher icon
      - "adaptive_preview": 108dp-proportioned full-bleed for Android mask preview
    """
    ss = supersample if size <= 512 else 2
    res = size * ss
    coords = (np.arange(res, dtype=np.float32) + 0.5) / res * 2.0 - 1.0
    xx, yy = np.meshgrid(coords, coords)
    px = 2.0 / res

    boost = float(np.clip(small_optical_boost, 0.0, 1.0))

    if mode == "adaptive_preview":
        # Matches Android 108dp viewport (safe circle radius 33/54 = 0.611).
        # Outer barrel r = 22.0dp (0.407), tally center (72.0, 36.0) -> (0.333, -0.333), r = 2.7dp (0.050).
        # Furthest tally edge = 28.2dp -> 4.8dp clear margin inside the 33dp safe circle!
        lens_scale = 0.562
        tally_cx, tally_cy = 0.333, -0.333
        tally_r = 0.050
    elif mode == "full_bleed":
        # Commanding presence inside Google Play Store's rounded-square mask
        lens_scale = 0.875
        tally_cx, tally_cy = 0.565, -0.565
        tally_r = 0.060
    elif mode == "round":
        lens_scale = 0.78
        tally_cx, tally_cy = 0.480, -0.480
        tally_r = 0.058
    else:
        # Desktop squircle (1024px -> 16px)
        lens_scale = 0.865 + 0.095 * boost
        # Keep tally sleek at hero sizes and cleanly inside the 16px corner (never colliding with rim)
        tally_cx = 0.565 - 0.030 * boost
        tally_cy = -0.565 + 0.030 * boost
        tally_r = 0.056 + 0.058 * boost

    # --- 1. Studio Obsidian Background Field ---
    grad_t = np.clip((yy + 1.0) * 0.5, 0.0, 1.0)
    bg_top = hex_rgb("#192336") * (1.0 + 0.14 * boost)
    bg_mid = hex_rgb("#0E1524") * (1.0 + 0.10 * boost)
    bg_bot = hex_rgb("#070B12")
    bg_rgb = np.where(
        grad_t[..., None] < 0.45,
        bg_top * (1.0 - (grad_t / 0.45)[..., None])
        + bg_mid * (grad_t / 0.45)[..., None],
        bg_mid * (1.0 - ((grad_t - 0.45) / 0.55)[..., None])
        + bg_bot * ((grad_t - 0.45) / 0.55)[..., None],
    )
    # Subtle top-center studio key light dome
    dome_d = np.sqrt((xx * 0.9) ** 2 + ((yy + 0.46) * 1.15) ** 2)
    dome_light = np.exp(-(dome_d ** 2) * 2.2) * (0.070 + 0.03 * boost)
    bg_rgb = np.clip(bg_rgb + dome_light[..., None] * hex_rgb("#94A3B8"), 0.0, 1.0)

    # Outer shape mask & razor-crisp machined titanium rim
    if mode == "squircle":
        sq_half = 0.908 + 0.068 * boost
        sq_p = 4.6 - 0.3 * boost
        sd_val = (np.abs(xx / sq_half) ** sq_p + np.abs(yy / sq_half) ** sq_p) ** (
            1.0 / sq_p
        ) - 1.0
        eps = 1e-6
        gx = (np.abs(xx / sq_half) + eps) ** (sq_p - 1.0) / sq_half
        gy = (np.abs(yy / sq_half) + eps) ** (sq_p - 1.0) / sq_half
        g_norm = np.sqrt(gx * gx + gy * gy) * (
            (np.abs(xx / sq_half) ** sq_p + np.abs(yy / sq_half) ** sq_p + eps)
            ** (1.0 / sq_p - 1.0)
        )
        sq_dist = sd_val / np.maximum(g_norm, 0.25)

        outer_alpha = 1.0 - smoothstep(-px * 0.75, px * 0.75, sq_dist)

        # Refined 1px-feel machined titanium edge (sleek at hero, slightly crisper at 16px)
        rim_w = 0.0125 + 0.036 * boost
        rim_mask = (1.0 - smoothstep(-px * 0.6, px * 0.6, sq_dist)) * smoothstep(
            -rim_w - px, -rim_w + px, sq_dist
        )
        rim_dir = np.clip(0.52 - 0.42 * yy - 0.16 * xx, 0.0, 1.0)
        rim_col = (
            hex_rgb("#1E293B") * (1.0 - rim_dir[..., None])
            + hex_rgb("#7C8DA6") * rim_dir[..., None]
        )
        if boost > 0.0:
            rim_col = np.clip(rim_col * (1.0 + 0.35 * boost), 0.0, 1.0)
        bg_rgb = mix_rgb(bg_rgb, rim_col, rim_mask * (0.86 + 0.14 * boost))

        # Delicate dark inset seam inside rim
        inset_mask = smoothstep(-rim_w - 0.010, -rim_w - 0.003, sq_dist) * (
            1.0 - smoothstep(-rim_w - 0.003, -rim_w + 0.002, sq_dist)
        )
        bg_rgb = mix_rgb(bg_rgb, hex_rgb("#03060B"), inset_mask * 0.50)

    elif mode == "round":
        r_circ = 0.94
        c_dist = np.sqrt(xx * xx + yy * yy) - r_circ
        outer_alpha = 1.0 - smoothstep(-px * 0.75, px * 0.75, c_dist)
        rim_w = 0.015
        rim_mask = (1.0 - smoothstep(-px * 0.6, px * 0.6, c_dist)) * smoothstep(
            -rim_w - px, -rim_w + px, c_dist
        )
        rim_dir = np.clip(0.52 - 0.42 * yy - 0.16 * xx, 0.0, 1.0)
        rim_col = (
            hex_rgb("#1E293B") * (1.0 - rim_dir[..., None])
            + hex_rgb("#7C8DA6") * rim_dir[..., None]
        )
        bg_rgb = mix_rgb(bg_rgb, rim_col, rim_mask * 0.86)
    else:
        outer_alpha = np.ones_like(xx)

    rgb = bg_rgb.copy()

    # --- 2. Precision Concentric Optical Lens ---
    r = np.sqrt(xx * xx + yy * yy)
    theta = np.arctan2(yy, xx)
    light_tl = np.clip(0.5 - 0.42 * np.sin(theta) - 0.25 * np.cos(theta), 0.0, 1.0)
    brush = 0.5 + 0.5 * np.cos(2.0 * (theta + 0.68))

    r_outer_housing = 0.725 * lens_scale
    r_groove1_out = 0.642 * lens_scale
    r_groove1_in = 0.602 * lens_scale
    r_mid_collar = 0.602 * lens_scale
    r_groove2_out = 0.525 * lens_scale
    r_glass_outer = 0.488 * lens_scale
    r_glass_step = 0.365 * lens_scale
    r_pupil_rim = 0.254 * lens_scale
    r_pupil = 0.224 * lens_scale

    # At tiny tray sizes (16px/24px), open the cobalt optical ring wider for unmistakable silhouette
    if boost > 0.25:
        r_glass_outer = (0.488 + 0.115 * boost) * lens_scale
        r_groove2_out = r_glass_outer + 0.022 * lens_scale
        r_mid_collar = np.maximum(r_mid_collar, r_groove2_out + 0.025 * lens_scale)
        r_pupil = (0.224 + 0.036 * boost) * lens_scale
        r_pupil_rim = r_pupil + 0.020 * lens_scale

    # Ambient occlusion shadow under outer barrel
    ao_r = np.sqrt(xx * xx + (yy - 0.030 * lens_scale) ** 2)
    ao_alpha = (1.0 - smoothstep(r_outer_housing * 0.92, r_outer_housing * 1.11, ao_r)) * 0.65
    rgb = mix_rgb(rgb, hex_rgb("#020408"), ao_alpha)

    # 2a. Outer Machined Titanium Barrel
    outer_barrel_mask = 1.0 - smoothstep(r_outer_housing - px, r_outer_housing + px, r)
    barrel_col = (
        hex_rgb("#1A2230") * (1.0 - light_tl[..., None])
        + hex_rgb("#4B5B73") * light_tl[..., None]
        + brush[..., None] * 0.055 * hex_rgb("#CBD5E1")
    )
    outer_lip = (
        smoothstep(r_outer_housing - 0.026 * lens_scale, r_outer_housing - 0.007 * lens_scale, r)
        * outer_barrel_mask
    )
    lip_col = (
        hex_rgb("#334155") * (1.0 - light_tl[..., None])
        + hex_rgb("#CBD5E1") * light_tl[..., None]
    )
    barrel_col = mix_rgb(barrel_col, lip_col, outer_lip * (0.76 + 0.18 * boost))
    rgb = mix_rgb(rgb, barrel_col, outer_barrel_mask)

    # 2b. Recessed Channel 1
    if boost < 0.7:
        g1_mask = (1.0 - smoothstep(r_groove1_out - px, r_groove1_out + px, r)) * smoothstep(
            r_groove1_in - px, r_groove1_in + px, r
        )
        rgb = mix_rgb(rgb, hex_rgb("#05080F"), g1_mask * (0.94 - 0.35 * boost))

    # 2c. Mid Machined Gunmetal Collar
    mid_mask = 1.0 - smoothstep(r_mid_collar - px, r_mid_collar + px, r)
    mid_col = (
        hex_rgb("#161D29") * (1.0 - light_tl[..., None])
        + hex_rgb("#3F4E66") * light_tl[..., None]
        + brush[..., None] * 0.04 * hex_rgb("#94A3B8")
    )
    mid_lip = (
        smoothstep(r_mid_collar - 0.022 * lens_scale, r_mid_collar - 0.006 * lens_scale, r)
        * mid_mask
    )
    mid_col = mix_rgb(
        mid_col,
        hex_rgb("#2A3649") * (1.0 - light_tl[..., None]) + hex_rgb("#94A3B8") * light_tl[..., None],
        mid_lip * 0.72,
    )
    rgb = mix_rgb(rgb, mid_col, mid_mask)

    # 2d. Inner Lens Seat Shadow Channel 2
    g2_mask = 1.0 - smoothstep(r_groove2_out - px, r_groove2_out + px, r)
    rgb = mix_rgb(rgb, hex_rgb("#04070E"), g2_mask)

    # --- 3. Multi-Coated Studio Cobalt-Sapphire Optical Glass ---
    glass_mask = 1.0 - smoothstep(r_glass_outer - px, r_glass_outer + px, r)
    rn = np.clip(r / np.maximum(r_glass_outer, 1e-5), 0.0, 1.0)
    diag = np.clip((xx + yy) / (2.0 * r_glass_outer) * 0.5 + 0.5, 0.0, 1.0)

    c_deep = hex_rgb("#0F2A6B")
    c_cobalt = hex_rgb("#1D4ED8")
    c_electric = hex_rgb("#2563EB")
    c_cyan = hex_rgb("#38BDF8")

    glass_col = c_deep * (1.0 - rn[..., None]) + c_cobalt * rn[..., None]
    glass_col = mix_rgb(glass_col, c_electric, smoothstep(0.18, 0.86, rn) * (0.68 + 0.28 * boost))

    # Anti-reflective cyan-sapphire crescent sweep on lower-right rim
    ar_angle = np.cos(theta - math.radians(42))
    ar_arc = smoothstep(0.05, 0.95, ar_angle) * smoothstep(0.52, 0.97, rn)
    glass_col = mix_rgb(glass_col, c_cyan, ar_arc * (0.80 + 0.16 * boost))

    # Subtle secondary cobalt-sky bloom on upper-left optical dome
    ar_tl = smoothstep(0.15, 0.95, np.cos(theta - math.radians(-135))) * smoothstep(0.42, 0.92, rn)
    glass_col = mix_rgb(glass_col, hex_rgb("#60A5FA"), ar_tl * 0.40)

    # Inner concentric optical element step
    step_mask = 1.0 - smoothstep(r_glass_step - px * 1.4, r_glass_step + px * 1.4, r)
    step_tint = (
        hex_rgb("#0C2054") * (1.0 - diag[..., None] * 0.5)
        + hex_rgb("#1E40AF") * (diag[..., None] * 0.5)
    )
    glass_col = mix_rgb(glass_col, step_tint, step_mask * (0.60 - 0.25 * boost))
    step_ring = np.exp(-(((r - r_glass_step) / (0.012 * lens_scale)) ** 2))
    glass_col = mix_rgb(glass_col, hex_rgb("#60A5FA"), step_ring * (0.28 + 0.10 * boost))

    # Outer glass inner-chamfer reflection
    glass_rim = (
        smoothstep(r_glass_outer - 0.022 * lens_scale, r_glass_outer - 0.004 * lens_scale, r)
        * glass_mask
    )
    glass_col = mix_rgb(
        glass_col,
        hex_rgb("#1E3A8A") * (1.0 - diag[..., None]) + hex_rgb("#7DD3FC") * diag[..., None],
        glass_rim * 0.65,
    )

    # Curved upper-left coated-glass dome reflection (authentic Leica/Sony lens reflection, replaces round dot)
    dome_r_band = smoothstep(0.56, 0.72, rn) * (1.0 - smoothstep(0.86, 0.96, rn))
    d_ang_glass_tl = np.abs(
        np.arctan2(
            np.sin(theta - math.radians(-128)),
            np.cos(theta - math.radians(-128)),
        )
    )
    dome_ang = 1.0 - smoothstep(math.radians(18), math.radians(52), d_ang_glass_tl)
    glass_dome_sheen = dome_r_band * dome_ang * glass_mask
    glass_col = mix_rgb(glass_col, hex_rgb("#E0F2FE"), glass_dome_sheen * (0.30 + 0.08 * boost))

    # Ultra-subtle diagonal optical cut sheen on upper-left glass hemisphere
    diag_cut = smoothstep(0.04 * lens_scale, -0.02 * lens_scale, xx + yy + 0.04 * lens_scale)
    glass_col = mix_rgb(glass_col, hex_rgb("#E0F2FE"), diag_cut * glass_mask * 0.09)

    rgb = mix_rgb(rgb, glass_col, glass_mask)

    # --- 4. Deep Glossy Obsidian Sensor Pupil ---
    pupil_rim_mask = 1.0 - smoothstep(r_pupil_rim - px, r_pupil_rim + px, r)
    rgb = mix_rgb(rgb, hex_rgb("#071433"), pupil_rim_mask * 0.85)

    pupil_mask = 1.0 - smoothstep(r_pupil - px, r_pupil + px, r)
    prn = np.clip(r / np.maximum(r_pupil, 1e-5), 0.0, 1.0)
    pupil_col = (
        hex_rgb("#020409") * (1.0 - prn[..., None] * 0.35)
        + hex_rgb("#071022") * (prn[..., None] * 0.35)
    )
    rgb = mix_rgb(rgb, pupil_col, pupil_mask)

    # --- 5. Precision Optical Specular Reflections ---
    # 5a. Primary razor-sharp curved white specular crescent inside upper-left of pupil
    arc_r_mid = r_pupil * 0.76
    arc_thick = r_pupil * (0.14 + 0.11 * boost)
    radial_band = 1.0 - smoothstep(arc_thick * 0.45, arc_thick * 1.15, np.abs(r - arc_r_mid))
    d_ang_tl = np.abs(
        np.arctan2(
            np.sin(theta - math.radians(-135)),
            np.cos(theta - math.radians(-135)),
        )
    )
    ang_band_tl = 1.0 - smoothstep(math.radians(24), math.radians(52), d_ang_tl)
    spec_primary = radial_band * ang_band_tl * pupil_mask
    rgb = mix_rgb(rgb, hex_rgb("#FFFFFF"), spec_primary * (0.95 + 0.05 * boost))

    # 5b. Secondary cyan-blue internal reflection arc at lower-right of pupil
    d_ang_br = np.abs(
        np.arctan2(
            np.sin(theta - math.radians(45)),
            np.cos(theta - math.radians(45)),
        )
    )
    arc_r_br = r_pupil * 0.76
    radial_br = 1.0 - smoothstep(arc_thick * 0.32, arc_thick * 0.85, np.abs(r - arc_r_br))
    ang_band_br = 1.0 - smoothstep(math.radians(14), math.radians(36), d_ang_br)
    spec_secondary = radial_br * ang_band_br * pupil_mask
    rgb = mix_rgb(rgb, hex_rgb("#7DD3FC"), spec_secondary * 0.65)

    # --- 6. Crisp Emerald Live Tally Indicator Dot (#22C55E) ---
    td = np.sqrt((xx - tally_cx) ** 2 + (yy - tally_cy) ** 2)

    # Tight, restrained emerald reflection on studio chassis (no blurry neon slop)
    tally_halo = np.exp(-((td / (tally_r * 1.9)) ** 2)) * (0.10 + 0.08 * boost)
    rgb = np.clip(rgb + tally_halo[..., None] * hex_rgb("#22C55E"), 0.0, 1.0)

    # Delicate recessed dark collar around tally LED
    tally_bezel = 1.0 - smoothstep(tally_r * 1.13 - px, tally_r * 1.13 + px, td)
    rgb = mix_rgb(rgb, hex_rgb("#050911"), tally_bezel * (0.62 - 0.30 * boost))

    # Crisp emerald #22C55E LED core
    tally_mask = 1.0 - smoothstep(tally_r - px, tally_r + px, td)
    t_dx = (xx - tally_cx) / np.maximum(tally_r, 1e-5)
    t_dy = (yy - tally_cy) / np.maximum(tally_r, 1e-5)
    t_light = np.clip(0.55 - 0.30 * t_dx - 0.30 * t_dy, 0.0, 1.0)
    tally_col = (
        hex_rgb("#16A34A") * (1.0 - t_light[..., None])
        + hex_rgb("#34D399") * t_light[..., None]
    )
    tally_col = mix_rgb(tally_col, hex_rgb("#22C55E"), np.full_like(td, 0.55))
    if boost > 0.3:
        tally_col = mix_rgb(tally_col, hex_rgb("#4ADE80"), np.full_like(td, 0.35 * boost))
    # Subtle micro-specular dot on upper-left of LED (only visible at larger sizes)
    t_glint_d = np.sqrt((t_dx + 0.28) ** 2 + (t_dy + 0.28) ** 2)
    t_glint = 1.0 - smoothstep(0.14, 0.36, t_glint_d)
    tally_col = mix_rgb(tally_col, hex_rgb("#DCFCE7"), t_glint * (0.38 * (1.0 - boost)))
    rgb = mix_rgb(rgb, tally_col, tally_mask)

    rgba = np.empty((res, res, 4), dtype=np.uint8)
    rgba[..., :3] = np.clip(np.round(rgb * 255.0), 0, 255).astype(np.uint8)
    rgba[..., 3] = np.clip(np.round(outer_alpha * 255.0), 0, 255).astype(np.uint8)

    img = Image.fromarray(rgba, mode="RGBA")
    if ss > 1:
        img = img.resize((size, size), Image.Resampling.LANCZOS)
    if mode in ("full_bleed", "adaptive_preview"):
        img = img.convert("RGB")
    return img


def render_monochrome_preview(
    size: int, fg_hex: str = "#D3E3FD", bg_hex: str = "#1F2937"
) -> Image.Image:
    """Render an exact preview of Android 13+ Material You monochrome themed icon."""
    ss = 4
    res = size * ss
    c = (np.arange(res, dtype=np.float32) + 0.5) / res * 108.0
    xx, yy = np.meshgrid(c, c)
    px = 108.0 / res

    dx = xx - 54.0
    dy = yy - 54.0
    r = np.sqrt(dx * dx + dy * dy)
    theta = np.arctan2(dy, dx)

    # Outer optical lens ring: outer r=22.0, inner r=16.2
    ring_outer = (1.0 - smoothstep(22.0 - px, 22.0 + px, r)) * smoothstep(
        16.2 - px, 16.2 + px, r
    )
    # Inner optical pupil solid disc r=10.2 with a curved specular crescent cutout
    pupil_disc = 1.0 - smoothstep(10.2 - px, 10.2 + px, r)
    # Curved crescent cutout in upper-left of pupil (r in [5.4, 8.2], angle around -135 deg)
    d_ang = np.abs(
        np.arctan2(
            np.sin(theta - math.radians(-135)),
            np.cos(theta - math.radians(-135)),
        )
    )
    crescent_cut = (
        smoothstep(5.2 - px, 5.2 + px, r)
        * (1.0 - smoothstep(8.2 - px, 8.2 + px, r))
        * (1.0 - smoothstep(math.radians(34), math.radians(40), d_ang))
    )
    pupil_shape = np.clip(pupil_disc * (1.0 - crescent_cut), 0.0, 1.0)

    # Tally dot strictly inside safe circle: (72.0, 36.0), r=3.1
    td = np.sqrt((xx - 72.0) ** 2 + (yy - 36.0) ** 2)
    tally = 1.0 - smoothstep(3.1 - px, 3.1 + px, td)

    fg_mask = np.clip(ring_outer + pupil_shape + tally, 0.0, 1.0)

    bg = hex_rgb(bg_hex)
    fg = hex_rgb(fg_hex)
    rgb = bg[None, None, :] * (1.0 - fg_mask[..., None]) + fg[None, None, :] * fg_mask[..., None]

    # Crop central 72/108dp viewport and apply circle mask
    crop_min = int(round(res * (18.0 / 108.0)))
    crop_max = int(round(res * (90.0 / 108.0)))
    rgb_crop = rgb[crop_min:crop_max, crop_min:crop_max]
    h, _, _ = rgb_crop.shape
    cc = (np.arange(h, dtype=np.float32) + 0.5) / h * 2.0 - 1.0
    cx, cy = np.meshgrid(cc, cc)
    cr = np.sqrt(cx * cx + cy * cy)
    alpha = 1.0 - smoothstep(1.0 - 2.0 / h, 1.0 + 2.0 / h, cr)

    out = np.empty((h, h, 4), dtype=np.uint8)
    out[..., :3] = np.clip(np.round(rgb_crop * 255.0), 0, 255).astype(np.uint8)
    out[..., 3] = np.clip(np.round(alpha * 255.0), 0, 255).astype(np.uint8)
    return Image.fromarray(out, mode="RGBA").resize((size, size), Image.Resampling.LANCZOS)


def apply_android_mask(adaptive_108: Image.Image, shape: str, out_size: int) -> Image.Image:
    """Crop the central 72/108dp viewport and apply an Android Adaptive Icon mask."""
    w, h = adaptive_108.size
    m = int(round(w * (18.0 / 108.0)))
    cropped = (
        adaptive_108.crop((m, m, w - m, h - m))
        .resize((out_size * 4, out_size * 4), Image.Resampling.LANCZOS)
        .convert("RGBA")
    )

    res = out_size * 4
    c = (np.arange(res, dtype=np.float32) + 0.5) / res * 2.0 - 1.0
    xx, yy = np.meshgrid(c, c)
    px = 2.0 / res

    if shape == "circle":
        dist = np.sqrt(xx * xx + yy * yy) - 0.99
    elif shape == "squircle":
        p = 4.2
        r0 = 0.99
        dist = ((np.abs(xx / r0) ** p + np.abs(yy / r0) ** p) ** (1.0 / p) - 1.0) * r0
    elif shape == "rounded_square":
        rad = 0.34
        b = 0.99 - rad
        qx = np.maximum(np.abs(xx) - b, 0.0)
        qy = np.maximum(np.abs(yy) - b, 0.0)
        dist = np.sqrt(qx * qx + qy * qy) - rad
    elif shape == "teardrop":
        r_circ = np.sqrt(xx * xx + yy * yy) - 0.99
        rad = 0.22
        b = 0.99 - rad
        qx = np.maximum(np.abs(xx) - b, 0.0)
        qy = np.maximum(np.abs(yy) - b, 0.0)
        r_sq = np.sqrt(qx * qx + qy * qy) - rad
        dist = np.where((xx > 0.0) & (yy > 0.0), r_sq, r_circ)
    else:
        raise ValueError(f"Unknown shape: {shape}")

    mask = 1.0 - smoothstep(-px * 1.2, px * 1.2, dist)
    arr = np.array(cropped, dtype=np.uint8)
    arr[..., 3] = np.clip(np.round(mask * 255.0), 0, 255).astype(np.uint8)
    return Image.fromarray(arr, mode="RGBA").resize((out_size, out_size), Image.Resampling.LANCZOS)


def write_multi_size_ico(path: Path, frames: list[Image.Image]) -> None:
    """
    Write a Windows multi-size .ico container with 32-bit RGBA PNG-compressed frames
    (crisp alpha on Windows Vista / 10 / 11 taskbar, Explorer, and system tray).
    """
    encoded_blobs: list[Tuple[int, int, bytes]] = []
    for img in frames:
        rgba = img.convert("RGBA")
        buf = io.BytesIO()
        rgba.save(buf, format="PNG", optimize=True)
        encoded_blobs.append((rgba.width, rgba.height, buf.getvalue()))

    count = len(encoded_blobs)
    header = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    directory = bytearray()
    payload = bytearray()

    for w, h, blob in encoded_blobs:
        bw = 0 if w >= 256 else w
        bh = 0 if h >= 256 else h
        directory.extend(
            struct.pack("<BBBBHHII", bw, bh, 0, 0, 1, 32, len(blob), offset)
        )
        payload.extend(blob)
        offset += len(blob)

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(header + bytes(directory) + bytes(payload))


def build_logo_svg() -> str:
    """Hand-crafted precision 512x512 SVG synced to the refined optical studio mark (GitHub-safe)."""
    return """<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512" role="img" aria-label="PhoneCam Studio Optical Lens Logo">
  <defs>
    <!-- Studio obsidian squircle base -->
    <linearGradient id="bgGrad" x1="50%" y1="0%" x2="50%" y2="100%">
      <stop offset="0%" stop-color="#192336" />
      <stop offset="48%" stop-color="#0E1524" />
      <stop offset="100%" stop-color="#070B12" />
    </linearGradient>
    <!-- Subtle top-center studio key light -->
    <radialGradient id="domeLight" cx="50%" cy="22%" r="60%">
      <stop offset="0%" stop-color="#94A3B8" stop-opacity="0.11" />
      <stop offset="100%" stop-color="#94A3B8" stop-opacity="0" />
    </radialGradient>
    <!-- Refined cool titanium squircle rim -->
    <linearGradient id="rimGrad" x1="20%" y1="0%" x2="80%" y2="100%">
      <stop offset="0%" stop-color="#7C8DA6" />
      <stop offset="48%" stop-color="#475569" />
      <stop offset="100%" stop-color="#1E293B" />
    </linearGradient>
    <!-- Outer brushed titanium barrel -->
    <linearGradient id="outerBarrel" x1="18%" y1="12%" x2="82%" y2="88%">
      <stop offset="0%" stop-color="#55657E" />
      <stop offset="42%" stop-color="#2B3649" />
      <stop offset="100%" stop-color="#161D2A" />
    </linearGradient>
    <!-- Outer titanium chamfer highlight -->
    <linearGradient id="barrelLip" x1="15%" y1="10%" x2="85%" y2="90%">
      <stop offset="0%" stop-color="#CBD5E1" stop-opacity="0.86" />
      <stop offset="50%" stop-color="#475569" stop-opacity="0.62" />
      <stop offset="100%" stop-color="#1E293B" stop-opacity="0.82" />
    </linearGradient>
    <!-- Mid machined gunmetal collar -->
    <linearGradient id="midCollar" x1="18%" y1="12%" x2="82%" y2="88%">
      <stop offset="0%" stop-color="#43536B" />
      <stop offset="48%" stop-color="#232D3F" />
      <stop offset="100%" stop-color="#131924" />
    </linearGradient>
    <!-- Studio cobalt-sapphire optical glass -->
    <radialGradient id="opticalGlass" cx="46%" cy="44%" r="56%">
      <stop offset="0%" stop-color="#0F2A6B" />
      <stop offset="52%" stop-color="#1D4ED8" />
      <stop offset="84%" stop-color="#2563EB" />
      <stop offset="100%" stop-color="#38BDF8" />
    </radialGradient>
    <!-- Anti-reflective cyan sweep on lower-right -->
    <linearGradient id="arSweep" x1="18%" y1="18%" x2="86%" y2="86%">
      <stop offset="0%" stop-color="#60A5FA" stop-opacity="0.22" />
      <stop offset="52%" stop-color="#1D4ED8" stop-opacity="0" />
      <stop offset="86%" stop-color="#38BDF8" stop-opacity="0.58" />
      <stop offset="100%" stop-color="#7DD3FC" stop-opacity="0.84" />
    </linearGradient>
    <!-- Inner telephoto optical element -->
    <linearGradient id="innerElement" x1="20%" y1="20%" x2="80%" y2="80%">
      <stop offset="0%" stop-color="#0B1E4F" />
      <stop offset="100%" stop-color="#1E40AF" />
    </linearGradient>
    <!-- Glossy obsidian sensor pupil -->
    <radialGradient id="pupilGrad" cx="45%" cy="45%" r="55%">
      <stop offset="0%" stop-color="#020409" />
      <stop offset="78%" stop-color="#040914" />
      <stop offset="100%" stop-color="#0A1730" />
    </radialGradient>
    <!-- Emerald live tally LED -->
    <linearGradient id="tallyGrad" x1="20%" y1="15%" x2="85%" y2="85%">
      <stop offset="0%" stop-color="#34D399" />
      <stop offset="50%" stop-color="#22C55E" />
      <stop offset="100%" stop-color="#16A34A" />
    </linearGradient>
    <radialGradient id="tallyGlow" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="#22C55E" stop-opacity="0.18" />
      <stop offset="100%" stop-color="#22C55E" stop-opacity="0" />
    </radialGradient>
    <clipPath id="glassClip">
      <circle cx="256" cy="256" r="108" />
    </clipPath>
  </defs>

  <!-- Matte deep-obsidian studio squircle with refined cool titanium rim -->
  <rect x="24" y="24" width="464" height="464" rx="108" fill="url(#bgGrad)" stroke="url(#rimGrad)" stroke-width="3" />
  <rect x="26" y="26" width="460" height="460" rx="106" fill="url(#domeLight)" />

  <!-- Subtle barrel ambient occlusion -->
  <circle cx="256" cy="263" r="164" fill="#020408" fill-opacity="0.55" />

  <!-- Concentric machined titanium lens housing -->
  <circle cx="256" cy="256" r="160.5" fill="url(#outerBarrel)" stroke="url(#barrelLip)" stroke-width="3.2" />
  <circle cx="256" cy="256" r="142" fill="#05080F" />
  <circle cx="256" cy="256" r="133" fill="url(#midCollar)" stroke="#64748B" stroke-opacity="0.34" stroke-width="1.8" />
  <circle cx="256" cy="256" r="116" fill="#04070E" />

  <!-- Studio cobalt-sapphire optical glass -->
  <circle cx="256" cy="256" r="108" fill="url(#opticalGlass)" />
  <circle cx="256" cy="256" r="108" fill="url(#arSweep)" />

  <!-- Subtle upper-left coated glass dome reflection arc -->
  <path d="M174,230 A86,86 0 0,1 236,172" fill="none" stroke="#E0F2FE" stroke-width="16" stroke-linecap="round" stroke-opacity="0.22" clip-path="url(#glassClip)" />
  <path d="M148,256 A108,108 0 0,1 332,179 L179,332 A108,108 0 0,1 148,256 Z" fill="#E0F2FE" fill-opacity="0.08" clip-path="url(#glassClip)" />

  <!-- Inner optical element step -->
  <circle cx="256" cy="256" r="81" fill="url(#innerElement)" fill-opacity="0.60" stroke="#60A5FA" stroke-opacity="0.32" stroke-width="1.8" />

  <!-- Deep glossy obsidian sensor pupil -->
  <circle cx="256" cy="256" r="56" fill="#071433" />
  <circle cx="256" cy="256" r="49.5" fill="url(#pupilGrad)" />

  <!-- Precision optical specular crescent & secondary internal reflection -->
  <path d="M224,239 A36,36 0 0,1 239,224" fill="none" stroke="#FFFFFF" stroke-width="6.5" stroke-linecap="round" stroke-opacity="0.95" />
  <path d="M286,269 A35,35 0 0,1 269,286" fill="none" stroke="#7DD3FC" stroke-width="3.5" stroke-linecap="round" stroke-opacity="0.66" />

  <!-- Crisp emerald live tally indicator -->
  <circle cx="400" cy="112" r="27" fill="url(#tallyGlow)" />
  <circle cx="400" cy="112" r="16.2" fill="#050911" fill-opacity="0.62" />
  <circle cx="400" cy="112" r="14.2" fill="url(#tallyGrad)" />
  <circle cx="396" cy="108" r="3.4" fill="#DCFCE7" fill-opacity="0.45" />
</svg>
"""


def build_android_background_xml() -> str:
    """Obsidian studio vertical gradient VectorDrawable (108dp x 108dp)."""
    return """<?xml version="1.0" encoding="utf-8"?>
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:aapt="http://schemas.android.com/aapt"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <path android:pathData="M0,0h108v108h-108z">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="linear"
                android:startX="54"
                android:startY="0"
                android:endX="54"
                android:endY="108">
                <item android:color="#FF192336" android:offset="0.0" />
                <item android:color="#FF0E1524" android:offset="0.48" />
                <item android:color="#FF070B12" android:offset="1.0" />
            </gradient>
        </aapt:attr>
    </path>
    <path android:pathData="M0,0h108v108h-108z">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="radial"
                android:centerX="54"
                android:centerY="28"
                android:gradientRadius="56">
                <item android:color="#2094A3B8" android:offset="0.0" />
                <item android:color="#0094A3B8" android:offset="1.0" />
            </gradient>
        </aapt:attr>
    </path>
</vector>
"""


def build_android_foreground_xml() -> str:
    """Precision optical lens + cobalt glass + pupil + emerald tally strictly inside 66dp safe circle."""
    # Safe circle: center (54, 54), radius 33dp.
    # Outer barrel r=22.0dp; tally dot center (72.0, 36.0) -> dist=25.46dp, r=2.7dp -> outer edge 28.16dp (4.8dp clear margin!).
    return """<?xml version="1.0" encoding="utf-8"?>
<!-- PhoneCam precision optical studio lens strictly inside the 66dp adaptive safe circle. -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:aapt="http://schemas.android.com/aapt"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">

    <!-- Subtle barrel ambient occlusion -->
    <path
        android:fillColor="#88020408"
        android:pathData="M54,55.0m-22.5,0a22.5,22.5 0,1 1,45.0 0a22.5,22.5 0,1 1,-45.0 0" />

    <!-- Outer brushed titanium housing -->
    <path android:pathData="M54,54m-22.0,0a22.0,22.0 0,1 1,44.0 0a22.0,22.0 0,1 1,-44.0 0">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="linear"
                android:startX="37"
                android:startY="37"
                android:endX="71"
                android:endY="71">
                <item android:color="#FF5C6D88" android:offset="0.0" />
                <item android:color="#FF2B3649" android:offset="0.45" />
                <item android:color="#FF151C28" android:offset="1.0" />
            </gradient>
        </aapt:attr>
    </path>

    <!-- Crisp outer machined rim highlight -->
    <path
        android:fillColor="#00000000"
        android:strokeWidth="0.65"
        android:pathData="M54,54m-21.65,0a21.65,21.65 0,1 1,43.3 0a21.65,21.65 0,1 1,-43.3 0">
        <aapt:attr name="android:strokeColor">
            <gradient
                android:type="linear"
                android:startX="36"
                android:startY="36"
                android:endX="72"
                android:endY="72">
                <item android:color="#DDCBD5E1" android:offset="0.0" />
                <item android:color="#70475569" android:offset="0.55" />
                <item android:color="#9C1E293B" android:offset="1.0" />
            </gradient>
        </aapt:attr>
    </path>

    <!-- Recessed channel 1 -->
    <path
        android:fillColor="#FF05080F"
        android:pathData="M54,54m-19.5,0a19.5,19.5 0,1 1,39.0 0a19.5,19.5 0,1 1,-39.0 0" />

    <!-- Mid machined gunmetal collar -->
    <path android:pathData="M54,54m-18.3,0a18.3,18.3 0,1 1,36.6 0a18.3,18.3 0,1 1,-36.6 0">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="linear"
                android:startX="39"
                android:startY="39"
                android:endX="69"
                android:endY="69">
                <item android:color="#FF475872" android:offset="0.0" />
                <item android:color="#FF222C3E" android:offset="0.50" />
                <item android:color="#FF131924" android:offset="1.0" />
            </gradient>
        </aapt:attr>
    </path>

    <!-- Inner lens seat -->
    <path
        android:fillColor="#FF04070E"
        android:pathData="M54,54m-15.9,0a15.9,15.9 0,1 1,31.8 0a15.9,15.9 0,1 1,-31.8 0" />

    <!-- Multi-coated cobalt-sapphire optical glass -->
    <path android:pathData="M54,54m-14.8,0a14.8,14.8 0,1 1,29.6 0a14.8,14.8 0,1 1,-29.6 0">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="radial"
                android:centerX="52.5"
                android:centerY="52.5"
                android:gradientRadius="16.0">
                <item android:color="#FF0F2A6B" android:offset="0.0" />
                <item android:color="#FF1D4ED8" android:offset="0.54" />
                <item android:color="#FF2563EB" android:offset="0.82" />
                <item android:color="#FF38BDF8" android:offset="1.0" />
            </gradient>
        </aapt:attr>
    </path>

    <!-- Anti-reflective cyan-sapphire lower-right sweep -->
    <path android:pathData="M54,54m-14.8,0a14.8,14.8 0,1 1,29.6 0a14.8,14.8 0,1 1,-29.6 0">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="linear"
                android:startX="42"
                android:startY="42"
                android:endX="66"
                android:endY="66">
                <item android:color="#2E60A5FA" android:offset="0.0" />
                <item android:color="#001D4ED8" android:offset="0.52" />
                <item android:color="#9C38BDF8" android:offset="0.88" />
                <item android:color="#D47DD3FC" android:offset="1.0" />
            </gradient>
        </aapt:attr>
    </path>

    <!-- Subtle upper-left coated glass dome reflection arc -->
    <path
        android:fillColor="#00000000"
        android:strokeColor="#3BE0F2FE"
        android:strokeWidth="2.1"
        android:strokeLineCap="round"
        android:pathData="M43.0,50.5 A11.8,11.8 0 0,1 51.2,42.5" />

    <!-- Inner telephoto optical element -->
    <path
        android:fillColor="#960E2359"
        android:strokeColor="#5260A5FA"
        android:strokeWidth="0.4"
        android:pathData="M54,54m-11.1,0a11.1,11.1 0,1 1,22.2 0a11.1,11.1 0,1 1,-22.2 0" />

    <!-- Deep glossy obsidian sensor pupil -->
    <path
        android:fillColor="#FF071433"
        android:pathData="M54,54m-7.7,0a7.7,7.7 0,1 1,15.4 0a7.7,7.7 0,1 1,-15.4 0" />
    <path
        android:fillColor="#FF02050B"
        android:pathData="M54,54m-6.8,0a6.8,6.8 0,1 1,13.6 0a6.8,6.8 0,1 1,-13.6 0" />

    <!-- Primary diagonal white specular crescent -->
    <path
        android:fillColor="#00000000"
        android:strokeColor="#F4FFFFFF"
        android:strokeWidth="1.2"
        android:strokeLineCap="round"
        android:pathData="M49.4,51.6 A5.2,5.2 0 0,1 51.6,49.4" />

    <!-- Secondary cyan internal reflection arc -->
    <path
        android:fillColor="#00000000"
        android:strokeColor="#AD7DD3FC"
        android:strokeWidth="0.65"
        android:strokeLineCap="round"
        android:pathData="M58.2,55.9 A4.9,4.9 0 0,1 55.9,58.2" />

    <!-- Crisp emerald live tally indicator strictly inside 66dp safe circle -->
    <path
        android:fillColor="#99050911"
        android:pathData="M72.0,36.0m-3.1,0a3.1,3.1 0,1 1,6.2 0a3.1,3.1 0,1 1,-6.2 0" />
    <path android:pathData="M72.0,36.0m-2.7,0a2.7,2.7 0,1 1,5.4 0a2.7,2.7 0,1 1,-5.4 0">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="linear"
                android:startX="70.0"
                android:startY="34.0"
                android:endX="74.0"
                android:endY="38.0">
                <item android:color="#FF34D399" android:offset="0.0" />
                <item android:color="#FF22C55E" android:offset="0.5" />
                <item android:color="#FF16A34A" android:offset="1.0" />
            </gradient>
        </aapt:attr>
    </path>
    <path
        android:fillColor="#72DCFCE7"
        android:pathData="M71.2,35.2m-0.65,0a0.65,0.65 0,1 1,1.3 0a0.65,0.65 0,1 1,-1.3 0" />
</vector>
"""


def build_android_monochrome_xml() -> str:
    """Clean single-color Material You themed icon silhouette (Android 13+) with curved crescent highlight."""
    return """<?xml version="1.0" encoding="utf-8"?>
<!-- Android 13+ Material You monochrome silhouette strictly inside the 66dp safe circle. -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <!-- Outer optical lens ring -->
    <path
        android:fillColor="#FFFFFFFF"
        android:fillType="evenOdd"
        android:pathData="M54,32.0 A22.0,22.0 0 1,0 54,76.0 A22.0,22.0 0 1,0 54,32.0 Z M54,37.8 A16.2,16.2 0 1,1 54,70.2 A16.2,16.2 0 1,1 54,37.8 Z" />
    <!-- Inner sensor pupil with curved optical specular crescent cutout -->
    <path
        android:fillColor="#FFFFFFFF"
        android:fillType="evenOdd"
        android:pathData="M54,43.8 A10.2,10.2 0 1,0 54,64.2 A10.2,10.2 0 1,0 54,43.8 Z M47.5,51.2 A7.0,7.0 0 0,1 51.2,47.5 A1.2,1.2 0 0,1 52.8,49.1 A4.8,4.8 0 0,0 49.1,52.8 A1.2,1.2 0 0,1 47.5,51.2 Z" />
    <!-- Live tally indicator dot -->
    <path
        android:fillColor="#FFFFFFFF"
        android:pathData="M72.0,36.0m-3.1,0a3.1,3.1 0,1 1,6.2 0a3.1,3.1 0,1 1,-6.2 0" />
</vector>
"""


def build_adaptive_launcher_xml() -> str:
    return """<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@drawable/ic_launcher_background" />
    <foreground android:drawable="@drawable/ic_launcher_foreground" />
    <monochrome android:drawable="@drawable/ic_launcher_monochrome" />
</adaptive-icon>
"""


def recomposite_hero_showcase(logo_1024: Image.Image) -> None:
    """Replace the top-left brand icon inside assets/hero-showcase.png with the new optical logo."""
    hero_path = REPO_ROOT / "assets" / "hero-showcase.png"
    if not hero_path.is_file():
        return
    hero = Image.open(hero_path).convert("RGBA")
    arr = np.array(hero)
    # Sample clean background stripe right above the icon (y=18, x=40..102) so gradient matches seamlessly
    bg_strip = arr[16:22, 42:102, :3].mean(axis=(0, 1)).astype(np.uint8)
    # Erase old icon cleanly in x=[42..98], y=[30..88]
    bg_patch = Image.new("RGBA", (58, 60), (int(bg_strip[0]), int(bg_strip[1]), int(bg_strip[2]), 255))
    hero.paste(bg_patch, (42, 29))

    # Composite crisp 54x54 new optical logo aligned with "PhoneCam Studio" header lockup
    icon_54 = logo_1024.resize((54, 54), Image.Resampling.LANCZOS)
    hero.alpha_composite(icon_54, (44, 32))
    hero.convert("RGB").save(hero_path, format="PNG", optimize=True)
    print("Re-composited assets/hero-showcase.png with new optical studio logo")


def generate_audit_sheet(
    hero_512: Image.Image,
    ico_frames: dict[int, Image.Image],
    adaptive_512: Image.Image,
    play_512: Image.Image,
    output_path: Path,
) -> None:
    """Create a comprehensive visual audit sheet on dark & light surfaces + Android masks."""
    sheet_w, sheet_h = 1560, 1080
    sheet = Image.new("RGB", (sheet_w, sheet_h), (15, 20, 30))
    draw = ImageDraw.Draw(sheet)

    try:
        font_title = ImageFont.truetype("segoeui.ttf", 22)
        font_sub = ImageFont.truetype("segoeui.ttf", 15)
        font_sm = ImageFont.truetype("consola.ttf", 13)
    except OSError:
        font_title = ImageFont.load_default()
        font_sub = ImageFont.load_default()
        font_sm = ImageFont.load_default()

    draw.text(
        (28, 18),
        "PhoneCam Precision Optical Studio Icon — Pass 2 Visual Audit Sheet",
        fill=(241, 245, 249),
        font=font_title,
    )

    # Row 1: 512x512 Hero on Dark (#0d1117) and Light (#f3f4f6)
    hero_disp = hero_512.resize((360, 360), Image.Resampling.LANCZOS)
    draw.rounded_rectangle([28, 60, 448, 490], radius=16, fill=(13, 17, 23), outline=(45, 55, 72))
    draw.text((44, 72), "Hero 512px on Dark (#0d1117)", fill=(203, 213, 225), font=font_sub)
    sheet.paste(hero_disp, (58, 104), hero_disp)

    draw.rounded_rectangle([472, 60, 892, 490], radius=16, fill=(243, 244, 246), outline=(203, 213, 225))
    draw.text((488, 72), "Hero 512px on Light (#f3f4f6)", fill=(30, 41, 59), font=font_sub)
    sheet.paste(hero_disp, (502, 104), hero_disp)

    # Right column: Small-Size Tests
    draw.rounded_rectangle([916, 60, 1532, 490], radius=16, fill=(20, 26, 38), outline=(45, 55, 72))
    draw.text((936, 72), "Small Sizes (64 / 32 / 24 / 16px + 4x Nearest Zoom)", fill=(203, 213, 225), font=font_sub)

    draw.rounded_rectangle([936, 106, 1512, 248], radius=12, fill=(13, 17, 23), outline=(38, 48, 64))
    draw.text((952, 114), "Dark Taskbar / Tray (#0d1117)", fill=(148, 163, 184), font=font_sm)
    x_cur = 956
    for sz, lbl in [(64, "64"), (32, "32"), (24, "24"), (16, "16")]:
        im = ico_frames[sz]
        sheet.paste(im, (x_cur, 150 + (64 - sz) // 2), im)
        draw.text((x_cur, 222), f"{lbl}px", fill=(148, 163, 184), font=font_sm)
        x_cur += max(sz, 36) + 34
    z16 = ico_frames[16].resize((64, 64), Image.Resampling.NEAREST)
    sheet.paste(z16, (x_cur + 8, 150), z16)
    draw.text((x_cur + 4, 222), "16px 4x", fill=(148, 163, 184), font=font_sm)
    z24 = ico_frames[24].resize((72, 72), Image.Resampling.NEAREST)
    sheet.paste(z24, (x_cur + 104, 146), z24)
    draw.text((x_cur + 104, 222), "24px 3x", fill=(148, 163, 184), font=font_sm)

    draw.rounded_rectangle([936, 266, 1512, 408], radius=12, fill=(243, 244, 246), outline=(203, 213, 225))
    draw.text((952, 274), "Light Taskbar / Tray (#f3f4f6)", fill=(51, 65, 85), font=font_sm)
    x_cur = 956
    for sz, lbl in [(64, "64"), (32, "32"), (24, "24"), (16, "16")]:
        im = ico_frames[sz]
        sheet.paste(im, (x_cur, 310 + (64 - sz) // 2), im)
        draw.text((x_cur, 382), f"{lbl}px", fill=(51, 65, 85), font=font_sm)
        x_cur += max(sz, 36) + 34
    sheet.paste(z16, (x_cur + 8, 310), z16)
    draw.text((x_cur + 4, 382), "16px 4x", fill=(51, 65, 85), font=font_sm)
    sheet.paste(z24, (x_cur + 104, 306), z24)
    draw.text((x_cur + 104, 382), "24px 3x", fill=(51, 65, 85), font=font_sm)

    draw.text((936, 428), "ICO sizes: 16, 24, 32, 48, 64, 128, 256 (32-bit RGBA PNG frames)", fill=(148, 163, 184), font=font_sm)
    draw.text((936, 450), "Pass 2: refined 1px titanium rim, dome arc sheen, crisp 16px tally.", fill=(148, 163, 184), font=font_sm)

    # Row 2: Android Adaptive Masks + Safe Zone + Material You Monochrome + Play Store Full Bleed
    draw.rounded_rectangle([28, 516, 1532, 1052], radius=16, fill=(20, 26, 38), outline=(45, 55, 72))
    draw.text(
        (48, 532),
        "Android Adaptive Icon Masks (66/108dp Safe Zone) · Material You Monochrome · Play Store 512px",
        fill=(203, 213, 225),
        font=font_sub,
    )

    mask_specs = [
        ("circle", "Adaptive Circle"),
        ("squircle", "Adaptive Squircle"),
        ("rounded_square", "Rounded Square"),
        ("teardrop", "Teardrop Mask"),
    ]
    x0 = 56
    for shape, label in mask_specs:
        masked = apply_android_mask(adaptive_512, shape, 200)
        draw.rounded_rectangle([x0 - 12, 574, x0 + 212, 854], radius=14, fill=(13, 17, 23), outline=(38, 48, 64))
        sheet.paste(masked, (x0, 594), masked)
        pad = int(round(200 * (3.0 / 72.0)))
        draw.ellipse(
            [x0 + pad, 594 + pad, x0 + 200 - pad, 594 + 200 - pad],
            outline=(34, 197, 94),
            width=1,
        )
        draw.text((x0 + 24, 816), label, fill=(203, 213, 225), font=font_sub)
        x0 += 242

    mono_dark = render_monochrome_preview(180, fg_hex="#A8C7FA", bg_hex="#1E293B")
    mono_light = render_monochrome_preview(180, fg_hex="#1D4ED8", bg_hex="#DBEAFE")
    draw.rounded_rectangle([x0 - 12, 574, x0 + 224, 1024], radius=14, fill=(13, 17, 23), outline=(38, 48, 64))
    sheet.paste(mono_dark, (x0 + 16, 594), mono_dark)
    draw.text((x0 + 12, 782), "Android 13+ Mono (Dark)", fill=(203, 213, 225), font=font_sm)
    sheet.paste(mono_light, (x0 + 16, 810), mono_light)
    draw.text((x0 + 10, 998), "Android 13+ Mono (Light)", fill=(203, 213, 225), font=font_sm)

    ps_x = x0 + 248
    draw.rounded_rectangle([ps_x - 12, 574, ps_x + 224, 854], radius=14, fill=(13, 17, 23), outline=(38, 48, 64))
    ps_disp = play_512.resize((200, 200), Image.Resampling.LANCZOS)
    sheet.paste(ps_disp, (ps_x + 6, 594))
    draw.text((ps_x + 12, 816), "Play Store 512 Full-Bleed", fill=(203, 213, 225), font=font_sm)

    draw.text(
        (56, 892),
        "Green 1px circle overlay = 66dp Android Adaptive Safe Zone boundary inside 72dp visible viewport.",
        fill=(134, 239, 172),
        font=font_sub,
    )
    draw.text(
        (56, 922),
        "Pass 2: 4.8dp clear margin inside safe zone across Circle, Squircle, Rounded Square, and Teardrop.",
        fill=(148, 163, 184),
        font=font_sub,
    )

    output_path.parent.mkdir(parents=True, exist_ok=True)
    sheet.save(output_path, format="PNG")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate PhoneCam brand icons and audit sheet.")
    parser.add_argument(
        "--audit-only",
        action="store_true",
        help="Only generate the diagnostic audit contact sheet without modifying repo assets.",
    )
    parser.add_argument(
        "--audit-out",
        type=Path,
        default=Path(tempfile.gettempdir()) / "icon_audit_sheet.png",
        help="Path for diagnostic audit contact sheet.",
    )
    parser.add_argument(
        "--composite-hero",
        action="store_true",
        help="Re-composite assets/hero-showcase.png top-left icon with the new logo.png.",
    )
    args = parser.parse_args()

    ico_sizes = [16, 24, 32, 48, 64, 128, 256]
    boost_map = {16: 1.0, 24: 0.65, 32: 0.30, 48: 0.12, 64: 0.05, 128: 0.0, 256: 0.0}
    ico_frames: dict[int, Image.Image] = {}
    for sz in ico_sizes:
        ss = 8 if sz <= 32 else 4
        ico_frames[sz] = render_icon(
            sz, mode="squircle", supersample=ss, small_optical_boost=boost_map[sz]
        )

    hero_512 = render_icon(512, mode="squircle", supersample=4, small_optical_boost=0.0)
    adaptive_512 = render_icon(512, mode="adaptive_preview", supersample=4, small_optical_boost=0.0)
    play_512 = render_icon(512, mode="full_bleed", supersample=4, small_optical_boost=0.0)

    generate_audit_sheet(hero_512, ico_frames, adaptive_512, play_512, args.audit_out)
    print(f"Wrote audit contact sheet: {args.audit_out}")

    if args.audit_only:
        return

    assets_dir = REPO_ROOT / "assets"
    dist_unpacked = REPO_ROOT / "dist" / "unpacked"
    android_res = REPO_ROOT / "apps" / "android" / "app" / "src" / "main" / "res"

    # 1. Desktop SVG
    (assets_dir / "logo.svg").write_text(build_logo_svg(), encoding="utf-8", newline="\n")
    print("Wrote assets/logo.svg")

    # 2. Desktop 1024x1024 squircle PNG
    logo_1024 = render_icon(1024, mode="squircle", supersample=2, small_optical_boost=0.0)
    logo_1024.save(assets_dir / "logo.png", format="PNG", optimize=True)
    print("Wrote assets/logo.png (1024x1024)")

    # 3. Play Store 512x512 full-bleed opaque PNG
    play_512.save(assets_dir / "playstore-icon-512.png", format="PNG", optimize=True)
    print("Wrote assets/playstore-icon-512.png (512x512)")

    # 4. Multi-size Windows ICO (assets + dist/unpacked)
    ordered_frames = [ico_frames[s] for s in ico_sizes]
    write_multi_size_ico(assets_dir / "icon.ico", ordered_frames)
    write_multi_size_ico(dist_unpacked / "icon.ico", ordered_frames)
    print("Wrote assets/icon.ico & dist/unpacked/icon.ico")

    # 5. Android Adaptive + Monochrome VectorDrawables
    drawable_dir = android_res / "drawable"
    mipmap_v26 = android_res / "mipmap-anydpi-v26"
    drawable_dir.mkdir(parents=True, exist_ok=True)
    mipmap_v26.mkdir(parents=True, exist_ok=True)

    (drawable_dir / "ic_launcher_background.xml").write_text(
        build_android_background_xml(), encoding="utf-8", newline="\n"
    )
    (drawable_dir / "ic_launcher_foreground.xml").write_text(
        build_android_foreground_xml(), encoding="utf-8", newline="\n"
    )
    (drawable_dir / "ic_launcher_monochrome.xml").write_text(
        build_android_monochrome_xml(), encoding="utf-8", newline="\n"
    )
    adaptive_xml = build_adaptive_launcher_xml()
    (mipmap_v26 / "ic_launcher.xml").write_text(adaptive_xml, encoding="utf-8", newline="\n")
    (mipmap_v26 / "ic_launcher_round.xml").write_text(adaptive_xml, encoding="utf-8", newline="\n")
    print("Wrote Android Adaptive & Monochrome VectorDrawables")

    # 6. Android Legacy Launcher Mipmaps
    densities = {
        "mipmap-mdpi": 48,
        "mipmap-hdpi": 72,
        "mipmap-xhdpi": 96,
        "mipmap-xxhdpi": 144,
        "mipmap-xxxhdpi": 192,
    }
    for folder, px_size in densities.items():
        d_path = android_res / folder
        d_path.mkdir(parents=True, exist_ok=True)
        sq_icon = render_icon(
            px_size,
            mode="squircle",
            supersample=4,
            small_optical_boost=0.10 if px_size <= 48 else 0.0,
        )
        rnd_icon = render_icon(
            px_size,
            mode="round",
            supersample=4,
            small_optical_boost=0.10 if px_size <= 48 else 0.0,
        )
        sq_icon.save(d_path / "ic_launcher.png", format="PNG", optimize=True)
        rnd_icon.save(d_path / "ic_launcher_round.png", format="PNG", optimize=True)
    print("Wrote Android legacy mipmap PNGs (mdpi..xxxhdpi)")

    if args.composite_hero:
        recomposite_hero_showcase(logo_1024)


if __name__ == "__main__":
    main()
