//! CQ WPX prefix derivation.
//!
//! WPX (Worked All Prefixes) counts unique callsign prefixes worked. The
//! prefix is the letter-and-digit lead-in of a callsign — e.g. W1AW is "W1",
//! VE3XYZ is "VE3", 9V1A is "9V1". When a call has a `/` modifier, rules
//! reshape the prefix:
//! - `/M`, `/MM`, `/AM`, `/P`, `/QRP` are operator qualifiers — ignored.
//! - `/<single-digit>` overrides the call-area digit (W1AW/3 → W3).
//! - Otherwise the shorter side is treated as the operating prefix
//!   (W1AW/VE3 → VE3, M0/W1AW → M0).
//!
//! Per CQ WPX rules: <https://www.cqwpx.com/rules.htm>

pub fn wpx_prefix(call: &str) -> Option<String> {
    let s = call.trim().to_ascii_uppercase();
    if s.is_empty() {
        return None;
    }

    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }

    if parts.len() == 1 {
        return base_wpx(parts[0]);
    }

    let real: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|p| !is_qualifier(p))
        .collect();

    match real.len() {
        0 => None,
        1 => base_wpx(real[0]),
        _ => {
            // Two or more real parts. Pick the shorter as the operating
            // prefix override (per WPX rules — it's the side that asserts
            // location). For 3+ parts (rare, e.g. VE3X/W1/M after qualifier
            // strip leaves VE3X+W1) treat the same way: pick shortest.
            let shortest = real.iter().min_by_key(|p| p.len()).copied()?;
            let longest = real.iter().max_by_key(|p| p.len()).copied()?;

            if shortest.chars().all(|c| c.is_ascii_digit()) && shortest.len() == 1 {
                let digit = shortest.chars().next().unwrap();
                return override_call_digit(longest, digit);
            }

            base_wpx(shortest)
        }
    }
}

fn is_qualifier(s: &str) -> bool {
    matches!(s, "M" | "MM" | "AM" | "P" | "QRP" | "QRPP" | "BCN")
}

fn base_wpx(call: &str) -> Option<String> {
    if call.is_empty() {
        return None;
    }
    let bytes = call.as_bytes();

    // Strip the trailing letter-only operator suffix.
    let mut split = bytes.len();
    while split > 0 && bytes[split - 1].is_ascii_alphabetic() {
        split -= 1;
    }

    if split == 0 {
        // No digit anywhere — append "0" per WPX convention for digitless
        // prefixes (rare, mostly applies to special-event calls).
        return Some(format!("{call}0"));
    }
    Some(call[..split].to_string())
}

fn override_call_digit(call: &str, new_digit: char) -> Option<String> {
    let prefix: String = call
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if prefix.is_empty() {
        return None;
    }
    Some(format!("{prefix}{new_digit}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_calls() {
        assert_eq!(wpx_prefix("W1AW").as_deref(), Some("W1"));
        assert_eq!(wpx_prefix("K1ABC").as_deref(), Some("K1"));
        assert_eq!(wpx_prefix("VE3XYZ").as_deref(), Some("VE3"));
        assert_eq!(wpx_prefix("WA6XYZ").as_deref(), Some("WA6"));
    }

    #[test]
    fn calls_with_digit_in_prefix() {
        assert_eq!(wpx_prefix("9V1A").as_deref(), Some("9V1"));
        assert_eq!(wpx_prefix("4U1ITU").as_deref(), Some("4U1"));
    }

    #[test]
    fn lower_case_normalized() {
        assert_eq!(wpx_prefix("w1aw").as_deref(), Some("W1"));
    }

    #[test]
    fn digit_suffix_overrides_call_area() {
        assert_eq!(wpx_prefix("K1ABC/0").as_deref(), Some("K0"));
        assert_eq!(wpx_prefix("W1AW/3").as_deref(), Some("W3"));
        assert_eq!(wpx_prefix("WA6XYZ/4").as_deref(), Some("WA4"));
    }

    #[test]
    fn qualifier_suffixes_ignored() {
        assert_eq!(wpx_prefix("W1AW/M").as_deref(), Some("W1"));
        assert_eq!(wpx_prefix("W1AW/MM").as_deref(), Some("W1"));
        assert_eq!(wpx_prefix("W1AW/AM").as_deref(), Some("W1"));
        assert_eq!(wpx_prefix("W1AW/P").as_deref(), Some("W1"));
        assert_eq!(wpx_prefix("K1ABC/QRP").as_deref(), Some("K1"));
    }

    #[test]
    fn prefix_override_uses_shorter_side() {
        assert_eq!(wpx_prefix("VE3X/W1").as_deref(), Some("W1"));
        assert_eq!(wpx_prefix("M0/W1AW").as_deref(), Some("M0"));
        assert_eq!(wpx_prefix("9V1/W1AW").as_deref(), Some("9V1"));
    }

    #[test]
    fn qualifier_combined_with_call_area_change() {
        // "K1ABC/4/M" — operating mobile in 4-land. /M filtered, leaves
        // K1ABC+4. Shorter is "4" (digit override) → "K4".
        assert_eq!(wpx_prefix("K1ABC/4/M").as_deref(), Some("K4"));
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(wpx_prefix(""), None);
        assert_eq!(wpx_prefix("/"), None);
        assert_eq!(wpx_prefix("   "), None);
    }

    #[test]
    fn trailing_slash_is_tolerated() {
        assert_eq!(wpx_prefix("W1AW/").as_deref(), Some("W1"));
    }

    #[test]
    fn digitless_prefix_appends_zero() {
        // Special-event style calls without a number.
        assert_eq!(wpx_prefix("ABCD").as_deref(), Some("ABCD0"));
    }
}
