//! Polimirun results on top of the [`endu`] client: one row schema for both
//! races, with rankings. [`edition_results`] is the entry point.

use endu::{Client, Edition, Entry, Error};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::Hash;

pub mod age_grade;

/// Event group holding every Polimirun edition (`endu.net/events/polimirunspring`).
pub const POLIMIRUN_GROUP: u64 = 6212;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
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
    pub race: RaceKind,
    pub bib: String,
    pub name: String,
    pub gender: String,
    pub year_of_birth: Option<u16>,
    pub team: String,
    pub nationality: String,
    pub category: String,
    /// `H:MM:SS`, or the raw value when it is not a time (e.g. `DSQ`).
    pub official_time: Option<String>,
    pub real_time: Option<String>,
    /// Chip time in seconds (official time when there is no chip time).
    pub seconds: Option<u32>,
    /// `seconds` as a percentage of the 10 km standard for the runner's age
    /// and gender, see [`age_grade`]. `None` without a gender or a plausible age.
    pub age_grade: Option<f64>,
    pub rank: Option<u32>,
    pub gender_rank: Option<u32>,
    pub general_rank: Option<u32>,
    pub general_gender_rank: Option<u32>,
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
        };
        let mut row = Self {
            edition_id: edition.id,
            year: edition.year(),
            runner_id: 0,
            race,
            bib: col("bib").into(),
            name: col("name").into(),
            gender: gender.unwrap_or_default(),
            year_of_birth: col("yearOfBirth").parse().ok(),
            team: col("teamName").into(),
            nationality: col("nationality").to_uppercase(),
            category: col("category").into(),
            official_time,
            real_time,
            seconds,
            age_grade: None,
            rank: None,
            gender_rank: None,
            general_rank: None,
            general_gender_rank: None,
        };
        row.age_grade = row.age().zip(row.seconds).and_then(|(age, s)| age_grade::percent(&row.gender, age, s));
        row
    }

    /// Age on race day as race year minus year of birth, so possibly one year
    /// too many. `None` outside 10 to 95, where the year of birth is wrong.
    fn age(&self) -> Option<u16> {
        let age = self.year.checked_sub(self.year_of_birth?)?;
        (10..=95).contains(&age).then_some(age)
    }

    /// The time `rank` is based on.
    fn race_time(&self) -> Option<u32> {
        match self.race {
            RaceKind::Competitive => self.official_time.as_deref().and_then(parse_time),
            RaceKind::NonCompetitive => self.seconds,
        }
    }
}

/// "0:31:28" or "01:37:39" -> seconds.
fn parse_time(t: &str) -> Option<u32> {
    // Some exports hold 00:00:00 for runners without a time.
    let s = t.split(':').try_fold(0, |acc, part| Some(acc * 60 + part.parse::<u32>().ok()?))?;
    (s > 0).then_some(s)
}

fn clean_time(raw: &str) -> Option<String> {
    match parse_time(raw) {
        Some(s) => Some(format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)),
        None if raw.is_empty() || raw == "-" => None,
        None => Some(raw.into()),
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
    let keys: Vec<(String, Option<u16>)> = rows
        .iter()
        .map(|r| (name_key(&r.name), r.age().and(r.year_of_birth)))
        .collect();

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
        rows.extend(entries.iter().map(|e| Row::new(edition, kind, e)));
    }

    assign_ranks(&mut rows, Row::race_time, |r| r.race, |r, n| r.rank = Some(n));
    assign_ranks(&mut rows, |r| r.race_time().filter(|_| !r.gender.is_empty()), |r| (r.race, r.gender.clone()), |r, n| r.gender_rank = Some(n));
    assign_ranks(&mut rows, |r| r.seconds, |_| (), |r, n| r.general_rank = Some(n));
    assign_ranks(&mut rows, |r| r.seconds.filter(|_| !r.gender.is_empty()), |r| r.gender.clone(), |r, n| r.general_gender_rank = Some(n));
    Ok(rows)
}
