use radio_core::{Band, Mode};

pub(super) const LOTW_SERVICE: &str = "lotw";
pub(super) const EQSL_SERVICE: &str = "eqsl";
pub(super) const CLUBLOG_SERVICE: &str = "clublog";
pub(super) const QRZ_SERVICE: &str = "qrz";
pub(super) const HRDLOG_SERVICE: &str = "hrdlog";

pub(super) const SPOT_HISTORY_LIMIT: usize = 100;
/// Spots older than this are filtered out of the panel display. They
/// remain in `App.spots` until eviction by the size cap, so the data
/// is still there for filtering toggles, but the operator-visible
/// view reflects current activity.
pub(super) const SPOT_MAX_AGE_SECS: u64 = 60 * 60;

pub(super) const BANDS: &[Band] = &[
    Band::M160,
    Band::M80,
    Band::M60,
    Band::M40,
    Band::M30,
    Band::M20,
    Band::M17,
    Band::M15,
    Band::M12,
    Band::M10,
    Band::M6,
    Band::M2,
    Band::Cm70,
    Band::Cm23,
];

pub(super) fn modes() -> Vec<Mode> {
    vec![
        Mode::CW,
        Mode::SSB,
        Mode::FT8,
        Mode::FT4,
        Mode::RTTY,
        Mode::PSK,
        Mode::AM,
        Mode::FM,
    ]
}
