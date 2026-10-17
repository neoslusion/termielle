use super::text_metrics;

fn prefix_end(text: &str, width: u32, font: i32, bold: bool, scale: f32) -> usize {
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(Some(text.len()))
        .collect();
    let mut lower = 0;
    let mut upper = boundaries.len();
    while lower + 1 < upper {
        let middle = (lower + upper) / 2;
        if text_metrics::width(&text[..boundaries[middle]], font, bold, scale) <= width {
            lower = middle;
        } else {
            upper = middle;
        }
    }
    boundaries[lower]
}

pub(crate) fn lines(
    text: &str,
    width: u32,
    font: i32,
    bold: bool,
    scale: f32,
    limit: usize,
) -> Vec<String> {
    if width == 0 || limit == 0 {
        return Vec::new();
    }
    let bounded: String = text.chars().take(512).collect();
    let mut normalized = bounded.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().nth(512).is_some() {
        normalized.push('…');
    }
    let mut remaining = normalized.as_str();
    let mut result = Vec::new();
    while !remaining.is_empty() && result.len() < limit {
        let end = prefix_end(remaining, width, font, bold, scale);
        if end == remaining.len() {
            result.push(remaining.to_owned());
            break;
        }
        if result.len() + 1 == limit {
            let ellipsis = text_metrics::width("…", font, bold, scale);
            if ellipsis > width {
                break;
            }
            let end = prefix_end(remaining, width - ellipsis, font, bold, scale);
            result.push(format!("{}…", remaining[..end].trim_end()));
            break;
        }
        if end == 0 {
            break;
        }
        let split = remaining[..end]
            .rfind(' ')
            .filter(|offset| *offset > 0)
            .unwrap_or(end);
        result.push(remaining[..split].trim_end().to_owned());
        remaining = remaining[split..].trim_start();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrapping_is_bounded_unicode_safe_and_measured_at_multiple_scales() {
        for scale in [1.0, 1.25, 2.0] {
            for text in [
                "A notification that needs several readable lines instead of a clipped title",
                "Résumé — Tiếng Việt 日本語 📚",
                "超長標題没有空格也可以分行",
                &"x".repeat(2000),
            ] {
                let wrapped = lines(text, 130, 13, true, scale, 2);
                assert!(!wrapped.is_empty() && wrapped.len() <= 2);
                assert!(
                    wrapped
                        .iter()
                        .all(|line| text_metrics::width(line, 13, true, scale) <= 130)
                );
            }
        }
        assert!(
            lines(&"Long title ".repeat(30), 100, 13, false, 1.0, 2)
                .last()
                .unwrap()
                .ends_with('…')
        );
        assert!(lines("hello", 0, 13, false, 1.0, 2).is_empty());
        assert!(lines("hello", 100, 13, false, 1.0, 0).is_empty());
        assert_eq!(lines("  one\n two  ", 200, 13, false, 1.0, 2), ["one two"]);
    }
}
