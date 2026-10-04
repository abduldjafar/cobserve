//! Prayer times, from where the sun stands — at a place, on a day — by the criteria Indonesia's
//! Ministry of Religious Affairs (Kemenag) publishes its schedules with. Nothing is asked of a
//! service: the sun is computed here.
//!
//! - **Subuh** when the sun is 20° below the horizon, **Isya** when it is 18° below.
//! - **Terbit** and **Maghrib** when its upper edge meets the horizon, refraction and the dip
//!   of the horizon included: 1° below.
//! - **Ashar** when a thing's shadow is its noon shadow plus its own length (Shafi'i).
//! - **Dzuhur** once the whole disc has crossed the meridian, a minute after its centre.
//! - Then the *ihtiyat*, the margin Kemenag adds so that one schedule holds for a whole town:
//!   two minutes on, rounded up to the minute — Terbit two minutes off, rounded down.
//!
//! The sun's position is the US Naval Observatory's approximation, good to about a minute of
//! arc for this century, as praytimes.org computes it. The tests hold it to Kemenag's own tables.

use chrono::{Datelike, NaiveDate, Weekday};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Prayer {
    Subuh,
    /// Sunrise: not a prayer, the end of Subuh's time.
    Terbit,
    Dzuhur,
    Ashar,
    Maghrib,
    Isya,
}

impl Prayer {
    pub const ALL: [Prayer; 6] = [Prayer::Subuh, Prayer::Terbit, Prayer::Dzuhur, Prayer::Ashar, Prayer::Maghrib, Prayer::Isya];

    /// Its name on that day: on a Friday the noon prayer is Jumat.
    pub fn name(self, friday: bool) -> &'static str {
        match self {
            Prayer::Subuh => "Subuh",
            Prayer::Terbit => "Terbit",
            Prayer::Dzuhur if friday => "Jumat",
            Prayer::Dzuhur => "Dzuhur",
            Prayer::Ashar => "Ashar",
            Prayer::Maghrib => "Maghrib",
            Prayer::Isya => "Isya",
        }
    }

    /// Whether it is a prayer, and so worth a reminder; sunrise is not.
    pub fn is_prayer(self) -> bool {
        self != Prayer::Terbit
    }
}

/// Where the times are for.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// How the place was chosen, for the screen to say: `PRAYER_CITY`, or the time zone's city.
    pub from: PlaceFrom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceFrom {
    /// Set in the configuration, by name or by coordinates.
    Chosen,
    /// The city of the machine's time zone: right to the minute only near that city.
    TimeZone,
}

/// A day's times at a place, as Unix seconds; `None` where the sun never gets there that day
/// (Isya in a northern summer).
#[derive(Debug, Clone, PartialEq)]
pub struct Day {
    pub date: NaiveDate,
    pub times: [Option<i64>; 6],
}

impl Day {
    pub fn at(&self, prayer: Prayer) -> Option<i64> {
        self.times[prayer as usize]
    }

    pub fn is_friday(&self) -> bool {
        self.date.weekday() == Weekday::Fri
    }
}

/// The schedule around now: yesterday, today and tomorrow at the place, today being the place's
/// own date (by its longitude, whatever zone the clock is shown in).
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub place: Place,
    pub days: [Day; 3],
}

/// A prayer's time, and on which day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    pub prayer: Prayer,
    pub at: i64,
    pub friday: bool,
}

impl Moment {
    pub fn name(&self) -> &'static str {
        self.prayer.name(self.friday)
    }
}

impl Schedule {
    pub fn around(place: &Place, now: i64) -> Schedule {
        let today = local_date(place, now);
        let day = |offset: i64| compute(today + chrono::Duration::days(offset), place);
        Schedule { place: place.clone(), days: [day(-1), day(0), day(1)] }
    }

    /// Whether it still describes `now` at `place` — the place's date has not moved on.
    pub fn holds(&self, place: &Place, now: i64) -> bool {
        self.place == *place && self.days[1].date == local_date(place, now)
    }

    pub fn today(&self) -> &Day {
        &self.days[1]
    }

    fn moments(&self) -> impl Iterator<Item = Moment> + '_ {
        self.days.iter().flat_map(|day| {
            Prayer::ALL.into_iter().filter_map(move |prayer| day.at(prayer).map(|at| Moment { prayer, at, friday: day.is_friday() }))
        })
    }

    /// The next prayer after `now` — sunrise is not one.
    pub fn next_prayer(&self, now: i64) -> Option<Moment> {
        self.moments().filter(|m| m.at > now && m.prayer.is_prayer()).min_by_key(|m| m.at)
    }

    /// The last prayer at or before `now`.
    pub fn last_prayer(&self, now: i64) -> Option<Moment> {
        self.moments().filter(|m| m.at <= now && m.prayer.is_prayer()).max_by_key(|m| m.at)
    }
}

/// What a prayer close by asks of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alert {
    /// It is less than the lead away: `left` seconds.
    Soon { moment: Moment, left: i64 },
    /// Its time came `since` seconds ago.
    Now { moment: Moment, since: i64 },
}

impl Alert {
    pub fn moment(&self) -> Moment {
        match *self {
            Alert::Soon { moment, .. } | Alert::Now { moment, .. } => moment,
        }
    }
}

/// The alert at `now`: a prayer due within `lead` seconds, or one whose time came less than
/// `after` seconds ago.
pub fn alert(schedule: &Schedule, now: i64, lead: i64, after: i64) -> Option<Alert> {
    if let Some(next) = schedule.next_prayer(now)
        && next.at - now <= lead
    {
        return Some(Alert::Soon { moment: next, left: next.at - now });
    }
    let last = schedule.last_prayer(now)?;
    (now - last.at < after).then_some(Alert::Now { moment: last, since: now - last.at })
}

/// The prayer times as the app keeps them: where, the schedule around now, and what was said
/// about which prayer.
#[derive(Debug, Clone, Default)]
pub struct Prayers {
    /// `None`: no place known, or `PRAYER=off`.
    pub place: Option<Place>,
    pub schedule: Option<Schedule>,
    /// How far ahead the reminder comes, in seconds; 0 for none.
    pub lead: i64,
    /// The prayer (by its time) whose reminder was waved away.
    pub dismissed: Option<i64>,
    /// What was already said beyond the screen: a prayer's time, and whether it was its coming
    /// (`false`) or its time (`true`).
    said: Vec<(i64, bool)>,
}

/// How long a prayer's time is shown after it came.
pub const NOW_FOR_S: i64 = 300;

impl Prayers {
    pub fn new(place: Option<Place>, remind_minutes: u32) -> Self {
        Prayers { place, lead: i64::from(remind_minutes) * 60, ..Self::default() }
    }

    /// Somewhere else from now on — the zone changed, or the place was chosen.
    pub fn move_to(&mut self, place: Option<Place>) {
        if self.place != place {
            self.place = place;
            self.schedule = None;
        }
    }

    /// The schedule brought up to `now`, and the alert that has just begun, once: the reminder of
    /// a prayer, or its time.
    pub fn tick(&mut self, now: i64) -> Option<Alert> {
        let place = self.place.as_ref()?;
        if !self.schedule.as_ref().is_some_and(|s| s.holds(place, now)) {
            self.schedule = Some(Schedule::around(place, now));
        }
        let alert = self.alert(now)?;
        let key = (alert.moment().at, matches!(alert, Alert::Now { .. }));
        if self.said.contains(&key) {
            return None;
        }
        // A reminder found late — the monitor started with two minutes to go — still counts;
        // one found after its prayer's time does not repeat it.
        self.said.retain(|(at, _)| now - at < 86_400);
        self.said.push(key);
        Some(alert)
    }

    /// What the screen shows now: the reminder or the time of a prayer, unless waved away.
    pub fn alert(&self, now: i64) -> Option<Alert> {
        let schedule = self.schedule.as_ref()?;
        let alert = if self.lead > 0 {
            alert(schedule, now, self.lead, NOW_FOR_S)?
        } else {
            alert(schedule, now, 0, NOW_FOR_S).filter(|a| matches!(a, Alert::Now { .. }))?
        };
        (self.dismissed != Some(alert.moment().at)).then_some(alert)
    }

    /// The alert on screen, waved away: neither it nor that prayer's time comes back.
    pub fn dismiss(&mut self, now: i64) {
        if let Some(alert) = self.alert(now) {
            self.dismissed = Some(alert.moment().at);
        }
    }
}

/// The place's own date at `now`: by its mean solar time, a degree of longitude four minutes.
fn local_date(place: &Place, now: i64) -> NaiveDate {
    let solar = now + (place.lon * 240.0).round() as i64;
    let days = solar.div_euclid(86_400);
    NaiveDate::from_num_days_from_ce_opt((days + 719_163) as i32).unwrap_or_default()
}

/// Kemenag's angles: the sun this far below the horizon.
const SUBUH_ANGLE: f64 = 20.0;
const ISYA_ANGLE: f64 = 18.0;
/// Its upper edge on the horizon: semi-diameter, refraction and the horizon's dip.
const HORIZON_ANGLE: f64 = 1.0;
const IHTIYAT_S: f64 = 120.0;
/// The time the sun's disc takes to cross the meridian after its centre has: half a degree at
/// a quarter of a degree a minute.
const DISC_S: f64 = 60.0;

/// One day's times at a place.
pub fn compute(date: NaiveDate, place: &Place) -> Day {
    let (lat, lon) = (place.lat, place.lon);
    let jd = julian(date) - lon / (15.0 * 24.0);
    // Hours of the place's mean solar day, refined once from a first guess: the sun is where it
    // is at that hour, not at midnight.
    let mut hours = [5.0, 6.0, 12.0, 13.0, 18.0, 18.0];
    for _ in 0..2 {
        let at = |i: usize| jd + hours[i] / 24.0;
        hours = [
            angle_time(SUBUH_ANGLE, at(0), lat, true),
            angle_time(HORIZON_ANGLE, at(1), lat, true),
            Some(midday(at(2))),
            asr_time(1.0, at(3), lat),
            angle_time(HORIZON_ANGLE, at(4), lat, false),
            angle_time(ISYA_ANGLE, at(5), lat, false),
        ]
        .map(|h| h.unwrap_or(f64::NAN));
    }
    let midnight = date.and_hms_opt(0, 0, 0).map(|t| t.and_utc().timestamp()).unwrap_or_default();
    // Hours of solar time at the place → seconds of UTC on its date.
    let instant = |h: f64| (h.is_finite()).then(|| midnight as f64 + (h - lon / 15.0) * 3600.0);
    let up = |s: f64| ((s / 60.0).ceil() * 60.0) as i64;
    let down = |s: f64| ((s / 60.0).floor() * 60.0) as i64;
    let after = |h: f64, extra: f64| instant(h).map(|s| up(s + extra - 1e-6));
    Day {
        date,
        times: [
            after(hours[0], IHTIYAT_S),
            instant(hours[1]).map(|s| down(s - IHTIYAT_S + 1e-6)),
            after(hours[2], DISC_S + IHTIYAT_S),
            after(hours[3], IHTIYAT_S),
            after(hours[4], IHTIYAT_S),
            after(hours[5], IHTIYAT_S),
        ],
    }
}

fn julian(date: NaiveDate) -> f64 {
    let (mut y, mut m) = (date.year() as f64, date.month() as f64);
    let d = date.day() as f64;
    if m <= 2.0 {
        y -= 1.0;
        m += 12.0;
    }
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + d + b - 1524.5
}

/// The sun's declination and the equation of time (hours) at a Julian date.
fn sun(jd: f64) -> (f64, f64) {
    let d = jd - 2_451_545.0;
    let g = fix(357.529 + 0.985_600_28 * d, 360.0);
    let q = fix(280.459 + 0.985_647_36 * d, 360.0);
    let l = fix(q + 1.915 * sin(g) + 0.020 * sin(2.0 * g), 360.0);
    let e = 23.439 - 0.000_000_36 * d;
    let ra = fix(atan2(cos(e) * sin(l), cos(l)) / 15.0, 24.0);
    (asin(sin(e) * sin(l)), q / 15.0 - ra)
}

/// When the sun crosses the meridian, in hours of mean solar time.
fn midday(jd: f64) -> f64 {
    fix(12.0 - sun(jd).1, 24.0)
}

/// When the sun is `angle` below the horizon (negative: above), before noon or after it.
fn angle_time(angle: f64, jd: f64, lat: f64, before_noon: bool) -> Option<f64> {
    let decl = sun(jd).0;
    let cos_h = (-sin(angle) - sin(decl) * sin(lat)) / (cos(decl) * cos(lat));
    if !(-1.0..=1.0).contains(&cos_h) {
        return None;
    }
    let t = acos(cos_h) / 15.0;
    let noon = midday(jd);
    Some(if before_noon { noon - t } else { noon + t })
}

/// When a shadow is its noon length plus `factor` times the thing casting it.
fn asr_time(factor: f64, jd: f64, lat: f64) -> Option<f64> {
    let decl = sun(jd).0;
    let altitude = (1.0 / (factor + tan((lat - decl).abs()))).atan().to_degrees();
    angle_time(-altitude, jd, lat, false)
}

fn fix(a: f64, b: f64) -> f64 {
    a.rem_euclid(b)
}
fn sin(d: f64) -> f64 {
    d.to_radians().sin()
}
fn cos(d: f64) -> f64 {
    d.to_radians().cos()
}
fn tan(d: f64) -> f64 {
    d.to_radians().tan()
}
fn asin(x: f64) -> f64 {
    x.asin().to_degrees()
}
fn acos(x: f64) -> f64 {
    x.acos().to_degrees()
}
fn atan2(y: f64, x: f64) -> f64 {
    y.atan2(x).to_degrees()
}

/// Cities a place can be named by, with the coordinates Kemenag's schedules are close to.
pub const CITIES: &[(&str, f64, f64)] = &[
    ("Jakarta", -6.2088, 106.8456),
    ("Bogor", -6.5950, 106.8166),
    ("Depok", -6.4025, 106.7942),
    ("Tangerang", -6.1781, 106.6300),
    ("Bekasi", -6.2383, 106.9756),
    ("Serang", -6.1200, 106.1503),
    ("Bandung", -6.9175, 107.6191),
    ("Cirebon", -6.7320, 108.5523),
    ("Tasikmalaya", -7.3274, 108.2207),
    ("Semarang", -6.9667, 110.4167),
    ("Solo", -7.5755, 110.8243),
    ("Yogyakarta", -7.7956, 110.3695),
    ("Surabaya", -7.2575, 112.7521),
    ("Malang", -7.9666, 112.6326),
    ("Denpasar", -8.6705, 115.2126),
    ("Mataram", -8.5833, 116.1167),
    ("Kupang", -10.1772, 123.6070),
    ("Banda Aceh", 5.5483, 95.3238),
    ("Medan", 3.5952, 98.6722),
    ("Padang", -0.9471, 100.4172),
    ("Pekanbaru", 0.5071, 101.4478),
    ("Batam", 1.1301, 104.0529),
    ("Jambi", -1.6101, 103.6131),
    ("Palembang", -2.9761, 104.7754),
    ("Bengkulu", -3.7928, 102.2608),
    ("Bandar Lampung", -5.3971, 105.2668),
    ("Pontianak", -0.0263, 109.3425),
    ("Banjarmasin", -3.3186, 114.5944),
    ("Palangka Raya", -2.2136, 113.9108),
    ("Balikpapan", -1.2379, 116.8529),
    ("Samarinda", -0.5022, 117.1536),
    ("Makassar", -5.1477, 119.4327),
    ("Kendari", -3.9985, 122.5129),
    ("Palu", -0.8917, 119.8707),
    ("Gorontalo", 0.5435, 123.0568),
    ("Manado", 1.4748, 124.8421),
    ("Ternate", 0.7893, 127.3772),
    ("Ambon", -3.6954, 128.1814),
    ("Sorong", -0.8762, 131.2558),
    ("Jayapura", -2.5337, 140.7181),
    ("Kuala Lumpur", 3.1390, 101.6869),
    ("Singapore", 1.3521, 103.8198),
    ("Bandar Seri Begawan", 4.9031, 114.9398),
    ("Mecca", 21.4225, 39.8262),
    ("Medina", 24.4672, 39.6111),
];

/// A city by name, any case: `bandung`, `Kota Bandung`.
pub fn city(name: &str) -> Option<Place> {
    let wanted = name.trim().to_lowercase();
    let wanted = wanted.strip_prefix("kota ").unwrap_or(&wanted);
    CITIES.iter().find(|(city, ..)| city.to_lowercase() == wanted).map(|&(name, lat, lon)| Place {
        name: name.to_string(),
        lat,
        lon,
        from: PlaceFrom::Chosen,
    })
}

/// `-6.91,107.61`: coordinates, latitude first.
pub fn coordinates(text: &str) -> Option<(f64, f64)> {
    let (lat, lon) = text.split_once(',')?;
    let (lat, lon) = (lat.trim().parse::<f64>().ok()?, lon.trim().parse::<f64>().ok()?);
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

/// The city a time zone is named for, where the system's list of zones (`zone1970.tab`) or the
/// list above has it: `Asia/Makassar` → Makassar.
pub fn of_time_zone(zone: &str, zone_tab: Option<&str>) -> Option<Place> {
    let city_name = zone.rsplit('/').next()?.replace('_', " ");
    if let Some(mut place) = city(&city_name) {
        place.from = PlaceFrom::TimeZone;
        return Some(place);
    }
    let line = zone_tab?.lines().find(|line| !line.starts_with('#') && line.split('\t').nth(2) == Some(zone))?;
    let (lat, lon) = iso6709(line.split('\t').nth(1)?)?;
    Some(Place { name: city_name, lat, lon, from: PlaceFrom::TimeZone })
}

/// `-0610+10648` or `+404251-0740023` → degrees.
fn iso6709(text: &str) -> Option<(f64, f64)> {
    let split = text[1..].find(['+', '-'])? + 1;
    let part = |s: &str, degrees: usize| -> Option<f64> {
        let sign = if s.starts_with('-') { -1.0 } else { 1.0 };
        let digits = &s[1..];
        let d: f64 = digits.get(..degrees)?.parse().ok()?;
        let m: f64 = digits.get(degrees..degrees + 2)?.parse().ok()?;
        let sec: f64 = digits.get(degrees + 2..degrees + 4).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        Some(sign * (d + m / 60.0 + sec / 3600.0))
    };
    Some((part(&text[..split], 2)?, part(&text[split..], 3)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};

    fn place(name: &str) -> Place {
        city(name).unwrap()
    }

    /// `HH:MM` in a zone `hours` east of UTC.
    fn hm(at: i64, hours: i32) -> String {
        FixedOffset::east_opt(hours * 3600).unwrap().timestamp_opt(at, 0).unwrap().format("%H:%M").to_string()
    }

    fn minutes(text: &str) -> i64 {
        let (h, m) = text.split_once(':').unwrap();
        h.parse::<i64>().unwrap() * 60 + m.parse::<i64>().unwrap()
    }

    #[test]
    fn jakarta_matches_kemenag_s_october_table() {
        // Kemenag's schedule for Kota Jakarta, 1–11 October 2026: Subuh, Dzuhur, Ashar,
        // Maghrib, Isya.
        let table = [
            ["04:23", "11:46", "14:51", "17:50", "18:59"],
            ["04:22", "11:46", "14:50", "17:50", "18:59"],
            ["04:22", "11:45", "14:49", "17:50", "18:59"],
            ["04:21", "11:45", "14:48", "17:50", "18:59"],
            ["04:21", "11:45", "14:47", "17:50", "18:58"],
            ["04:20", "11:44", "14:46", "17:50", "18:58"],
            ["04:20", "11:44", "14:46", "17:49", "18:58"],
            ["04:19", "11:44", "14:45", "17:49", "18:58"],
            ["04:19", "11:44", "14:44", "17:49", "18:58"],
            ["04:18", "11:43", "14:44", "17:49", "18:58"],
            ["04:18", "11:43", "14:45", "17:49", "18:58"],
        ];
        let jakarta = place("Jakarta");
        let mut exact = 0;
        for (i, row) in table.iter().enumerate() {
            let day = compute(NaiveDate::from_ymd_opt(2026, 10, 1 + i as u32).unwrap(), &jakarta);
            for (prayer, want) in [Prayer::Subuh, Prayer::Dzuhur, Prayer::Ashar, Prayer::Maghrib, Prayer::Isya].into_iter().zip(row) {
                let got = hm(day.at(prayer).unwrap(), 7);
                let off = (minutes(&got) - minutes(want)).abs();
                assert!(off <= 1, "{prayer:?} on October {}: {got}, Kemenag says {want}", i + 1);
                exact += usize::from(off == 0);
            }
        }
        assert!(exact >= 50, "to the minute in {exact} of 55");
    }

    #[test]
    fn medan_matches_kemenag_in_february() {
        let day = compute(NaiveDate::from_ymd_opt(2026, 2, 20).unwrap(), &place("Medan"));
        let got: Vec<String> = [Prayer::Subuh, Prayer::Ashar, Prayer::Maghrib, Prayer::Isya].map(|p| hm(day.at(p).unwrap(), 7)).into();
        assert_eq!(got, ["05:23", "16:00", "18:43", "19:52"]);
        let dzuhur = minutes(&hm(day.at(Prayer::Dzuhur).unwrap(), 7));
        assert!((dzuhur - minutes("12:42")).abs() <= 1);
    }

    #[test]
    fn the_times_come_in_their_order_and_sunrise_before_noon() {
        let day = compute(NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(), &place("Makassar"));
        let times: Vec<i64> = day.times.iter().map(|t| t.unwrap()).collect();
        assert!(times.windows(2).all(|w| w[0] < w[1]), "{times:?}");
        assert!(hm(times[2], 8).as_str() < "12:30", "noon near noon in WITA");
    }

    #[test]
    fn where_the_sun_never_gets_that_low_there_is_no_isya() {
        let oslo = Place { name: "Oslo".into(), lat: 59.91, lon: 10.75, from: PlaceFrom::Chosen };
        let day = compute(NaiveDate::from_ymd_opt(2026, 6, 21).unwrap(), &oslo);
        assert_eq!(day.at(Prayer::Isya), None);
        assert!(day.at(Prayer::Maghrib).is_some() && day.at(Prayer::Dzuhur).is_some());
    }

    #[test]
    fn on_friday_noon_is_jumat() {
        let day = compute(NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(), &place("Jakarta"));
        assert!(day.is_friday());
        assert_eq!(Prayer::Dzuhur.name(day.is_friday()), "Jumat");
        assert_eq!(Prayer::Dzuhur.name(false), "Dzuhur");
    }

    #[test]
    fn after_isya_the_next_is_tomorrow_s_subuh_and_before_it_last_night_s_isya_was_the_last() {
        let jakarta = place("Jakarta");
        // 22:00 WIB on 4 October 2026.
        let night = chrono::Utc.with_ymd_and_hms(2026, 10, 4, 15, 0, 0).unwrap().timestamp();
        let schedule = Schedule::around(&jakarta, night);
        assert_eq!(schedule.today().date, NaiveDate::from_ymd_opt(2026, 10, 4).unwrap());
        let next = schedule.next_prayer(night).unwrap();
        assert_eq!((next.prayer, hm(next.at, 7)), (Prayer::Subuh, "04:21".to_string()));
        assert_eq!(schedule.last_prayer(night).unwrap().prayer, Prayer::Isya);
        // 02:00 WIB on the 5th: a new date, and Isya of the 4th is still the current period.
        let small_hours = night + 4 * 3600;
        let schedule = Schedule::around(&jakarta, small_hours);
        assert_eq!(schedule.today().date, NaiveDate::from_ymd_opt(2026, 10, 5).unwrap());
        assert_eq!(schedule.last_prayer(small_hours).unwrap().prayer, Prayer::Isya);
        assert_eq!(schedule.next_prayer(small_hours).unwrap().prayer, Prayer::Subuh);
        assert!(!Schedule::around(&jakarta, night).holds(&jakarta, small_hours), "a new day, a new schedule");
    }

    #[test]
    fn the_alert_comes_ten_minutes_before_and_stays_a_while_after() {
        let jakarta = place("Jakarta");
        let day = compute(NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(), &jakarta);
        let ashar = day.at(Prayer::Ashar).unwrap();
        let schedule = Schedule::around(&jakarta, ashar);
        assert_eq!(alert(&schedule, ashar - 11 * 60, 600, 300), None);
        let soon = alert(&schedule, ashar - 9 * 60, 600, 300).unwrap();
        assert!(matches!(soon, Alert::Soon { left: 540, .. }), "{soon:?}");
        assert_eq!(soon.moment().prayer, Prayer::Ashar);
        assert!(matches!(alert(&schedule, ashar + 60, 600, 300), Some(Alert::Now { since: 60, .. })));
        assert_eq!(alert(&schedule, ashar + 301, 600, 300), None);
        // Sunrise is no prayer: nothing before it.
        let terbit = day.at(Prayer::Terbit).unwrap();
        assert_eq!(alert(&schedule, terbit - 300, 600, 300), None);
    }

    #[test]
    fn the_reminder_is_said_once_its_time_once_and_a_wave_hides_both() {
        let jakarta = place("Jakarta");
        let day = compute(NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(), &jakarta);
        let ashar = day.at(Prayer::Ashar).unwrap();
        let mut prayers = Prayers::new(Some(jakarta.clone()), 10);
        assert_eq!(prayers.tick(ashar - 20 * 60), None, "nothing yet");
        assert!(matches!(prayers.tick(ashar - 10 * 60), Some(Alert::Soon { .. })), "ten minutes before");
        assert_eq!(prayers.tick(ashar - 9 * 60), None, "said once");
        assert!(prayers.alert(ashar - 9 * 60).is_some(), "and still on screen");
        assert!(matches!(prayers.tick(ashar), Some(Alert::Now { .. })), "its time");
        assert_eq!(prayers.tick(ashar + 30), None);
        prayers.dismiss(ashar + 30);
        assert_eq!(prayers.alert(ashar + 60), None, "waved away");

        // Waved away while it was coming: its time is not shown either, nor said.
        let maghrib = day.at(Prayer::Maghrib).unwrap();
        assert!(prayers.tick(maghrib - 5 * 60).is_some());
        prayers.dismiss(maghrib - 5 * 60);
        assert_eq!(prayers.alert(maghrib + 10), None);
        assert!(prayers.tick(maghrib + 10).is_none(), "a waved-away prayer is not said again");

        // No reminder asked for: only the time itself.
        let mut quiet = Prayers::new(Some(jakarta), 0);
        assert_eq!(quiet.tick(ashar - 5 * 60), None);
        assert!(matches!(quiet.tick(ashar + 1), Some(Alert::Now { .. })));
        // Nowhere: nothing.
        assert_eq!(Prayers::new(None, 10).tick(ashar), None);
    }

    #[test]
    fn places_by_name_coordinates_or_time_zone() {
        assert_eq!(city("kota bandung").unwrap().name, "Bandung");
        assert!(city("Atlantis").is_none());
        assert_eq!(coordinates(" -6.91, 107.61 "), Some((-6.91, 107.61)));
        assert_eq!(coordinates("-95,1"), None);
        let makassar = of_time_zone("Asia/Makassar", None).unwrap();
        assert_eq!((makassar.name.as_str(), makassar.from), ("Makassar", PlaceFrom::TimeZone));
        let tab = "# comment\nNO\t+5955+01045\tEurope/Oslo\nUS\t+404251-0740023\tAmerica/New_York\tEastern (most areas)\n";
        let oslo = of_time_zone("Europe/Oslo", Some(tab)).unwrap();
        assert!((oslo.lat - 59.9167).abs() < 0.001 && (oslo.lon - 10.75).abs() < 0.001, "{oslo:?}");
        let new_york = of_time_zone("America/New_York", Some(tab)).unwrap();
        assert_eq!(new_york.name, "New York");
        assert!((new_york.lat - 40.7142).abs() < 0.001 && (new_york.lon + 74.0064).abs() < 0.001, "{new_york:?}");
        assert!(of_time_zone("Etc/UTC", Some(tab)).is_none());
    }
}
