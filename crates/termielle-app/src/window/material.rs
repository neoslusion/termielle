//! Frosted material composition. Material opacity and silhouette coverage
//! are different: premultiplied edge colour already contains its coverage.
pub(super) fn composite(source: &[u8], background: Option<&[u8]>, tint_alpha: u8) -> [u8; 4] {
    let raw = [source[0], source[1], source[2], source[3]];
    if source[3] == 0 {
        return [0; 4];
    }
    let Some(background) = background else {
        return raw;
    };
    if tint_alpha == 0 {
        return raw;
    }
    let alpha = u32::from(source[3]);
    let coverage = ((alpha * 255 + u32::from(tint_alpha) / 2) / u32::from(tint_alpha)).min(255);
    let uncovered = coverage - alpha;
    let mut out = [0; 4];
    for channel in 0..3 {
        out[channel] = (u32::from(source[channel])
            + u32::from(background[channel]) * uncovered / 255)
            .min(coverage) as u8;
    }
    out[3] = coverage as u8;
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_background_and_fully_transparent_pixels_stay_unchanged() {
        assert_eq!(composite(&[24, 32, 40, 128], None, 224), [24, 32, 40, 128]);
        assert_eq!(composite(&[0; 4], Some(&[240, 240, 240, 255]), 224), [0; 4]);
        assert_eq!(
            composite(&[0, 0, 0, 48], Some(&[240, 240, 240, 255]), 0),
            [0, 0, 0, 48]
        );
    }
    #[test]
    fn frosted_edges_do_not_get_premultiplied_twice_or_jump_to_opaque() {
        let background = [200, 200, 200, 255];
        let source = [40, 40, 40, 112];
        let pixel = composite(&source, Some(&background), 224);
        assert_eq!(pixel, [52, 52, 52, 128]);
        // Composite against the same unblurred desktop: appearance must be
        // identical to the original source-over (apart from integer rounding).
        let over = |rgb: u8, alpha: u8| u32::from(rgb) + 200 * (255 - u32::from(alpha)) / 255;
        assert!(over(pixel[0], pixel[3]).abs_diff(over(source[0], source[3])) <= 1);
        assert!(composite(&[40, 40, 40, 209], Some(&background), 224)[3] < 255);
    }
    #[test]
    fn every_alpha_keeps_valid_pbgra_and_opaque_content_is_not_retinted() {
        for alpha in 0..=255 {
            let pixel = composite(
                &[alpha / 2, alpha / 3, alpha / 4, alpha],
                Some(&[255, 40, 128, 255]),
                224,
            );
            assert!(pixel[..3].iter().all(|c| *c <= pixel[3]));
        }
        assert_eq!(
            composite(&[18, 100, 220, 255], Some(&[10, 20, 30, 255]), 224),
            [18, 100, 220, 255]
        );
    }
}
