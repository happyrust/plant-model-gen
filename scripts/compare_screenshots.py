import argparse
import glob
import os

import numpy as np
from PIL import Image


def blue_mask(img: Image.Image) -> np.ndarray:
    """Segment 'pipe' pixels by HSV hue range (robust to shading/background)."""
    img = img.convert("RGB")
    hsv = img.convert("HSV")
    h, s, v = hsv.split()
    h = np.asarray(h, dtype=np.uint8)
    s = np.asarray(s, dtype=np.uint8)
    v = np.asarray(v, dtype=np.uint8)

    # Pillow HSV: H in [0,255] maps to [0,360).
    # Blue roughly 200-260deg => ~142-184. Widen a bit for highlights.
    return (h >= 120) & (h <= 210) & (s >= 50) & (v >= 50)


def iou_and_dice(a: np.ndarray, b: np.ndarray) -> tuple[float, float, int]:
    inter = np.logical_and(a, b).sum()
    union = np.logical_or(a, b).sum()
    iou = float(inter) / float(union) if union else 0.0
    denom = int(a.sum() + b.sum())
    dice = float(2 * inter) / float(denom) if denom else 0.0
    return iou, dice, int(b.sum())

def normalize_mask(mask: np.ndarray, out_w: int = 512, out_h: int = 256) -> np.ndarray:
    """Crop to tight bbox (with small margin) and resize to a fixed canvas for comparison."""
    ys, xs = np.nonzero(mask)
    if len(xs) == 0 or len(ys) == 0:
        return np.zeros((out_h, out_w), dtype=bool)

    x0, x1 = int(xs.min()), int(xs.max())
    y0, y1 = int(ys.min()), int(ys.max())
    w = max(1, x1 - x0 + 1)
    h = max(1, y1 - y0 + 1)
    # 5% margin
    mx = max(2, int(round(w * 0.05)))
    my = max(2, int(round(h * 0.05)))
    x0 = max(0, x0 - mx)
    y0 = max(0, y0 - my)
    x1 = min(mask.shape[1] - 1, x1 + mx)
    y1 = min(mask.shape[0] - 1, y1 + my)

    cropped = mask[y0 : y1 + 1, x0 : x1 + 1]
    # Resize via PIL (nearest)
    img = Image.fromarray((cropped.astype(np.uint8) * 255), mode="L")
    img = img.resize((out_w, out_h), resample=Image.NEAREST)
    arr = np.asarray(img, dtype=np.uint8)
    return arr > 0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--baseline", required=True, help="baseline png path")
    ap.add_argument("--candidates", required=True, help="glob pattern for candidate pngs")
    ap.add_argument("--top", type=int, default=12, help="top-N to print")
    args = ap.parse_args()

    base_img = Image.open(args.baseline)
    base_mask = blue_mask(base_img)
    base_norm = normalize_mask(base_mask)

    rows: list[tuple[float, float, int, str]] = []
    for p in glob.glob(args.candidates):
        try:
            img = Image.open(p)
        except Exception:
            continue
        if img.size != base_img.size:
            continue
        cand_mask = blue_mask(img)
        cand_norm = normalize_mask(cand_mask)
        iou, dice, area = iou_and_dice(base_norm, cand_norm)
        rows.append((iou, dice, area, p))

    rows.sort(key=lambda r: r[0], reverse=True)
    print(
        f"baseline={os.path.basename(args.baseline)} size={base_img.size} area={int(base_mask.sum())} norm={base_norm.shape}"
    )
    print(f"candidates={len(rows)}")
    print(f"top {min(args.top, len(rows))} by IoU:")
    for i, (iou, dice, area, p) in enumerate(rows[: args.top], 1):
        print(f"{i:2d}. iou={iou:.4f} dice={dice:.4f} area={area:7d}  {os.path.basename(p)}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
