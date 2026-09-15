use ratatui_textarea::{CursorMove, TextArea};

pub fn char_display_width(c: char) -> usize {
    use unicode_width::UnicodeWidthChar;
    c.width().unwrap_or(0)
}

pub fn line_display_width(chars: &[char]) -> usize {
    chars.iter().map(|&c| char_display_width(c)).sum()
}

/// Choose where to split an over-long line so the first part fits `max_cols`
/// columns. Prefers the last space that fits (the space is consumed by the
/// break), falling back to an exact column split at a char boundary.
/// Returns `(split_index, drop_space)`.
pub fn wrap_split(chars: &[char], max_cols: usize) -> (usize, bool) {
    let mut width = 0usize;
    let mut fitting = 0usize;
    let mut last_space: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        let w = char_display_width(c);
        if width + w > max_cols {
            break;
        }
        width += w;
        fitting = i + 1;
        if c == ' ' && i >= 1 {
            last_space = Some(i);
        }
    }
    match last_space {
        Some(i) => (i, true),
        None => (fitting.max(1), false),
    }
}

/// Insert hard newlines so the cursor's line stops exceeding `max_cols` columns.
/// Only acts when the cursor sits at the end of its line (the tail-typing case
/// that fills the input box).
pub fn wrap_cursor_line(ta: &mut TextArea<'_>, max_cols: usize) {
    if max_cols == 0 {
        return;
    }
    loop {
        let cur = ta.cursor();
        let (row, col) = (cur.0, cur.1);
        let chars: Vec<char> = ta.lines()[row].chars().collect();
        if col != chars.len() || line_display_width(&chars) <= max_cols {
            break;
        }
        let (split, drop_space) = wrap_split(&chars, max_cols);
        if split == 0 || split >= chars.len() {
            break;
        }
        ta.move_cursor(CursorMove::Jump(row as u16, split as u16));
        if drop_space {
            ta.delete_next_char();
        }
        ta.insert_newline();
        ta.move_cursor(CursorMove::End);
    }
}

/// Hard-wrap text before insertion (e.g. bracketed paste): each logical line is
/// wrapped independently at `max_cols` columns.
pub fn hard_wrap_str(text: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return text.to_string();
    }
    text.split('\n')
        .map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let mut out = String::new();
            let mut rest: Vec<char> = line.chars().collect();
            while line_display_width(&rest) > max_cols {
                let (split, drop_space) = wrap_split(&rest, max_cols);
                if split == 0 || split >= rest.len() {
                    break;
                }
                out.extend(rest[..split].iter());
                out.push('\n');
                let skip = if drop_space { split + 1 } else { split };
                rest = rest[skip.min(rest.len())..].to_vec();
            }
            out.extend(rest.iter());
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_prefers_word_boundary_and_drops_break_space() {
        assert_eq!(hard_wrap_str("hello world foo", 10), "hello\nworld foo");
    }

    #[test]
    fn wrap_falls_back_to_exact_column_without_stripping_chars() {
        assert_eq!(hard_wrap_str("abcdefghij", 4), "abcd\nefgh\nij");
    }

    #[test]
    fn wrap_measures_display_width_for_wide_chars() {
        assert_eq!(hard_wrap_str("ab犬猫", 4), "ab犬\n猫");
        assert_eq!(hard_wrap_str("ab犬猫", 3), "ab\n犬\n猫");
    }

    #[test]
    fn wrap_leaves_fitting_text_untouched() {
        assert_eq!(hard_wrap_str("hello", 10), "hello");
        assert_eq!(hard_wrap_str("ab\ncd", 10), "ab\ncd");
    }

    #[test]
    fn wrap_cursor_line_wraps_tail_typing() {
        let mut ta = TextArea::from(["hello world"]);
        ta.move_cursor(CursorMove::End);
        wrap_cursor_line(&mut ta, 6);
        assert_eq!(ta.lines().to_vec(), vec!["hello", "world"]);
        assert_eq!(ta.cursor(), (1, 5));
    }

    #[test]
    fn wrap_cursor_line_ignores_cursor_away_from_line_end() {
        let mut ta = TextArea::from(["hello world"]);
        ta.move_cursor(CursorMove::Head);
        wrap_cursor_line(&mut ta, 6);
        assert_eq!(ta.lines().to_vec(), vec!["hello world"]);
    }

    #[test]
    fn wrap_cursor_line_uses_char_boundaries_for_long_tokens() {
        let mut ta = TextArea::from(["abcdefgh"]);
        ta.move_cursor(CursorMove::End);
        wrap_cursor_line(&mut ta, 4);
        assert_eq!(ta.lines().to_vec(), vec!["abcd", "efgh"]);
    }
}
