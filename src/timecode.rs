//! Parsing and formatting of the timestamps typed into the trim boxes.
//!
//! Accepted input: `25`, `25.5`, `1:25`, `1:25.250`, `0:01:25.500`.

/// Parse a user-typed timestamp into seconds.
pub fn parse(input: &str) -> Result<f64, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("Enter a time".into());
    }
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() > 3 {
        return Err("Too many ':' — use h:mm:ss".into());
    }
    let mut total = 0.0;
    let last = parts.len() - 1;
    for (i, part) in parts.iter().enumerate() {
        let part = part.trim();
        if part.is_empty() {
            return Err("Missing number around ':'".into());
        }
        let value: f64 = if i == last {
            if !part.chars().all(|c| c.is_ascii_digit() || c == '.') || part.matches('.').count() > 1 {
                return Err(format!("'{part}' isn't a number"));
            }
            part.parse().map_err(|_| format!("'{part}' isn't a number"))?
        } else {
            if !part.chars().all(|c| c.is_ascii_digit()) {
                return Err("Only the seconds can have decimals".into());
            }
            part.parse().map_err(|_| format!("'{part}' isn't a number"))?
        };
        // Minutes and seconds must be under 60 when a larger unit is present.
        if i > 0 && value >= 60.0 {
            return Err(format!("'{part}' must be under 60"));
        }
        total = total * 60.0 + value;
    }
    Ok(total)
}

/// Format seconds as `h:mm:ss.mmm`.
pub fn format(seconds: f64) -> String {
    let ms_total = (seconds.max(0.0) * 1000.0).round() as u64;
    let ms = ms_total % 1000;
    let s = (ms_total / 1000) % 60;
    let m = (ms_total / 60_000) % 60;
    let h = ms_total / 3_600_000;
    format!("{h}:{m:02}:{s:02}.{ms:03}")
}

/// Short form for display next to the player, e.g. `1:05` or `1:02:03`.
pub fn format_short(seconds: f64) -> String {
    let t = seconds.max(0.0).floor() as u64;
    let (h, m, s) = (t / 3600, (t / 60) % 60, t % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Shortest exact form for writing back into a trim box: `25`, `25.5`, `1:05.25`, `1:02:03`.
pub fn format_compact(seconds: f64) -> String {
    let ms_total = (seconds.max(0.0) * 1000.0).round() as u64;
    let ms = ms_total % 1000;
    let s = (ms_total / 1000) % 60;
    let m = (ms_total / 60_000) % 60;
    let h = ms_total / 3_600_000;
    let frac = if ms == 0 { String::new() } else { format!(".{ms:03}").trim_end_matches('0').to_string() };
    if h > 0 {
        format!("{h}:{m:02}:{s:02}{frac}")
    } else if m > 0 {
        format!("{m}:{s:02}{frac}")
    } else {
        format!("{s}{frac}")
    }
}

/// Compact form for file names, e.g. `25`, `25.5`, `1m05s`, `1h02m03s`.
pub fn format_for_filename(seconds: f64) -> String {
    let whole = seconds.floor() as u64;
    let frac = ((seconds - whole as f64) * 10.0).round() as u64;
    let (h, m, s) = (whole / 3600, (whole / 60) % 60, whole % 60);
    let secs = if frac > 0 && frac < 10 { format!("{s}.{frac}") } else { format!("{s}") };
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{:0>2}s", secs)
    } else {
        secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_forms() {
        assert_eq!(parse("25").unwrap(), 25.0);
        assert_eq!(parse(" 25.5 ").unwrap(), 25.5);
        assert_eq!(parse("1:25").unwrap(), 85.0);
        assert_eq!(parse("0:01:25.500").unwrap(), 85.5);
        assert_eq!(parse("90").unwrap(), 90.0);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse("").is_err());
        assert!(parse("1:75").is_err());
        assert!(parse("1.5:20").is_err());
        assert!(parse("abc").is_err());
        assert!(parse("1:2:3:4").is_err());
        assert!(parse("1::2").is_err());
        assert!(parse("-5").is_err());
    }

    #[test]
    fn formats() {
        assert_eq!(format(85.5), "0:01:25.500");
        assert_eq!(format(3725.25), "1:02:05.250");
        assert_eq!(format_short(65.9), "1:05");
        assert_eq!(format_compact(25.5), "25.5");
        assert_eq!(format_compact(65.25), "1:05.25");
        assert_eq!(parse(&format_compact(3723.4)).unwrap(), 3723.4);
        assert_eq!(format_for_filename(25.0), "25");
        assert_eq!(format_for_filename(25.5), "25.5");
        assert_eq!(format_for_filename(65.0), "1m05s");
    }
}
