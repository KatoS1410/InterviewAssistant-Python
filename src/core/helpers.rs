use std::path::PathBuf;

/// Возвращает путь к папке с файлами Whisper (модель, DLL).
pub fn whisper_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".katos_whisper")
}

pub fn timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

pub fn to_int(value: &str, default: i32) -> i32 {
    value.trim().parse().unwrap_or(default)
}

/// Обрезает строки в `s` до `max_lines`, выкидывая самые старые (верхние).
/// Разделитель — `\n`. Если строк в пределах лимита — ничего не делает.
///
/// Нужна для логов, чтобы не раздувались бесконечно.
pub fn trim_lines(s: &mut String, max_lines: usize) {
    let total_newlines = s.bytes().filter(|&b| b == b'\n').count();
    let total_lines = total_newlines + 1;
    if total_lines <= max_lines {
        return;
    }

    let drop = total_lines - max_lines;
    let mut count = 0usize;
    let mut cut_pos = s.len();
    for (i, b) in s.bytes().enumerate() {
        if b == b'\n' {
            count += 1;
            if count == drop {
                cut_pos = i + 1;
                break;
            }
        }
    }
    if cut_pos > 0 && cut_pos < s.len() {
        s.drain(0..cut_pos);
    } else if cut_pos >= s.len() {
        s.clear();
    }
}

/// Разделитель записей в истории — невидимый PARAGRAPH SEPARATOR (U+2029).
/// В тексте AI такой символ не попадается (в отличие от `\n\n`).
pub const ENTRY_SEP: char = '\u{2029}';

/// Обрезает записи в `s` (разделённые `ENTRY_SEP`) до `max_entries`.
pub fn trim_entries(s: &mut String, max_entries: usize) {
    if s.is_empty() {
        return;
    }
    let sep_str = ENTRY_SEP.to_string();
    let separators = s.matches(&sep_str).count();
    let total_entries = separators + 1;
    if total_entries <= max_entries {
        return;
    }

    let drop = total_entries - max_entries;
    let mut count = 0usize;
    let mut cut_pos = s.len();
    let mut search_from = 0usize;
    while let Some(idx) = s[search_from..].find(&sep_str) {
        count += 1;
        let abs_idx = search_from + idx;
        if count == drop {
            cut_pos = abs_idx + ENTRY_SEP.len_utf8();
            break;
        }
        search_from = abs_idx + ENTRY_SEP.len_utf8();
    }
    if cut_pos > 0 && cut_pos < s.len() {
        s.drain(0..cut_pos);
    } else if cut_pos >= s.len() {
        s.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_lines_keeps_short() {
        let mut s = String::from("a\nb\nc");
        trim_lines(&mut s, 5);
        assert_eq!(s, "a\nb\nc");
    }

    #[test]
    fn trim_lines_drops_oldest() {
        let mut s = String::from("a\nb\nc\nd\ne");
        trim_lines(&mut s, 3);
        assert_eq!(s, "c\nd\ne");
    }

    #[test]
    fn trim_entries_keeps_short() {
        let mut s = String::from("q1\u{2029}q2\u{2029}q3");
        trim_entries(&mut s, 5);
        assert_eq!(s, "q1\u{2029}q2\u{2029}q3");
    }

    #[test]
    fn trim_entries_drops_oldest() {
        let mut s = String::from("q1\u{2029}q2\u{2029}q3\u{2029}q4\u{2029}q5");
        trim_entries(&mut s, 3);
        assert_eq!(s, "q3\u{2029}q4\u{2029}q5");
    }

    #[test]
    fn trim_entries_with_internal_newlines() {
        let mut s = String::from("q1\n\npara\u{2029}q2");
        trim_entries(&mut s, 1);
        assert_eq!(s, "q2");
    }
}