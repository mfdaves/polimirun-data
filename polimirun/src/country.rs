//! Nationality codes as IOC codes.
//!
//! Timing companies publish nationality in different ways: IOC codes (`GER`),
//! ISO 3166 codes (`DEU`) or the first letters of the Italian name (`SPA` for
//! Spagna, `SVI` for Svizzera), sometimes cut or accented (`EL`, `MÉX`). Only
//! codes found in Polimirun results whose meaning is certain are mapped;
//! anything else is kept as published (`COR`, `UNI`, `REP`, `MAL`, `LIB`...).

/// Published code and the IOC code it stands for.
const CODES: [(&str, &str); 60] = [
    // ISO 3166 codes that differ from IOC ones
    ("BGR", "BUL"),
    ("CHE", "SUI"),
    ("CHL", "CHI"),
    ("CRI", "CRC"),
    ("DEU", "GER"),
    ("DNK", "DEN"),
    ("GRC", "GRE"),
    ("HRV", "CRO"),
    ("IDN", "INA"),
    ("IRN", "IRI"),
    ("LBY", "LBA"),
    ("LKA", "SRI"),
    ("LVA", "LAT"),
    ("MUS", "MRI"),
    ("NLD", "NED"),
    ("PHL", "PHI"),
    ("PRT", "POR"),
    ("SDN", "SUD"),
    ("SLV", "ESA"),
    ("TWN", "TPE"),
    ("VNM", "VIE"),
    // Former IOC code
    ("ROM", "ROU"),
    // Italian names
    ("BIE", "BLR"), // Bielorussia
    ("BOS", "BIH"), // Bosnia
    ("CEC", "CZE"), // Ceca
    ("CIL", "CHI"), // Cile
    ("CIN", "CHN"), // Cina
    ("DAN", "DEN"), // Danimarca
    ("EGI", "EGY"), // Egitto
    ("ETI", "ETH"), // Etiopia
    ("FED", "RUS"), // Federazione Russa
    ("FIL", "PHI"), // Filippine
    ("GIA", "JPN"), // Giappone
    ("GIO", "JOR"), // Giordania
    // Italian "Iran" or "Iraq": only in 2018, 2021 and 2022, years with no IRI
    // and no IRQ, and in the numbers other years show for Iran.
    ("IRA", "IRI"),
    ("LUS", "LUX"), // Lussemburgo
    ("MES", "MEX"), // Messico
    ("MOL", "MDA"), // Moldavia
    ("PAE", "NED"), // Paesi Bassi
    ("REG", "GBR"), // Regno Unito
    ("SAN", "SMR"), // San Marino
    ("SER", "SRB"), // Serbia
    ("SIR", "SYR"), // Siria
    ("SPA", "ESP"), // Spagna
    ("STA", "USA"), // Stati Uniti
    ("SVE", "SWE"), // Svezia
    ("SVI", "SUI"), // Svizzera
    ("TAI", "TPE"), // Taiwan
    ("UCR", "UKR"), // Ucraina
    ("UNG", "HUN"), // Ungheria
    // English and Spanish names
    ("IRE", "IRL"), // Ireland
    ("JAP", "JPN"), // Japan
    ("LEB", "LBN"), // Lebanon
    ("NET", "NED"), // Netherlands
    ("SWI", "SUI"), // Switzerland
    ("BÉL", "BEL"), // Bélgica
    ("IRÁ", "IRI"), // Irán
    ("MÉX", "MEX"), // México
    ("PAÍ", "NED"), // Países Bajos
    ("EL", "ESA"),  // El Salvador, cut to its first word
];

/// The IOC code for a published nationality, or the nationality itself.
pub fn ioc(nationality: &str) -> &str {
    CODES.iter().find(|(code, _)| *code == nationality).map_or(nationality, |(_, ioc)| ioc)
}
