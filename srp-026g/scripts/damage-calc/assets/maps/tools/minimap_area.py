"""The play area a native HUD minimap bitmap shows, as a boolean mask of a
W x W frame (the bitmap covers the same square frame as the render).

Two kinds of bitmap:
  "alpha" (the 0.26g maps): the play area is opaque and the rest clear, so
    the area is alpha > 0.15. London's bitmap fades out in a soft vignette
    around a more opaque play area: "alpha:0.4".
  "line" (the 0.11e ports): opaque everywhere with a blurred edge, the play
    area outlined by a drawn light-grey line; the area is what that line
    encloses (the line's gaps closed by 2 px).
"""
import numpy as np
from PIL import Image
from scipy import ndimage


def area_mask(bitmap, W, kind='alpha'):
    im = Image.open(bitmap).convert('RGBA')
    if kind.startswith('alpha'):
        threshold = float(kind.split(':', 1)[1]) if ':' in kind else 0.15
        alpha = np.asarray(im.resize((W, W), Image.BILINEAR), np.float32)[..., 3] / 255
        return ndimage.binary_fill_holes(alpha > threshold)
    if kind != 'line':
        raise ValueError(f'unknown play-area kind {kind!r}')
    x = np.asarray(im).astype(int)
    rgb = x[..., :3]
    line = (rgb.min(-1) > 150) & (rgb.max(-1) - rgb.min(-1) < 40) & (x[..., 3] > 200)
    line = ndimage.binary_dilation(line, iterations=2)
    seed = np.zeros_like(line)
    seed[0, :] = seed[-1, :] = seed[:, 0] = seed[:, -1] = True
    outside = ndimage.binary_propagation(seed & ~line, mask=~line)
    inside = ndimage.binary_opening(~outside, iterations=6)
    lab, n = ndimage.label(inside)
    inside = lab == 1 + int(np.argmax(ndimage.sum(inside, lab, range(1, n + 1))))
    return np.asarray(Image.fromarray(inside).resize((W, W), Image.NEAREST))
