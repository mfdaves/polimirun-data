//! Polimirun results on top of the [`endu`] client: one row schema for both
//! races, with rankings. [`edition_results`] is the entry point;
//! [`assign_runner_ids`] and [`assign_families`] link rows across a download.

use endu::{Client, Edition, Entry, Error};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::Hash;

pub mod age_grade;
pub mod country;
pub mod sqlite;

/// Event group holding every Polimirun edition (`endu.net/events/polimirunspring`).
pub const POLIMIRUN_GROUP: u64 = 6212;

/// Runners with the same surname finishing this many seconds apart or less
/// count as a family (see [`assign_families`]).
pub const FAMILY_WINDOW: u32 = 10;

/// Words that belong to the surname that follows them: "DI PRESA", "DE LA CRUZ".
const SURNAME_PARTICLES: [&str; 20] = [
    "D", "DA", "DAL", "DALL", "DALLA", "DAS", "DE", "DEGLI", "DEL", "DELL", "DELLA", "DELLE", "DI", "DOS", "LA",
    "LI", "LO", "MC", "VAN", "VON",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RaceKind {
    Competitive,
    NonCompetitive,
}

/// One finisher, the same shape for both races.
///
/// Ranks are 1-based and `None` for runners without a valid time (gender ranks
/// also for runners of unknown gender):
/// - `rank`, `gender_rank`: inside the runner's race. The competitive race is
///   ordered by official time, matching the published results; the
///   non-competitive one, which has no published ranking, by chip time.
/// - `general_rank`, `general_gender_rank`: across both races, by chip time.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub edition_id: u64,
    pub year: u16,
    /// The same person across editions, see [`assign_runner_ids`].
    pub runner_id: u32,
    /// Runners who crossed the line together, see [`assign_families`].
    pub family_id: Option<u32>,
    pub race: RaceKind,
    pub bib: String,
    pub name: String,
    /// First word of the name, which endu lists surname first, with particles
    /// such as "DI" or "DE LA" kept. Empty for one-word names.
    pub surname: String,
    pub gender: String,
    pub year_of_birth: Option<u16>,
    /// Race year minus year of birth, so possibly one year too many. `None`
    /// outside 10 to 95, where the year of birth is wrong.
    pub age: Option<u16>,
    pub team: String,
    pub nationality: String,
    /// `nationality` as an IOC code, see [`country`].
    pub country: String,
    pub category: String,
    /// `H:MM:SS`, or the raw value when it is not a time (e.g. `DSQ`).
    pub official_time: Option<String>,
    pub real_time: Option<String>,
    /// Chip time in seconds (official time when there is no chip time).
    pub seconds: Option<u32>,
    /// Seconds between the gun and crossing the start line: official minus
    /// chip time. `None` in races that publish chip time as official time.
    pub start_delay: Option<u32>,
    /// `seconds` as a percentage of the 10 km standard for the runner's age
    /// and gender, see [`age_grade`]. `None` without a gender or an age.
    pub age_grade: Option<f64>,
    pub rank: Option<u32>,
    pub gender_rank: Option<u32>,
    pub general_rank: Option<u32>,
    pub general_gender_rank: Option<u32>,
}

/// Runners with the same surname who crossed the line within
/// [`FAMILY_WINDOW`] seconds of each other in the same race.
#[derive(Debug, Clone, Serialize)]
pub struct Family {
    pub family_id: u32,
    pub edition_id: u64,
    pub year: u16,
    pub race: RaceKind,
    pub surname: String,
    pub members: u32,
    /// Official time of the last member to finish.
    pub official_time: String,
    /// How many runners with this surname would finish this close by chance:
    /// near 0 means almost certainly together, above 0.1 possibly luck.
    pub chance: f64,
}

impl RaceKind {
    /// From the race name: "Competitive - bib from ..." and "COMPETITIVA" are
    /// competitive; "Not competitive", "Non competitiva" and plain names like
    /// 2016's "POLIMIRUN" (which has no real ranking) are not.
    pub fn of(race_name: &str) -> Self {
        let n = race_name.to_lowercase();
        if n.contains("compet") && !n.contains("non") && !n.contains("not") {
            Self::Competitive
        } else {
            Self::NonCompetitive
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Competitive => "competitive",
            Self::NonCompetitive => "non_competitive",
        }
    }
}

impl Row {
    fn new(edition: &Edition, race: RaceKind, e: &Entry) -> Self {
        let col = |k: &str| e.columns.get(k).map_or("", |v| v.trim());
        let official_time = clean_time(col("officialTime"));
        let real_time = clean_time(col("realTime"));
        let seconds = real_time.as_deref().or(official_time.as_deref()).and_then(parse_time);
        // XLS exports have no gender column, but Italian categories carry it (SM, PF, SF45...).
        let gender = match col("gender") {
            "" => col("category").chars().nth(1).filter(|c| matches!(c, 'M' | 'F')).map(String::from),
            g => Some(g.to_uppercase()),
        }
        .unwrap_or_default();
        let year = edition.year();
        let year_of_birth: Option<u16> = col("yearOfBirth").parse().ok();
        let age = year_of_birth.and_then(|y| year.checked_sub(y)).filter(|a| (10..=95).contains(a));
        let nationality = col("nationality").to_uppercase();
        Self {
            edition_id: edition.id,
            year,
            runner_id: 0,
            family_id: None,
            race,
            bib: col("bib").into(),
            name: col("name").into(),
            surname: surname(col("name")),
            age_grade: age.zip(seconds).and_then(|(a, s)| age_grade::percent(&gender, a, s)),
            gender,
            year_of_birth,
            age,
            team: col("teamName").into(),
            country: country::ioc(&nationality).into(),
            nationality,
            category: col("category").into(),
            official_time,
            real_time,
            seconds,
            start_delay: None,
            rank: None,
            gender_rank: None,
            general_rank: None,
            general_gender_rank: None,
        }
    }

    /// The time `rank` is based on.
    fn race_time(&self) -> Option<u32> {
        match self.race {
            RaceKind::Competitive => self.official_time.as_deref().and_then(parse_time),
            RaceKind::NonCompetitive => self.seconds,
        }
    }

    fn gun_minus_chip(&self) -> Option<u32> {
        let official = self.official_time.as_deref().and_then(parse_time)?;
        official.checked_sub(self.real_time.as_deref().and_then(parse_time)?)
    }
}

/// "0:31:28" or "01:37:39" -> seconds.
fn parse_time(t: &str) -> Option<u32> {
    // Some exports hold 00:00:00 for runners without a time.
    let s = t.split(':').try_fold(0, |acc, part| Some(acc * 60 + part.parse::<u32>().ok()?))?;
    (s > 0).then_some(s)
}

fn format_time(s: u32) -> String {
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn clean_time(raw: &str) -> Option<String> {
    match parse_time(raw) {
        Some(s) => Some(format_time(s)),
        None if raw.is_empty() || raw == "-" => None,
        None => Some(raw.into()),
    }
}

/// endu lists names surname first: "DI PRESA ALBERTO" -> "DI PRESA".
fn surname(name: &str) -> String {
    let words: Vec<String> = name.split_whitespace().map(str::to_uppercase).collect();
    if words.len() < 2 {
        return String::new();
    }
    let mut n = 1;
    while n < words.len() - 1 && SURNAME_PARTICLES.contains(&words[n - 1].as_str()) {
        n += 1;
    }
    words[..n].join(" ")
}

/// Sets `start_delay` on the rows of one race if its official time is gun
/// time, which is the case when most runners' official and chip times differ.
fn set_start_delays(race: &mut [Row]) {
    let timed = race.iter().filter(|r| r.gun_minus_chip().is_some()).count();
    let differ = race.iter().filter(|r| r.gun_minus_chip().is_some_and(|d| d > 0)).count();
    if differ * 2 > timed {
        for r in race {
            r.start_delay = r.gun_minus_chip();
        }
    }
}

/// Numbers the rows that have a `time` 1, 2, 3... by that time, separately for
/// each `group`. Equal times keep the source order.
fn assign_ranks<K: Eq + Hash>(
    rows: &mut [Row],
    time: impl Fn(&Row) -> Option<u32>,
    group: impl Fn(&Row) -> K,
    set: impl Fn(&mut Row, u32),
) {
    let mut order: Vec<usize> = (0..rows.len()).filter(|&i| time(&rows[i]).is_some()).collect();
    order.sort_by_key(|&i| time(&rows[i]));
    let mut counters = HashMap::new();
    for i in order {
        let n = counters.entry(group(&rows[i])).or_insert(0);
        *n += 1;
        set(&mut rows[i], *n);
    }
}

/// Gives the same `runner_id` to the rows of the same person across editions:
/// same name, ignoring accents, punctuation and word order, and same year of
/// birth. A row without a plausible year of birth (the 2022 and 2023 exports
/// have none) joins the only runner with its name; when there is none, or
/// several, those rows share one id per name. Names with fewer than two words
/// can't be matched and get their own id.
///
/// Ids are numbered 1.. in name order, so they only hold for this set of rows.
pub fn assign_runner_ids(rows: &mut [Row]) {
    let keys: Vec<(String, Option<u16>)> =
        rows.iter().map(|r| (name_key(&r.name), r.age.and(r.year_of_birth))).collect();

    let mut births: HashMap<&str, BTreeSet<u16>> = HashMap::new();
    for (name, yob) in &keys {
        births.entry(name).or_default().extend(yob);
    }
    let resolved: Vec<(&str, Option<u16>, usize)> = keys
        .iter()
        .enumerate()
        .map(|(i, (name, yob))| {
            let only = || Some(&births[name.as_str()]).filter(|b| b.len() == 1).and_then(|b| b.first().copied());
            let unmatchable = if name.contains(' ') { 0 } else { i + 1 };
            (name.as_str(), yob.or_else(only), unmatchable)
        })
        .collect();

    let ids: BTreeMap<_, u32> = resolved.iter().collect::<BTreeSet<_>>().into_iter().zip(1..).collect();
    for (row, key) in rows.iter_mut().zip(&resolved) {
        row.runner_id = ids[key];
    }
}

/// "D’Amico Nicolò" and "NICOLO D'AMICO" -> "AMICO D NICOLO".
fn name_key(name: &str) -> String {
    let ascii = deunicode::deunicode(name).to_uppercase();
    let mut words: Vec<&str> = ascii.split(|c: char| !c.is_ascii_alphabetic()).filter(|w| !w.is_empty()).collect();
    words.sort_unstable();
    words.join(" ")
}

/// Finds runners with the same surname (ignoring accents and punctuation) who
/// crossed the line together: same edition and race, official times chained
/// at most [`FAMILY_WINDOW`] seconds apart. Official time counts from the gun,
/// so close official times mean crossing the finish together. Groups where two
/// runners have the same full name are skipped: namesakes or double entries.
///
/// Sets `family_id` on the members and returns the families, numbered 1.. by
/// year, race and time.
pub fn assign_families(rows: &mut [Row]) -> Vec<Family> {
    let mut races: BTreeMap<(u64, RaceKind), Vec<(u32, usize)>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        if let Some(t) = r.official_time.as_deref().and_then(parse_time) {
            races.entry((r.edition_id, r.race)).or_default().push((t, i));
        }
    }

    let mut found: Vec<(u16, RaceKind, u32, String, Vec<usize>, f64)> = Vec::new();
    for ((_, race), mut timed) in races {
        timed.sort_unstable();
        let times: Vec<u32> = timed.iter().map(|&(t, _)| t).collect();
        let mut by_surname: BTreeMap<String, Vec<(u32, usize)>> = BTreeMap::new();
        for &(t, i) in &timed {
            let key: String = deunicode::deunicode(&rows[i].surname).chars().filter(char::is_ascii_alphabetic).collect();
            if !key.is_empty() {
                by_surname.entry(key).or_default().push((t, i));
            }
        }
        for same_surname in by_surname.values() {
            for group in same_surname.chunk_by(|a, b| b.0 - a.0 <= FAMILY_WINDOW) {
                let names: BTreeSet<&str> = group.iter().map(|&(_, i)| rows[i].name.as_str()).collect();
                if group.len() < 2 || names.len() < group.len() {
                    continue;
                }
                let (first, last) = (group[0].0, group[group.len() - 1].0);
                let around = times.partition_point(|&t| t <= last + FAMILY_WINDOW)
                    - times.partition_point(|&t| t + FAMILY_WINDOW < first)
                    - group.len();
                let chance = ((same_surname.len() - group.len()) * around) as f64 / times.len() as f64;
                let members: Vec<usize> = group.iter().map(|&(_, i)| i).collect();
                let first_row = &rows[members[0]];
                found.push((first_row.year, race, last, first_row.surname.clone(), members, chance));
            }
        }
    }

    found.sort_by(|a, b| (a.0, a.1, a.2, &a.3).cmp(&(b.0, b.1, b.2, &b.3)));
    found
        .into_iter()
        .zip(1..)
        .map(|((year, race, last, surname, members, chance), family_id)| {
            for &i in &members {
                rows[i].family_id = Some(family_id);
            }
            Family {
                family_id,
                edition_id: rows[members[0]].edition_id,
                year,
                race,
                surname,
                members: members.len() as u32,
                official_time: format_time(last),
                chance: (chance * 1000.0).round() / 1000.0,
            }
        })
        .collect()
}

/// Every finisher of both races of `edition`, ranked (see [`Row`]).
///
/// A race whose live results are gone (2022, 2023) is read from its XLS or PDF
/// export when endu still has one. Editions without results give no rows.
pub fn edition_results(client: &Client, edition: &Edition) -> Result<Vec<Row>, Error> {
    let Some(settings) = client.settings(edition.id)? else { return Ok(Vec::new()) };

    let mut rows = Vec::new();
    for race in &settings.races {
        let Some(ranking) = settings.general_ranking(race) else { continue };
        let mut entries = client.results(ranking)?;
        if entries.is_empty() {
            if let Some(download) = client.downloads(edition.id)?.remove(&ranking.option_id) {
                entries = client.export(&download)?;
            }
        }
        let kind = RaceKind::of(&race.name);
        let start = rows.len();
        rows.extend(entries.iter().map(|e| Row::new(edition, kind, e)));
        set_start_delays(&mut rows[start..]);
    }

    assign_ranks(&mut rows, Row::race_time, |r| r.race, |r, n| r.rank = Some(n));
    assign_ranks(&mut rows, |r| r.race_time().filter(|_| !r.gender.is_empty()), |r| (r.race, r.gender.clone()), |r, n| r.gender_rank = Some(n));
    assign_ranks(&mut rows, |r| r.seconds, |_| (), |r, n| r.general_rank = Some(n));
    assign_ranks(&mut rows, |r| r.seconds.filter(|_| !r.gender.is_empty()), |r| r.gender.clone(), |r, n| r.general_gender_rank = Some(n));
    Ok(rows)
}
