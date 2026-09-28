//! Bar text helper: middle-ellipsize for the active-window title.

/// Ellipsize `text` to `max_chars`, keeping head and tail around one `…`.
/// Long `HOST: session` titles keep both ends instead of losing the
/// distinctive tail to a head cut. Unicode-safe (char boundaries).
pub(crate) fn ellipsize_middle(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars || max_chars < 4 {
        return text.to_string();
    }
    let tail = 14.min(max_chars - 2);
    let head = max_chars - tail - 1;
    let head_str: String = text.chars().take(head).collect();
    let tail_str: String = text.chars().skip(count - tail).collect();
    format!("{head_str}…{tail_str}")
}
#[test]
fn ellipsize_middle_keeps_head_and_tail() {
    assert_eq!(ellipsize_middle("short", 36), "short");
    assert_eq!(ellipsize_middle(&"x".repeat(36), 36).chars().count(), 36);
    assert_eq!(ellipsize_middle(&"x".repeat(37), 36).chars().count(), 36);
    let long = "DESKTOP-LGG1QFB: some-very-long-session-name-here";
    let cut = ellipsize_middle(long, 36);
    assert_eq!(cut.chars().count(), 36);
    assert!(cut.starts_with("DESKTOP-LGG1QFB: so"), "head kept: {cut}");
    assert!(cut.ends_with("sion-name-here"), "tail kept: {cut}");
    assert!(cut.contains('…'));
    // Unicode-safe: char boundaries only.
    let uni = "WezTerm — session with shades and more text here plus";
    let ucut = ellipsize_middle(uni, 36);
    assert_eq!(ucut.chars().count(), 36);
    assert!(ucut.contains('…'));
}
