//! The Greyline panel: a world map with the night side shaded, the
//! terminator (the greyline) drawn across it, and the station marked.
//!
//! Everything is worked out locally from the clock. The sun's position
//! uses the low-precision formulas of the Astronomical Almanac, good to
//! about 0.01° for decades either side of 2000, which is far finer than
//! a pixel of this map.
//!
//! The map is equirectangular (longitude across, latitude up), so the
//! land outlines need no projection maths. They are Natural Earth's
//! 1:110m land polygons (public domain), in `assets/land-110m.txt`: one
//! ring per line, as `lon,lat` pairs.

use std::sync::OnceLock;

use chrono::{DateTime, Duration, Utc};
use iced::widget::canvas::{self, Cache, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme, mouse};
use radio_core::StationLocation;

use super::theme;

/// Where the sun is overhead, and how far north or south.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Sun {
    /// Declination: the latitude of the subsolar point, degrees.
    pub declination: f64,
    /// Longitude of the subsolar point, degrees east, -180..=180.
    pub longitude: f64,
}

impl Sun {
    pub(super) fn at(when: DateTime<Utc>) -> Self {
        // Days (and fractions) since 2000-01-01 12:00 UTC.
        let n = (when.timestamp_millis() as f64 / 1000.0 - 946_728_000.0) / 86_400.0;
        let mean_longitude = 280.460 + 0.985_647_4 * n;
        let mean_anomaly = (357.528 + 0.985_600_3 * n).to_radians();
        let ecliptic_longitude = (mean_longitude
            + 1.915 * mean_anomaly.sin()
            + 0.020 * (2.0 * mean_anomaly).sin())
        .to_radians();
        let obliquity = (23.439 - 0.000_000_4 * n).to_radians();

        let right_ascension = (obliquity.cos() * ecliptic_longitude.sin())
            .atan2(ecliptic_longitude.cos())
            .to_degrees();
        let declination = (obliquity.sin() * ecliptic_longitude.sin()).asin().to_degrees();
        // Greenwich mean sidereal time, in degrees.
        let sidereal = 15.0 * (18.697_374_558 + 24.065_709_824_419_08 * n);

        Self { declination, longitude: wrap_longitude(right_ascension - sidereal) }
    }

    /// Sine of the sun's altitude above the horizon at a place.
    fn sin_altitude(&self, latitude: f64, longitude: f64) -> f64 {
        let (lat, dec) = (latitude.to_radians(), self.declination.to_radians());
        let hour_angle = (longitude - self.longitude).to_radians();
        lat.sin() * dec.sin() + lat.cos() * dec.cos() * hour_angle.cos()
    }

    /// Latitude of the terminator at a longitude: north of it is night
    /// when the sun is south of the equator, and the reverse.
    fn terminator_latitude(&self, longitude: f64) -> f64 {
        let hour_angle = (longitude - self.longitude).to_radians();
        // At the equinox the terminator runs pole to pole; keep the
        // tangent off zero so it does so without dividing by zero.
        let mut tan_dec = self.declination.to_radians().tan();
        if tan_dec.abs() < 1e-6 {
            tan_dec = 1e-6_f64.copysign(tan_dec);
        }
        (-hour_angle.cos() / tan_dec).atan().to_degrees()
    }
}

fn wrap_longitude(degrees: f64) -> f64 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

/// The centre of a Maidenhead locator as (latitude, longitude). Takes
/// 2, 4, 6 or 8 characters; anything else is `None`.
pub(super) fn grid_center(grid: &str) -> Option<(f64, f64)> {
    let g: Vec<char> = grid.trim().chars().map(|c| c.to_ascii_uppercase()).collect();
    if !matches!(g.len(), 2 | 4 | 6 | 8) {
        return None;
    }
    let letter = |c: char, count: u8| {
        let i = (c as u32).checked_sub('A' as u32)?;
        (c.is_ascii_uppercase() && i < count as u32).then_some(i as f64)
    };
    let digit = |c: char| c.to_digit(10).map(f64::from);

    let mut lon = -180.0 + letter(g[0], 18)? * 20.0;
    let mut lat = -90.0 + letter(g[1], 18)? * 10.0;
    // The size of the square located so far.
    let (mut lon_size, mut lat_size) = (20.0, 10.0);
    if g.len() >= 4 {
        (lon_size, lat_size) = (2.0, 1.0);
        lon += digit(g[2])? * lon_size;
        lat += digit(g[3])? * lat_size;
    }
    if g.len() >= 6 {
        (lon_size, lat_size) = (lon_size / 24.0, lat_size / 24.0);
        lon += letter(g[4], 24)? * lon_size;
        lat += letter(g[5], 24)? * lat_size;
    }
    if g.len() == 8 {
        (lon_size, lat_size) = (lon_size / 10.0, lat_size / 10.0);
        lon += digit(g[6])? * lon_size;
        lat += digit(g[7])? * lat_size;
    }
    Some((lat + lat_size / 2.0, lon + lon_size / 2.0))
}

/// Where the station is: its stored coordinates if it has them, else
/// the centre of its grid square.
pub(super) fn station_position(location: &StationLocation) -> Option<(f64, f64)> {
    match (location.latitude, location.longitude) {
        (Some(lat), Some(lon)) if lat.abs() <= 90.0 && lon.abs() <= 180.0 => Some((lat, lon)),
        _ => location.grid.as_deref().and_then(grid_center),
    }
}

/// The sun's altitude at which it is taken to rise and set: its upper
/// edge on the horizon, allowing for refraction.
const RISE_SET_ALTITUDE: f64 = -0.833;

/// The next sunrise and the next sunset at a place within 24 hours of
/// `from`, to the minute. Either is `None` when it doesn't happen (polar
/// day or night).
pub(super) fn next_rise_and_set(
    latitude: f64,
    longitude: f64,
    from: DateTime<Utc>,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let threshold = RISE_SET_ALTITUDE.to_radians().sin();
    let up = |t: DateTime<Utc>| Sun::at(t).sin_altitude(latitude, longitude) > threshold;
    let (mut rise, mut set) = (None, None);
    let mut was_up = up(from);
    for minute in 1..=24 * 60 {
        let t = from + Duration::minutes(minute);
        let is_up = up(t);
        if is_up && !was_up && rise.is_none() {
            rise = Some(t);
        }
        if !is_up && was_up && set.is_none() {
            set = Some(t);
        }
        if rise.is_some() && set.is_some() {
            break;
        }
        was_up = is_up;
    }
    (rise, set)
}

/// The line under the map: the grid in use and the next sunrise and
/// sunset there.
pub(super) fn caption(location: Option<&StationLocation>, now: DateTime<Utc>) -> String {
    let Some(location) = location else {
        return "No station location is active, so your position isn't marked.".into();
    };
    let Some((lat, lon)) = station_position(location) else {
        return format!(
            "Give the station location \"{}\" a grid square to mark your position.",
            location.name
        );
    };
    let place = location.grid.clone().unwrap_or_else(|| format!("{lat:.1}, {lon:.1}"));
    let time = |t: Option<DateTime<Utc>>| {
        t.map_or_else(|| "none today".to_string(), |t| t.format("%H:%MZ").to_string())
    };
    let (rise, set) = next_rise_and_set(lat, lon, now);
    format!("{place}  ·  sunrise {}  ·  sunset {}", time(rise), time(set))
}

/// The stretches of a meridian, as (south, north) latitudes, where the
/// sun is below `altitude` degrees. `hour_angle` is the meridian's
/// longitude east of the subsolar point, in degrees.
fn dark_spans(sun: &Sun, hour_angle: f64, altitude: f64) -> Vec<(f64, f64)> {
    const STEP: f64 = 1.0;
    let threshold = altitude.to_radians().sin();
    let dec = sun.declination.to_radians();
    let cos_h = hour_angle.to_radians().cos();
    // How far below the threshold the sun is at a latitude: positive
    // means dark.
    let depth = |lat: f64| {
        let lat = lat.to_radians();
        threshold - (lat.sin() * dec.sin() + lat.cos() * dec.cos() * cos_h)
    };

    let mut spans = Vec::new();
    let mut start = None;
    let (mut lat, mut d) = (-90.0, depth(-90.0));
    if d > 0.0 {
        start = Some(-90.0);
    }
    while lat < 90.0 {
        let next = (lat + STEP).min(90.0);
        let next_d = depth(next);
        if (d > 0.0) != (next_d > 0.0) {
            // The edge falls between the two samples.
            let edge = lat + (next - lat) * d / (d - next_d);
            match start.take() {
                Some(from) => spans.push((from, edge)),
                None => start = Some(edge),
            }
        }
        (lat, d) = (next, next_d);
    }
    if let Some(from) = start {
        spans.push((from, 90.0));
    }
    spans
}

/// The land outlines: one ring of (longitude, latitude) per polygon.
fn land() -> &'static [Vec<(f32, f32)>] {
    static LAND: OnceLock<Vec<Vec<(f32, f32)>>> = OnceLock::new();
    LAND.get_or_init(|| parse_land(include_str!("../../assets/land-110m.txt")))
}

fn parse_land(text: &str) -> Vec<Vec<(f32, f32)>> {
    text.lines()
        .map(|line| {
            line.split_whitespace()
                .filter_map(|pair| {
                    let (lon, lat) = pair.split_once(',')?;
                    Some((lon.parse().ok()?, lat.parse().ok()?))
                })
                .collect::<Vec<_>>()
        })
        .filter(|ring| ring.len() >= 3)
        .collect()
}

/// The largest 2:1 rectangle that fits in `size`, centred, on whole
/// pixels so that neighbouring strips of shading meet without seams.
fn map_rect(size: Size) -> Rectangle {
    let width = size.width.min(size.height * 2.0).floor().max(2.0);
    let height = (width / 2.0).floor();
    Rectangle {
        x: ((size.width - width) / 2.0).floor().max(0.0),
        y: ((size.height - height) / 2.0).floor().max(0.0),
        width,
        height,
    }
}

/// How dark each band of twilight makes the map. The bands stack: day,
/// then civil, nautical and astronomical twilight, then full night.
const SHADE_STEPS: [(f64, f32); 4] = [(0.0, 0.16), (-6.0, 0.12), (-12.0, 0.10), (-18.0, 0.10)];

/// The map itself, drawn into `cache` until that is cleared.
pub(super) struct Map<'a> {
    pub cache: &'a Cache,
    pub now: DateTime<Utc>,
    pub station: Option<(f64, f64)>,
}

impl<Message> canvas::Program<Message> for Map<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            let map = map_rect(frame.size());
            let x_of = |lon: f64| map.x + ((lon + 180.0) / 360.0) as f32 * map.width;
            let y_of = |lat: f64| map.y + ((90.0 - lat) / 180.0) as f32 * map.height;

            let hues = theme::hues(theme);
            let palette = theme.palette();
            let sea = mix(hues.pane, palette.primary, 0.10);
            let ground = mix(hues.pane, palette.text, 0.28);
            let coast = mix(hues.pane, palette.text, 0.50);

            frame.fill_rectangle(map.position(), map.size(), sea);

            for ring in land() {
                let outline = Path::new(|path| {
                    for (i, &(lon, lat)) in ring.iter().enumerate() {
                        let point = Point::new(x_of(lon as f64), y_of(lat as f64));
                        if i == 0 { path.move_to(point) } else { path.line_to(point) }
                    }
                    path.close();
                });
                frame.fill(&outline, ground);
                frame.stroke(&outline, Stroke::default().with_color(coast).with_width(1.0));
            }

            // Night, one pixel column at a time.
            let sun = Sun::at(self.now);
            let columns = map.width as usize;
            for (altitude, alpha) in SHADE_STEPS {
                let shade = Color { a: alpha, ..Color::BLACK };
                for column in 0..columns {
                    let lon = (column as f64 + 0.5) / columns as f64 * 360.0 - 180.0;
                    for (south, north) in dark_spans(&sun, lon - sun.longitude, altitude) {
                        let top = y_of(north);
                        frame.fill_rectangle(
                            Point::new(map.x + column as f32, top),
                            Size::new(1.0, y_of(south) - top),
                            shade,
                        );
                    }
                }
            }

            // The greyline.
            let terminator = Path::new(|path| {
                for column in 0..=columns {
                    let lon = column as f64 / columns as f64 * 360.0 - 180.0;
                    let point = Point::new(x_of(lon), y_of(sun.terminator_latitude(lon)));
                    if column == 0 { path.move_to(point) } else { path.line_to(point) }
                }
            });
            frame.stroke(&terminator, Stroke::default().with_color(hues.warn).with_width(1.5));

            // The station.
            if let Some((lat, lon)) = self.station {
                let here = Point::new(x_of(lon), y_of(lat));
                // Ringed in the theme's own text colour, so that a
                // night-vision theme stays free of white.
                frame.fill(&Path::circle(here, 5.5), hues.pane);
                frame.fill(&Path::circle(here, 4.5), palette.text);
                frame.fill(&Path::circle(here, 3.0), hues.bad);
            }

            let frame_line = Path::rectangle(map.position(), map.size());
            frame.stroke(&frame_line, Stroke::default().with_color(coast).with_width(1.0));
        });
        vec![geometry]
    }
}

/// `a` moved `amount` of the way to `b`.
fn mix(a: Color, b: Color, amount: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * amount,
        a.g + (b.g - a.g) * amount,
        a.b + (b.b - a.b) * amount,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().to_utc()
    }

    fn near(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn the_sun_is_where_the_almanac_says() {
        // June solstice 2026 (08:25 UTC): as far north as it gets.
        let solstice = Sun::at(utc("2026-06-21T08:25:00Z"));
        assert!(near(solstice.declination, 23.44, 0.02), "{solstice:?}");

        // March equinox 2026 (14:46 UTC): on the equator.
        let equinox = Sun::at(utc("2026-03-20T14:46:00Z"));
        assert!(near(equinox.declination, 0.0, 0.02), "{equinox:?}");

        // At 12:00 UTC the sun is over Greenwich, less the equation of
        // time: +14.2 min in mid February, -16.4 min in early November,
        // at a quarter of a degree per minute.
        let february = Sun::at(utc("2026-02-11T12:00:00Z"));
        assert!(near(february.longitude, 14.2 / 4.0, 0.1), "{february:?}");
        let november = Sun::at(utc("2026-11-03T12:00:00Z"));
        assert!(near(november.longitude, -16.4 / 4.0, 0.1), "{november:?}");

        // Six hours later it has moved a quarter of the way round, west.
        let later = Sun::at(utc("2026-11-03T18:00:00Z"));
        assert!(near(later.longitude, november.longitude - 90.0, 0.1), "{later:?}");
    }

    #[test]
    fn grid_squares_give_their_centres() {
        let (lat, lon) = grid_center("FN31pr").unwrap();
        assert!(near(lat, 41.729, 0.001) && near(lon, -72.708, 0.001), "{lat} {lon}");
        assert_eq!(grid_center("fn31"), Some((41.5, -73.0)));
        assert_eq!(grid_center("JJ"), Some((5.0, 10.0)));
        assert_eq!(grid_center(" IO91wm "), grid_center("IO91WM"));
        let (lat, lon) = grid_center("IO91wm53").unwrap();
        assert!(near(lat, 51.5146, 0.001) && near(lon, -0.1208, 0.001), "{lat} {lon}");

        for bad in ["", "F", "FN3", "SN31", "FN31zz", "31FN", "FN31pr1", "FNé1"] {
            assert_eq!(grid_center(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn sunrise_and_sunset_match_published_times() {
        // Newington, Connecticut (FN31pr) on 2026-11-23: sunrise 06:50
        // and sunset 16:24 Eastern, which is 11:50 and 21:24 UTC.
        let (lat, lon) = grid_center("FN31pr").unwrap();
        let (rise, set) = next_rise_and_set(lat, lon, utc("2026-11-23T05:00:00Z"));
        let minutes = |t: DateTime<Utc>, want: &str| (t - utc(want)).num_minutes().abs();
        assert!(minutes(rise.unwrap(), "2026-11-23T11:50:00Z") <= 3, "{rise:?}");
        assert!(minutes(set.unwrap(), "2026-11-23T21:24:00Z") <= 3, "{set:?}");

        // Asked in the afternoon, the sunset is today's and the sunrise
        // tomorrow's.
        let (rise, set) = next_rise_and_set(lat, lon, utc("2026-11-23T18:00:00Z"));
        assert!(set.unwrap() < rise.unwrap());

        // Midsummer at 80° north: the sun neither rises nor sets.
        assert_eq!(next_rise_and_set(80.0, 0.0, utc("2026-06-21T00:00:00Z")), (None, None));
    }

    #[test]
    fn the_dark_part_of_a_meridian_follows_the_season() {
        // Northern winter, sun over 20° south.
        let sun = Sun { declination: -20.0, longitude: 0.0 };

        // Under the sun: dark only beyond 70° north (polar night).
        let noon = dark_spans(&sun, 0.0, 0.0);
        assert_eq!(noon.len(), 1);
        assert!(near(noon[0].0, 70.0, 0.05) && noon[0].1 == 90.0, "{noon:?}");

        // Opposite the sun: dark except beyond 70° south (midnight sun).
        let midnight = dark_spans(&sun, 180.0, 0.0);
        assert_eq!(midnight.len(), 1);
        assert!(near(midnight[0].0, -70.0, 0.05) && midnight[0].1 == 90.0, "{midnight:?}");

        // The edge of each is where the closed-form terminator puts it.
        assert!(near(sun.terminator_latitude(0.0), 70.0, 1e-6));
        assert!(near(sun.terminator_latitude(180.0), -70.0, 1e-6));

        // Astronomical night reaches less far than plain night.
        let deep = dark_spans(&sun, 180.0, -18.0);
        assert!(near(deep[0].0, -52.0, 0.05), "{deep:?}");

        // Six hours round, at the equinox, the terminator is the
        // meridian itself and nothing is clearly either.
        let equinox = Sun { declination: 0.0, longitude: 0.0 };
        assert_eq!(dark_spans(&equinox, 0.0, 0.0), []);
        assert_eq!(dark_spans(&equinox, 180.0, 0.0), [(-90.0, 90.0)]);
    }

    #[test]
    fn the_land_outlines_load_and_stay_on_the_map() {
        let rings = land();
        assert!(rings.len() > 100, "{} rings", rings.len());
        assert!(rings.iter().map(Vec::len).sum::<usize>() > 4000);
        for &(lon, lat) in rings.iter().flatten() {
            assert!((-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat));
        }
    }

    #[test]
    fn the_map_keeps_its_shape_in_any_panel() {
        let wide = map_rect(Size::new(900.0, 200.0));
        assert_eq!((wide.width, wide.height, wide.x, wide.y), (400.0, 200.0, 250.0, 0.0));
        let tall = map_rect(Size::new(301.0, 500.0));
        assert_eq!((tall.width, tall.height, tall.x, tall.y), (301.0, 150.0, 0.0, 175.0));
    }
}
