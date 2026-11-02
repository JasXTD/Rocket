//! Exact decoded RGBA comparison. Unequal sizes share a top-left origin; absent
//! pixels differ even from fully transparent pixels. No resampling or color claim.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stats {
    pub changed: usize,
    pub compared: usize,
    /// x, y, width, height in decoded pixels; excludes the space outside both images.
    pub bounds: Option<[usize; 4]>,
}
pub(super) struct Difference {
    pub stats: Stats,
    pub image: ColorImage,
}
pub(super) fn compare(
    before: Option<&Raw>,
    after: Option<&Raw>,
    cancel: &CancellationToken,
) -> Result<Difference> {
    cancel.check()?;
    let width = before
        .iter()
        .chain(after.iter())
        .map(|i| i.info.width)
        .max()
        .unwrap_or(1);
    let height = before
        .iter()
        .chain(after.iter())
        .map(|i| i.info.height)
        .max()
        .unwrap_or(1);
    if width as u64 * height as u64 > MAX_PIXELS {
        return Err(Error::Unsupported("The combined image canvas exceeds 8 megapixels. Side by side, overlay and swipe remain available.".into()));
    }
    let pixel = |raw: Option<&Raw>, x: usize, y: usize| -> Option<[u8; 4]> {
        let raw = raw?;
        if x >= raw.info.width || y >= raw.info.height {
            return None;
        }
        let offset = (y * raw.info.width + x) * 4;
        Some(
            raw.rgba[offset..offset + 4]
                .try_into()
                .expect("decoded RGBA pixel"),
        )
    };
    let mut image = ColorImage::filled([width, height], Color32::TRANSPARENT);
    let (mut changed, mut compared) = (0, 0);
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (width, height, 0, 0);
    for y in 0..height {
        cancel.check()?;
        for x in 0..width {
            let (a, b) = (pixel(before, x, y), pixel(after, x, y));
            if a.is_none() && b.is_none() {
                continue;
            }
            compared += 1;
            image.pixels[y * width + x] = if a != b {
                changed += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                PIXEL_CHANGED()
            } else {
                PIXEL_SAME()
            };
        }
    }
    cancel.check()?;
    Ok(Difference {
        stats: Stats {
            changed,
            compared,
            bounds: (changed > 0).then_some([
                min_x,
                min_y,
                max_x.saturating_sub(min_x) + 1,
                max_y.saturating_sub(min_y) + 1,
            ]),
        },
        image,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn raw(w: usize, h: usize, pixel: [u8; 4]) -> Raw {
        Raw {
            info: Info {
                width: w,
                height: h,
                encoded_bytes: 0,
                format: "PNG",
                alpha: true,
            },
            rgba: pixel.repeat(w * h),
        }
    }
    #[test]
    fn exact_rgba_counts_hidden_color_alpha_and_bounds() {
        let a = raw(4, 3, [0, 0, 0, 0]);
        let mut b = raw(4, 3, [0, 0, 0, 0]);
        b.rgba[(4 + 1) * 4] = 1; // RGB still counts with zero alpha.
        b.rgba[(8 + 2) * 4 + 3] = 1;
        let d = compare(Some(&a), Some(&b), &CancellationToken::new()).unwrap();
        assert_eq!(
            d.stats,
            Stats {
                changed: 2,
                compared: 12,
                bounds: Some([1, 1, 2, 2])
            }
        );
        assert_eq!(
            compare(Some(&a), Some(&a), &CancellationToken::new())
                .unwrap()
                .stats
                .bounds,
            None
        );
    }
    #[test]
    fn resized_and_absent_images_preserve_origin_and_exclude_outside_union() {
        let a = raw(3, 1, [1, 2, 3, 0]);
        let b = raw(1, 3, [1, 2, 3, 0]);
        let d = compare(Some(&a), Some(&b), &CancellationToken::new()).unwrap();
        assert_eq!(d.image.size, [3, 3]);
        assert_eq!(d.stats.compared, 5);
        assert_eq!(d.stats.changed, 4);
        assert_eq!(d.image.pixels[8], Color32::TRANSPARENT);
        assert_eq!(
            compare(None, Some(&a), &CancellationToken::new())
                .unwrap()
                .stats
                .changed,
            3
        );
        assert_eq!(
            compare(Some(&a), None, &CancellationToken::new())
                .unwrap()
                .stats
                .changed,
            3
        );
    }
    #[test]
    fn cancellation_and_union_limit_before_allocation() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            compare(None, None, &cancel),
            Err(Error::Cancelled)
        ));
        // A valid individual decoded size can still exceed the combined-canvas limit.
        let mut a = raw(1, 1, [0; 4]);
        let mut b = raw(1, 1, [0; 4]);
        a.info.width = 4096;
        a.info.height = 2048;
        b.info.width = 2048;
        b.info.height = 4096;
        assert!(matches!(
            compare(Some(&a), Some(&b), &CancellationToken::new()),
            Err(Error::Unsupported(_))
        ));
    }
}
