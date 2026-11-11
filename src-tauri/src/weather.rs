//! Current weather: location from the Windows location service, conditions from Open-Meteo.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::Devices::Geolocation::{GeolocationAccessStatus, Geolocator, PositionAccuracy};
use windows::Foundation::TimeSpan;

#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct Weather {
    pub temp_c: f32,
    /// WMO weather code from Open-Meteo.
    pub code: u8,
    pub is_day: bool,
    /// Simplified for the icon.
    pub kind: Kind,
}

#[derive(Serialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Clear,
    PartlyCloudy,
    Cloudy,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunder,
}

const LOCATION_TTL: Duration = Duration::from_secs(6 * 3600);
static LOCATION: Mutex<Option<(Instant, (f64, f64))>> = Mutex::new(None);

/// Blocking: location (cached for 6 h) + one Open-Meteo request.
/// Err with a short reason ("location off", "offline", ...) so the UI can hide the weather.
pub fn fetch() -> Result<Weather, String> {
    let (lat, lon) = location()?;
    // Only lat/lon rounded to ~1 km leave the machine.
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat:.2}&longitude={lon:.2}&current=temperature_2m,weather_code,is_day&timezone=auto"
    );
    let body = crate::http::get(&url, &[]).map_err(|e| format!("offline ({e})"))?;
    parse(&body)
}

fn location() -> Result<(f64, f64), String> {
    let mut cache = LOCATION.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, pos)) = *cache {
        if at.elapsed() < LOCATION_TTL {
            return Ok(pos);
        }
    }
    let access = Geolocator::RequestAccessAsync().and_then(|a| a.join()).map_err(|_| "location off")?;
    if access != GeolocationAccessStatus::Allowed {
        return Err("location off".into());
    }
    let pos = (|| {
        let geo = Geolocator::new()?;
        geo.SetDesiredAccuracy(PositionAccuracy::Default)?;
        let max_age = TimeSpan { Duration: 10 * 60 * 10_000_000 };
        let timeout = TimeSpan { Duration: 10 * 10_000_000 };
        let p = geo.GetGeopositionAsyncWithAgeAndTimeout(max_age, timeout)?.join()?.Coordinate()?.Point()?.Position()?;
        Ok::<_, windows::core::Error>((p.Latitude, p.Longitude))
    })()
    .map_err(|e| format!("no location ({})", e.message()))?;
    *cache = Some((Instant::now(), pos));
    Ok(pos)
}

#[derive(Deserialize)]
struct Response {
    current: Current,
}

#[derive(Deserialize)]
struct Current {
    temperature_2m: f32,
    weather_code: u8,
    is_day: u8,
}

fn parse(json: &str) -> Result<Weather, String> {
    let c = serde_json::from_str::<Response>(json).map_err(|_| "bad response")?.current;
    Ok(Weather { temp_c: c.temperature_2m, code: c.weather_code, is_day: c.is_day == 1, kind: kind(c.weather_code) })
}

/// WMO code to icon kind; unknown codes count as cloudy.
fn kind(code: u8) -> Kind {
    match code {
        0 => Kind::Clear,
        1 | 2 => Kind::PartlyCloudy,
        45 | 48 => Kind::Fog,
        51..=57 => Kind::Drizzle,
        61..=67 | 80..=82 => Kind::Rain,
        71..=77 | 85 | 86 => Kind::Snow,
        95..=99 => Kind::Thunder,
        _ => Kind::Cloudy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_codes() {
        let cases = [
            (0, Kind::Clear),
            (2, Kind::PartlyCloudy),
            (3, Kind::Cloudy),
            (48, Kind::Fog),
            (53, Kind::Drizzle),
            (63, Kind::Rain),
            (81, Kind::Rain),
            (75, Kind::Snow),
            (86, Kind::Snow),
            (96, Kind::Thunder),
        ];
        for (code, k) in cases {
            assert_eq!(kind(code), k, "code {code}");
        }
    }

    #[test]
    fn parses_response() {
        let json = r#"{"latitude":52.52,"longitude":13.41,"current_units":{"temperature_2m":"°C"},
            "current":{"time":"2026-11-11T16:45","interval":900,"temperature_2m":8.3,"weather_code":3,"is_day":0}}"#;
        assert_eq!(parse(json), Ok(Weather { temp_c: 8.3, code: 3, is_day: false, kind: Kind::Cloudy }));
        assert!(parse("{}").is_err());
    }

    #[test]
    #[ignore]
    fn weather_live() {
        println!("{:?}", fetch());
    }
}
