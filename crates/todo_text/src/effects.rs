//! Pixel effects on premultiplied RGBA buffers: erosion mask and blur.

/// Integer hash → [0, 1). Stable across platforms so erosion is reproducible.
fn hash(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sy) = (smooth(fx), smooth(fy));
    let (ix, iy) = (x0 as i32, y0 as i32);
    let top = hash(ix, iy, seed) * (1.0 - sx) + hash(ix + 1, iy, seed) * sx;
    let bottom = hash(ix, iy + 1, seed) * (1.0 - sx) + hash(ix + 1, iy + 1, seed) * sx;
    top * (1.0 - sy) + bottom * sy
}

/// Two-octave worn-ink noise in [0, 1].
pub(crate) fn erosion_noise(x: f32, y: f32, seed: u32) -> f32 {
    0.65 * value_noise(x, y, seed)
        + 0.35 * value_noise(x * 2.7 + 17.3, y * 2.7 - 5.1, seed ^ 0x9e37)
}

/// Coverage kept for noise `n`: holes open where the noise is below `amount`.
pub(crate) fn erosion_keep(n: f32, amount: f32) -> f32 {
    let edge = 0.05;
    ((n - (amount - edge)) / (2.0 * edge)).clamp(0.0, 1.0)
}

/// Approximate Gaussian blur with standard deviation `sigma` px: three box passes.
pub(crate) fn blur(data: &mut [u8], width: usize, height: usize, sigma: f32) {
    if sigma < 0.5 || width == 0 || height == 0 {
        return;
    }
    // Box width whose three-fold convolution matches the Gaussian variance.
    let box_width = (12.0 * sigma * sigma / 3.0 + 1.0).sqrt();
    let radius = ((box_width - 1.0) / 2.0).round().max(1.0) as usize;
    let mut scratch = vec![0u8; data.len()];
    for _ in 0..3 {
        box_pass(data, &mut scratch, width, height, radius, true);
        box_pass(&scratch, data, width, height, radius, false);
    }
}

/// One separable box pass from `src` into `dst` (horizontal or vertical).
fn box_pass(
    src: &[u8],
    dst: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
    horizontal: bool,
) {
    let (lines, length) = if horizontal {
        (height, width)
    } else {
        (width, height)
    };
    let index = |line: usize, i: usize| {
        if horizontal {
            (line * width + i) * 4
        } else {
            (i * width + line) * 4
        }
    };
    let window = (2 * radius + 1) as u32;
    for line in 0..lines {
        let mut sum = [0u32; 4];
        // Edge pixels beyond the buffer count as transparent.
        for i in 0..=radius.min(length - 1) {
            let p = index(line, i);
            for c in 0..4 {
                sum[c] += u32::from(src[p + c]);
            }
        }
        for i in 0..length {
            let p = index(line, i);
            for c in 0..4 {
                dst[p + c] = ((sum[c] + window / 2) / window) as u8;
            }
            let add = i + radius + 1;
            if add < length {
                let q = index(line, add);
                for c in 0..4 {
                    sum[c] += u32::from(src[q + c]);
                }
            }
            if i >= radius {
                let q = index(line, i - radius);
                for c in 0..4 {
                    sum[c] -= u32::from(src[q + c]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_deterministic_and_bounded() {
        for i in 0..200 {
            let (x, y) = (i as f32 * 0.37, i as f32 * 0.91);
            let n = erosion_noise(x, y, 7);
            assert!((0.0..=1.0).contains(&n));
            assert_eq!(n, erosion_noise(x, y, 7));
        }
        assert_ne!(erosion_noise(1.5, 2.5, 1), erosion_noise(1.5, 2.5, 2));
        assert_eq!(erosion_keep(0.9, 0.2), 1.0);
        assert_eq!(erosion_keep(0.0, 0.2), 0.0);
    }

    #[test]
    fn blur_spreads_and_roughly_preserves_mass() {
        let (w, h) = (31, 31);
        let mut data = vec![0u8; w * h * 4];
        let center = (15 * w + 15) * 4;
        data[center..center + 4].copy_from_slice(&[255, 255, 255, 255]);
        // A 9x9 opaque block keeps the total measurable after rounding.
        for y in 11..20 {
            for x in 11..20 {
                let p = (y * w + x) * 4;
                data[p..p + 4].copy_from_slice(&[200, 0, 0, 200]);
            }
        }
        let before: u32 = data.iter().skip(3).step_by(4).map(|&a| u32::from(a)).sum();
        blur(&mut data, w, h, 2.0);
        let after: u32 = data.iter().skip(3).step_by(4).map(|&a| u32::from(a)).sum();
        assert!(
            data[(15 * w + 6) * 4 + 3] > 0,
            "blur reaches outside the block"
        );
        assert!((after as f32 - before as f32).abs() / (before as f32) < 0.05);
        // Premultiplied invariant: color never exceeds alpha.
        assert!(data.chunks(4).all(|p| p[0] <= p[3].saturating_add(1)));
    }
}
