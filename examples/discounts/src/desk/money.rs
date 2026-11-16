//! Dollars are kept in cents (u64), so sums never round.

/// "$70,800" for whole dollars, "$108.75" otherwise.
pub fn dollars(cents: u64) -> String {
    let whole = group(cents / 100);
    match cents % 100 {
        0 => format!("${whole}"),
        c => format!("${whole}.{c:02}"),
    }
}

/// "$58k", "$1.2M": for tight spots like the quarter's numbers.
pub fn dollars_short(cents: u64) -> String {
    let d = cents / 100;
    if d >= 1_000_000 {
        format!("${:.1}M", d as f64 / 1_000_000.0)
    } else if d >= 10_000 {
        format!("${}k", (d + 500) / 1000)
    } else {
        dollars(cents - cents % 100)
    }
}

/// 120000 -> "120,000".
pub fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_dollars() {
        assert_eq!(dollars(7_080_000), "$70,800");
        assert_eq!(dollars(10_875), "$108.75");
        assert_eq!(dollars(5), "$0.05");
        assert_eq!(dollars(0), "$0");
        assert_eq!(dollars(11_611_200), "$116,112");
        assert_eq!(dollars(10_000_000_000), "$100,000,000");
    }

    #[test]
    fn short_dollars() {
        assert_eq!(dollars_short(5_805_600), "$58k");
        assert_eq!(dollars_short(123_456_700), "$1.2M");
        assert_eq!(dollars_short(950_050), "$9,500");
    }
}
