//! ARRL DXCC entity numeric ID lookup, keyed by the prefix string returned
//! by `station_data::CtyDb`. Numbers come from the ARRL DXCC list — they
//! identify the entity for awards purposes (the DXCC entity is what
//! "Worked All DXCC" / "DXCC Challenge" / "Marathon" all join on).
//!
//! cty.dat does not carry these numeric IDs. station-data returns the
//! *matched* prefix from cty.dat's rule list (e.g. "W" for W1AW, not the
//! canonical "K"), so this table maps the variants directly.
//!
//! Coverage: ~70 most-active prefixes, accounting for the great majority
//! of HF QSOs. Returns `None` for unknown — caller should treat that as a
//! hint, not authoritative truth. Extend the match as needed.

/// Map a station-data prefix string to ARRL DXCC entity ID. Comparison is
/// case-insensitive.
pub fn dxcc_id_for_prefix(prefix: &str) -> Option<u16> {
    let p = prefix.trim().to_ascii_uppercase();
    Some(match p.as_str() {
        // North America
        "K" | "W" | "N" | "AA" | "AB" | "AC" | "AD" | "AE" | "AF" | "AG"
        | "AH" | "AI" | "AJ" | "AK" | "AL" => 291, // United States
        "VE" | "VA" | "VO" | "VY" => 1,            // Canada
        "XE" | "XF" | "4A" | "4B" | "4C" | "6D" | "6E" | "6F" | "6G" | "6H" | "6I" => 50, // Mexico
        "CO" | "CL" | "CM" | "T4" => 70,           // Cuba
        "FP" => 277,                                // St. Pierre & Miquelon
        "FM" => 84,                                 // Martinique
        "FG" => 79,                                 // Guadeloupe
        "VP9" => 64,                                // Bermuda
        "ZF" => 69,                                 // Cayman
        "C6" => 60,                                 // Bahamas
        // South America
        "PY" | "PP" | "PR" | "PS" | "PT" | "PU" | "PV" | "PW" => 108, // Brazil
        "LU" | "AY" | "AZ" | "L2" | "L3" | "L4" | "L5" | "L6" | "L7" | "L8" | "L9" => 100, // Argentina
        "CE" | "XQ" | "3G" => 112,                                                          // Chile
        "OA" => 136,                                                                        // Peru
        "HC" => 120,                                                                        // Ecuador
        "CX" => 144,                                                                        // Uruguay
        "ZP" => 132,                                                                        // Paraguay
        "CP" => 104,                                                                        // Bolivia
        "HK" => 116,                                                                        // Colombia
        "YV" | "YW" | "YX" | "YY" | "4M" => 148,                                            // Venezuela
        // Europe
        "G" | "M" | "2E" => 223,                                  // England
        "GW" | "MW" | "2W" => 294,                                // Wales
        "GM" | "MM" | "2M" => 279,                                // Scotland
        "GD" | "MD" | "2D" => 114,                                // Isle of Man
        "GI" | "MI" | "2I" => 265,                                // Northern Ireland
        "GJ" | "MJ" | "2J" => 122,                                // Jersey
        "GU" | "MU" | "2U" => 106,                                // Guernsey
        "EI" | "EJ" => 245,                                       // Ireland
        "DL" | "DA" | "DB" | "DC" | "DD" | "DF" | "DG" | "DH"
        | "DJ" | "DK" | "DM" | "DN" | "DO" | "DP" | "DQ" | "DR" => 230, // Germany
        "F" | "TM" => 227,                                        // France
        "I" | "IK" | "IZ" | "IW" | "IV" | "IU" | "IN" | "IR" => 248, // Italy
        "EA" | "EB" | "EC" | "ED" | "EE" | "EF" | "EG" | "EH" => 281, // Spain
        "EA6" | "EB6" | "EC6" => 21,                              // Balearic
        "EA8" | "EB8" | "EC8" => 29,                              // Canary
        "EA9" | "EB9" | "EC9" => 32,                              // Ceuta & Melilla
        "CT" | "CR" | "CS" => 272,                                // Portugal
        "CT3" | "CR3" => 256,                                     // Madeira
        "CU" => 149,                                              // Azores
        "ON" | "OO" | "OP" | "OQ" | "OR" | "OS" | "OT" => 209,    // Belgium
        "PA" | "PB" | "PC" | "PD" | "PE" | "PF" | "PG" | "PH"
        | "PI" => 263,                                            // Netherlands
        "LX" => 254,                                              // Luxembourg
        "OE" => 206,                                              // Austria
        "HB9" | "HB3" => 287,                                     // Switzerland
        "HB0" => 251,                                             // Liechtenstein
        "OK" | "OL" => 503,                                       // Czech Republic
        "OM" => 504,                                              // Slovakia
        "HA" | "HG" => 239,                                       // Hungary
        "SM" | "SA" | "SB" | "SC" | "SD" | "SE" | "SF" | "SG"
        | "SH" | "SI" | "SJ" | "SK" | "SL" | "8S" | "7S" => 284,  // Sweden
        "LA" | "LB" | "LC" | "LD" | "LE" | "LF" | "LG" | "LH"
        | "LI" | "LJ" | "LK" | "LL" | "LM" | "LN" => 266,         // Norway
        "OZ" | "OU" | "OV" | "OW" | "5P" | "5Q" => 221,           // Denmark
        "OH" | "OF" | "OG" | "OI" => 224,                         // Finland
        "SP" | "SN" | "SO" | "SQ" | "3Z" => 269,                  // Poland
        "OK1" => 503,                                             // (kept for safety)
        "UA" | "RA" | "RC" | "RD" | "RE" | "RF" | "RG" | "RJ"
        | "RK" | "RL" | "RM" | "RN" | "RO" | "RP" | "RQ" | "RT"
        | "RU" | "RV" | "RW" | "RX" | "RY" | "RZ" | "R" => 54,    // European Russia (rough)
        "UA9" | "RA9" => 15,                                      // Asiatic Russia
        "YL" => 145,                                              // Latvia
        "LY" => 146,                                              // Lithuania
        "ES" => 52,                                               // Estonia
        "UR" | "US" | "UT" | "UU" | "UV" | "UW" | "UX" | "UY"
        | "UZ" | "EM" | "EN" | "EO" => 288,                       // Ukraine
        "EU" | "EV" | "EW" => 27,                                 // Belarus
        "ER" => 179,                                              // Moldova
        "9A" => 497,                                              // Croatia
        "S5" => 499,                                              // Slovenia
        "Z3" => 502,                                              // North Macedonia
        "E7" => 501,                                              // Bosnia
        "YT" | "YU" => 296,                                       // Serbia
        "ZA" => 7,                                                // Albania
        "SV" => 236,                                              // Greece
        "SV5" => 45,                                              // Dodecanese
        "SV9" => 40,                                              // Crete
        "5B" | "C4" => 215,                                       // Cyprus
        "TA" | "TC" => 390,                                       // Turkey
        "9H" => 257,                                              // Malta
        "1A" => 246,                                              // SMOM
        // Asia
        "JA" | "JE" | "JF" | "JG" | "JH" | "JI" | "JJ" | "JK"
        | "JL" | "JM" | "JN" | "JO" | "JP" | "JQ" | "JR" | "JS"
        | "7J" | "7K" | "7L" | "7M" | "7N" | "8J" | "8N" => 339,  // Japan
        "BY" | "BG" | "BA" | "BD" | "BH" | "BI" | "BJ" | "BT" => 318, // China
        "BV" => 386,                                              // Taiwan
        "HL" | "DS" | "6K" | "6L" | "6M" | "6N" => 137,           // South Korea
        "P5" => 344,                                              // North Korea
        "VU" => 324,                                              // India
        "AP" => 372,                                              // Pakistan
        "S2" => 305,                                              // Bangladesh
        "9V" => 381,                                              // Singapore
        "9M2" | "9W2" => 299,                                     // West Malaysia
        "9M6" | "9M8" | "9W6" | "9W8" => 46,                      // East Malaysia
        "HS" | "E2" => 387,                                       // Thailand
        "XV" | "3W" => 293,                                       // Vietnam
        "DU" | "DV" | "DW" | "DX" | "DY" | "DZ" | "4D" | "4E"
        | "4F" | "4G" | "4H" | "4I" => 375,                       // Philippines
        "YB" | "YC" | "YD" | "YE" | "YF" | "YG" | "YH" => 327,    // Indonesia
        "VR" => 321,                                              // Hong Kong
        "XX9" => 152,                                             // Macao
        "4X" | "4Z" => 336,                                       // Israel
        "JY" => 342,                                              // Jordan
        "OD" => 354,                                              // Lebanon
        "YK" => 384,                                              // Syria
        "YI" => 333,                                              // Iraq
        "EP" | "EQ" => 330,                                       // Iran
        "A4" => 370,                                              // Oman
        "A6" => 391,                                              // UAE
        "A7" => 376,                                              // Qatar
        "A9" => 304,                                              // Bahrain
        "9K" => 348,                                              // Kuwait
        "HZ" | "7Z" | "8Z" => 378,                                // Saudi Arabia
        // Africa
        "ZS" | "ZR" => 462,                                       // South Africa
        "5N" => 450,                                              // Nigeria
        "5R" | "6X" => 438,                                       // Madagascar
        "SU" => 478,                                              // Egypt
        "7X" => 400,                                              // Algeria
        "CN" => 446,                                              // Morocco
        "3V" => 474,                                              // Tunisia
        "5A" => 436,                                              // Libya
        "5T" => 444,                                              // Mauritania
        "ST" => 466,                                              // Sudan
        "ET" => 53,                                               // Ethiopia
        "5Z" => 430,                                              // Kenya
        "5H" => 470,                                              // Tanzania
        "C9" => 181,                                              // Mozambique
        "Z2" => 452,                                              // Zimbabwe
        "9J" => 482,                                              // Zambia
        "V5" => 464,                                              // Namibia
        // Oceania
        "VK" | "AX" => 150,                                       // Australia
        "ZL" => 170,                                              // New Zealand
        "KH6" | "AH6" | "WH6" | "NH6" => 110,                     // Hawaii
        "KL7" | "AL7" | "WL7" | "NL7" => 6,                       // Alaska
        "KP4" | "WP4" | "NP4" => 202,                             // Puerto Rico
        "KP2" | "WP2" | "NP2" => 285,                             // US Virgin Is.
        "VP2E" => 12,                                             // Anguilla
        "VP2M" => 96,                                             // Montserrat
        "VP2V" => 65,                                             // British Virgin Is.
        "PJ7" => 519,                                             // Sint Maarten
        "FK" => 162,                                              // New Caledonia
        "YJ" => 158,                                              // Vanuatu
        "T8" => 22,                                               // Palau
        // Antarctic / Arctic / oceans
        "OX" => 237,                                              // Greenland
        "TF" => 242,                                              // Iceland
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_prefixes_resolve() {
        assert_eq!(dxcc_id_for_prefix("K"), Some(291));
        assert_eq!(dxcc_id_for_prefix("W"), Some(291));
        assert_eq!(dxcc_id_for_prefix("VE"), Some(1));
        assert_eq!(dxcc_id_for_prefix("JA"), Some(339));
        assert_eq!(dxcc_id_for_prefix("DL"), Some(230));
        assert_eq!(dxcc_id_for_prefix("G"), Some(223));
        assert_eq!(dxcc_id_for_prefix("VK"), Some(150));
    }

    #[test]
    fn case_insensitive_and_trimmed() {
        assert_eq!(dxcc_id_for_prefix("w"), Some(291));
        assert_eq!(dxcc_id_for_prefix(" JA "), Some(339));
    }

    #[test]
    fn unknown_returns_none() {
        assert_eq!(dxcc_id_for_prefix("ZZ"), None);
        assert_eq!(dxcc_id_for_prefix(""), None);
    }

    #[test]
    fn alaska_distinct_from_lower_48() {
        // Alaska (KL7) is its own DXCC entity (#6), not USA (#291).
        // station-data returns "KL7" for Alaska calls.
        assert_eq!(dxcc_id_for_prefix("KL7"), Some(6));
        assert_ne!(dxcc_id_for_prefix("KL7"), dxcc_id_for_prefix("K"));
    }

    #[test]
    fn hawaii_distinct_from_mainland() {
        assert_eq!(dxcc_id_for_prefix("KH6"), Some(110));
        assert_ne!(dxcc_id_for_prefix("KH6"), dxcc_id_for_prefix("K"));
    }
}
