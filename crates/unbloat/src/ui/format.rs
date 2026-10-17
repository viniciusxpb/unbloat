//! Numbers, sizes and durations as the progress window shows them.
use unbloat::i18n::Language;

/// (decimal separator, digit grouping separator)
fn separators(language: Language) -> (char, char) {
    match language {
        Language::Portuguese => (',', '.'),
        Language::English => ('.', ','),
    }
}

/// 26115 → "26.115" in Portuguese, "26,115" in English.
pub fn count(value: u64, language: Language) -> String {
    let (_, group) = separators(language);
    let digits = value.to_string();
    let mut text = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push(group);
        }
        text.push(digit);
    }
    text
}

/// Binary multiples with the labels Explorer uses and about three significant
/// digits: "1,41 GB", "44,9 MB", "449 MB", "512 bytes".
pub fn bytes(value: u64, language: Language) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if value < 1024 {
        return format!("{value} {}", UNITS[0]);
    }
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    let decimals = if size >= 99.95 {
        0
    } else if size >= 9.995 {
        1
    } else {
        2
    };
    let number = format!("{size:.decimals$}").replace('.', &separators(language).0.to_string());
    format!("{number} {}", UNITS[unit])
}

pub fn speed(bytes_per_second: f64, language: Language) -> String {
    format!("{}/s", bytes(bytes_per_second.max(0.0) as u64, language))
}

/// Elapsed time as a clock: "00:38", "12:05", "1:02:03".
pub fn clock(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// A deliberately coarse estimate: "~5 s", "~45 s", "~3 min", "~1 h 05 min".
pub fn remaining(seconds: f64) -> String {
    let seconds = seconds.max(1.0).ceil() as u64;
    if seconds < 10 {
        format!("~{seconds} s")
    } else if seconds < 60 {
        format!("~{} s", seconds.div_ceil(5) * 5)
    } else if seconds < 3600 {
        format!("~{} min", seconds.div_ceil(60))
    } else {
        let minutes = seconds.div_ceil(60);
        format!("~{} h {:02} min", minutes / 60, minutes % 60)
    }
}

/// Wall-clock "HH:MM" `later` seconds after the given local time of day.
pub fn time_of_day(hour: u16, minute: u16, second: u16, later: f64) -> String {
    let now = u64::from(hour) * 3600 + u64::from(minute) * 60 + u64::from(second);
    let minutes = (now + later.max(0.0).round() as u64) / 60 % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Splits a template around `{0}` so that value can be emphasized; any other
/// placeholders in the template are filled from `rest`, which skips index 0.
pub fn emphasis(template: &str, value: &str, rest: &[&dyn std::fmt::Display]) -> [String; 3] {
    let (before, after) = template.split_once("{0}").unwrap_or((template, ""));
    let mut args: Vec<&dyn std::fmt::Display> = vec![&""];
    args.extend_from_slice(rest);
    [
        unbloat::i18n::render(before, &args),
        value.to_owned(),
        unbloat::i18n::render(after, &args),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use Language::{English, Portuguese};

    #[test]
    fn numbers_follow_the_language() {
        assert_eq!(count(26115, Portuguese), "26.115");
        assert_eq!(count(26115, English), "26,115");
        assert_eq!(count(1234567, Portuguese), "1.234.567");
        assert_eq!(count(999, English), "999");
        assert_eq!(count(0, English), "0");
    }

    #[test]
    fn sizes_have_three_significant_digits() {
        const MB: u64 = 1024 * 1024;
        assert_eq!(bytes(512, Portuguese), "512 bytes");
        assert_eq!(bytes(1024, Portuguese), "1,00 KB");
        assert_eq!(bytes(449 * MB, Portuguese), "449 MB");
        assert_eq!(bytes(1444 * MB, Portuguese), "1,41 GB");
        assert_eq!(bytes(1444 * MB, English), "1.41 GB");
        assert_eq!(bytes(46 * MB - MB / 10, English), "45.9 MB");
        assert_eq!(bytes(1023 * MB + MB / 2, English), "1024 MB");
        assert_eq!(speed(40.3 * MB as f64, Portuguese), "40,3 MB/s");
    }

    #[test]
    fn durations_and_clock_times() {
        assert_eq!(clock(38), "00:38");
        assert_eq!(clock(80), "01:20");
        assert_eq!(clock(3723), "1:02:03");
        assert_eq!(remaining(4.2), "~5 s");
        assert_eq!(remaining(42.0), "~45 s");
        assert_eq!(remaining(130.0), "~3 min");
        assert_eq!(remaining(3900.0), "~1 h 05 min");
        assert_eq!(time_of_day(19, 3, 20, 43.0), "19:04");
        assert_eq!(time_of_day(23, 59, 0, 120.0), "00:01");
    }

    #[test]
    fn emphasis_splits_around_the_first_value() {
        assert_eq!(
            emphasis("{0} de {1}", "1,41 GB", &[&"3,01 GB"]),
            ["", "1,41 GB", " de 3,01 GB"]
        );
        assert_eq!(
            emphasis("Liberou {0} ({1}% menor)", "2,08 GB", &[&69]),
            ["Liberou ", "2,08 GB", " (69% menor)"]
        );
        assert_eq!(
            emphasis("restam {0}", "~43 s", &[]),
            ["restam ", "~43 s", ""]
        );
    }
}
