//! Age grading of 10 km road times.
//!
//! The standards are Alan Jones's 2025 road age standards (single-age bests by
//! Tom Bernhard), approved on 2025-01-10 by the USATF Masters Long Distance
//! Running Council: https://github.com/AlanLyttonJones/Age-Grade-Tables,
//! `2025 Files/MaleRoadStd2025.xlsx` and `FemaleRoadStd2025.xlsx` (version
//! 2025-07-27, commit 061cb85), "10 km" column of the standards in seconds.
//! Released under CC0 1.0.

/// 10 km standard in seconds for men aged 5 to 100.
const MEN: [u16; 96] = [
    3122, 2790, 2537, 2340, 2184, 2058, 1955, 1871, 1801, 1745, 1699, 1662,
    1634, 1613, 1600, 1589, 1584, 1584, 1584, 1584, 1584, 1584, 1584, 1584,
    1584, 1584, 1585, 1586, 1589, 1593, 1599, 1605, 1613, 1622, 1632, 1644,
    1657, 1670, 1683, 1697, 1710, 1724, 1739, 1753, 1768, 1783, 1798, 1813,
    1829, 1845, 1861, 1878, 1895, 1912, 1929, 1947, 1965, 1983, 2002, 2021,
    2041, 2061, 2081, 2102, 2123, 2145, 2167, 2193, 2221, 2252, 2286, 2324,
    2365, 2410, 2460, 2514, 2573, 2638, 2710, 2789, 2876, 2972, 3080, 3199,
    3333, 3484, 3655, 3849, 4073, 4331, 4634, 4994, 5427, 5955, 6617, 7468,
];

/// 10 km standard in seconds for women aged 5 to 100.
const WOMEN: [u16; 96] = [
    2520, 2403, 2302, 2215, 2138, 2071, 2013, 1962, 1917, 1878, 1844, 1813,
    1783, 1758, 1740, 1729, 1726, 1726, 1726, 1726, 1726, 1726, 1726, 1726,
    1728, 1729, 1732, 1736, 1740, 1745, 1751, 1758, 1765, 1774, 1783, 1794,
    1805, 1817, 1831, 1845, 1860, 1877, 1895, 1914, 1935, 1956, 1979, 2002,
    2025, 2049, 2073, 2098, 2124, 2150, 2177, 2205, 2233, 2262, 2292, 2323,
    2354, 2387, 2420, 2454, 2489, 2526, 2563, 2601, 2641, 2681, 2724, 2769,
    2819, 2874, 2936, 3004, 3080, 3164, 3257, 3361, 3478, 3608, 3755, 3920,
    4109, 4325, 4573, 4863, 5203, 5609, 6101, 6706, 7469, 8457, 9790, 12172,
];

/// A 10 km time as a percentage of the standard for `gender` (`M` or `F`)
/// and `age`, rounded to two decimals: 100 matches the standard, and above
/// 100 beats it. `None` for other genders and ages outside 5 to 100.
pub fn percent(gender: &str, age: u16, seconds: u32) -> Option<f64> {
    let standards = match gender {
        "M" => &MEN,
        "F" => &WOMEN,
        _ => return None,
    };
    let standard = standards.get(usize::from(age.checked_sub(5)?))?;
    Some((f64::from(*standard) / f64::from(seconds) * 10_000.0).round() / 100.0)
}
