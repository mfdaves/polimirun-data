use clap::{Parser, Subcommand, ValueEnum};
use endu::Client;
use polimirun::{POLIMIRUN_GROUP, RaceKind, Row, assign_families, assign_runner_ids, edition_results};
use rusqlite::{Connection, OpenFlags, ToSql, params};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Download Polimirun results from endu.net, for personal use only.
///
/// endu.net allows its results and rankings to be used for personal use only:
/// commercial use and redistribution, even partial, need its express authorization.
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List the Polimirun editions.
    Editions,
    /// Fetch results, filter them, and print them or save them to a file.
    Results(Query),
    /// Positions and history of one runner, from a file saved with `results --out x.db`.
    Runner(RunnerQuery),
}

#[derive(clap::Args)]
struct Query {
    /// Edition year, repeatable. Every year when omitted.
    #[arg(long)]
    year: Vec<u16>,
    #[arg(long, value_enum, default_value_t = RankingSet::General)]
    ranking: RankingSet,
    /// M or F.
    #[arg(long)]
    gender: Option<String>,
    /// Part of the runner's name, case-insensitive.
    #[arg(long)]
    name: Option<String>,
    /// Keep the first N rows of each edition.
    #[arg(long)]
    top: Option<usize>,
    /// Output file: .csv, .json or .db (SQLite). Prints a table when omitted.
    #[arg(long)]
    out: Option<PathBuf>,
    /// Pages downloaded in parallel.
    #[arg(short = 'j', long, default_value_t = 8)]
    concurrency: usize,
}

#[derive(clap::Args)]
struct RunnerQuery {
    /// SQLite file saved with `results --out`.
    #[arg(long)]
    db: PathBuf,
    /// Edition year.
    #[arg(long)]
    year: u16,
    /// Bib number.
    #[arg(long, required_unless_present = "name")]
    bib: Option<String>,
    /// Part of the runner's name, case-insensitive.
    #[arg(long, conflicts_with = "bib")]
    name: Option<String>,
    /// Also rank among runners born in these years, e.g. 2000-2004.
    #[arg(long, value_parser = parse_years)]
    born: Option<(u16, u16)>,
}

#[derive(Clone, Copy, ValueEnum)]
enum RankingSet {
    /// Both races together, by chip time.
    General,
    Competitive,
    NonCompetitive,
}

enum Format {
    Csv,
    Json,
    Sqlite,
}

fn main() -> ExitCode {
    match run(Cli::parse().cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cmd: Cmd) -> Result<()> {
    match cmd {
        Cmd::Editions => {
            for e in Client::new(1).editions(POLIMIRUN_GROUP)? {
                println!("{}  {}  {}  {}", e.year(), e.id, &e.date.from[..10], e.name);
            }
        }
        Cmd::Results(q) => results(q)?,
        Cmd::Runner(q) => runner(q)?,
    }
    Ok(())
}

fn results(q: Query) -> Result<()> {
    // Check the output before spending time on the download.
    let format = q.out.as_deref().map(format_of).transpose()?;

    let client = Client::new(q.concurrency);
    let mut rows = Vec::new();
    for e in client.editions(POLIMIRUN_GROUP)? {
        if !q.year.is_empty() && !q.year.contains(&e.year()) {
            continue;
        }
        let edition = edition_results(&client, &e).map_err(|err| format!("{}: {err}", e.id))?;
        eprintln!("{} {}: {} finishers", e.year(), e.name, edition.len());
        rows.extend(edition);
    }
    assign_runner_ids(&mut rows);
    let mut families = assign_families(&mut rows);

    let (race, rank): (Option<RaceKind>, fn(&Row) -> Option<u32>) = match q.ranking {
        RankingSet::General => (None, |r| r.general_rank),
        RankingSet::Competitive => (Some(RaceKind::Competitive), |r| r.rank),
        RankingSet::NonCompetitive => (Some(RaceKind::NonCompetitive), |r| r.rank),
    };
    let name = q.name.map(|n| n.to_uppercase());
    rows.retain(|r| {
        race.is_none_or(|k| r.race == k)
            && q.gender.as_ref().is_none_or(|g| r.gender.eq_ignore_ascii_case(g))
            && name.as_ref().is_none_or(|n| r.name.to_uppercase().contains(n))
    });
    // Newest edition first, then by rank; runners without a rank last.
    rows.sort_by_key(|r| (Reverse(r.edition_id), rank(r).unwrap_or(u32::MAX)));
    if let Some(top) = q.top {
        let mut kept = HashMap::new();
        rows.retain(|r| {
            let n = kept.entry(r.edition_id).or_insert(0);
            *n += 1;
            *n <= top
        });
    }
    let kept: HashSet<u32> = rows.iter().filter_map(|r| r.family_id).collect();
    families.retain(|f| kept.contains(&f.family_id));

    match (q.out, format) {
        (Some(path), Some(format)) => {
            match format {
                Format::Csv => {
                    let mut w = csv::Writer::from_path(&path)?;
                    for r in &rows {
                        w.serialize(r)?;
                    }
                    w.flush()?;
                }
                Format::Json => serde_json::to_writer(BufWriter::new(File::create(&path)?), &rows)?,
                Format::Sqlite => polimirun::sqlite::save(&rows, &families, &path)?,
            }
            eprintln!("{} rows -> {}", rows.len(), path.display());
        }
        _ => print_table(&rows, q.ranking),
    }
    Ok(())
}

fn format_of(path: &Path) -> Result<Format> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("csv") => Ok(Format::Csv),
        Some("json") => Ok(Format::Json),
        Some("db" | "sqlite" | "sqlite3") => Ok(Format::Sqlite),
        _ => Err("--out must end in .csv, .json or .db".into()),
    }
}

fn print_table(rows: &[Row], ranking: RankingSet) {
    let or_dash = |v: Option<u32>| v.map_or("-".into(), |v| v.to_string());
    println!(
        "{:<4}  {:>5}  {:>6}  {:<15}  {:>5}  {:<30}  {:1}  {:>8}  {:>8}",
        "YEAR", "RANK", "SEX#", "RACE", "BIB", "NAME", "G", "OFFICIAL", "CHIP"
    );
    for r in rows {
        let (rank, gender_rank) = match ranking {
            RankingSet::General => (r.general_rank, r.general_gender_rank),
            _ => (r.rank, r.gender_rank),
        };
        println!(
            "{:<4}  {:>5}  {:>6}  {:<15}  {:>5}  {:<30.30}  {:1}  {:>8}  {:>8}",
            r.year,
            or_dash(rank),
            or_dash(gender_rank),
            r.race.as_str(),
            r.bib,
            r.name,
            r.gender,
            r.official_time.as_deref().unwrap_or("-"),
            r.real_time.as_deref().unwrap_or("-"),
        );
    }
}

fn parse_years(s: &str) -> std::result::Result<(u16, u16), String> {
    let (from, to) = s.split_once('-').unwrap_or((s, s));
    let year = |y: &str| y.trim().parse::<u16>().map_err(|_| format!("expected YEAR or YEAR-YEAR, got {s}"));
    Ok((year(from)?, year(to)?))
}

/// One finisher, as read back from the SQLite file.
struct Finisher {
    runner_id: u32,
    family_id: Option<u32>,
    race: String,
    bib: String,
    name: String,
    gender: String,
    year_of_birth: Option<u16>,
    age: Option<u16>,
    official_time: Option<String>,
    real_time: Option<String>,
    seconds: Option<u32>,
    age_grade: Option<f64>,
    rank: Option<u32>,
    gender_rank: Option<u32>,
    general_rank: Option<u32>,
    general_gender_rank: Option<u32>,
}

fn runner(q: RunnerQuery) -> Result<()> {
    let db = Connection::open_with_flags(&q.db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let pattern = q.name.as_ref().map(|n| format!("%{}%", n.to_uppercase()));
    let found: Vec<Finisher> = db
        .prepare(
            "SELECT runner_id, family_id, race, bib, name, gender, year_of_birth, age, official_time, real_time,
                    seconds, age_grade, rank, gender_rank, general_rank, general_gender_rank
             FROM results WHERE year = ?1 AND (bib = ?2 OR upper(name) LIKE ?3) ORDER BY name",
        )?
        .query_map(params![q.year, q.bib, pattern], |r| {
            Ok(Finisher {
                runner_id: r.get(0)?,
                family_id: r.get(1)?,
                race: r.get(2)?,
                bib: r.get(3)?,
                name: r.get(4)?,
                gender: r.get(5)?,
                year_of_birth: r.get(6)?,
                age: r.get(7)?,
                official_time: r.get(8)?,
                real_time: r.get(9)?,
                seconds: r.get(10)?,
                age_grade: r.get(11)?,
                rank: r.get(12)?,
                gender_rank: r.get(13)?,
                general_rank: r.get(14)?,
                general_gender_rank: r.get(15)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let me = match found.as_slice() {
        [me] => me,
        [] => return Err(format!("nobody in {} matches", q.year).into()),
        many => {
            for m in many {
                eprintln!("{:>6}  {}", m.bib, m.name);
            }
            return Err(format!("{} runners match, choose one with --bib", many.len()).into());
        }
    };

    let race = me.race.replace('_', "-");
    let people = match me.gender.as_str() {
        "M" => "men",
        "F" => "women",
        _ => "",
    };
    let born = me.year_of_birth.map_or(String::new(), |y| format!(", born {y}"));
    println!("{}, bib {}, {}{born}", me.name, me.bib, if me.gender.is_empty() { "gender unknown" } else { &me.gender });
    let grade = me.age_grade.map_or(String::new(), |g| format!(", age grade {g:.2}"));
    println!(
        "{} {race}: {} chip, {} official{grade}\n",
        q.year,
        me.real_time.as_deref().unwrap_or("-"),
        me.official_time.as_deref().unwrap_or("-")
    );
    let Some(rank) = me.rank else {
        println!("No valid time, so no positions.");
        return Ok(());
    };

    // Position among the runners of the same race matching `filter`.
    let position = |filter: &str, args: &[&dyn ToSql]| -> rusqlite::Result<(u32, u32)> {
        let sql = format!(
            "SELECT count(*) FILTER (WHERE rank < ?1) + 1, count(*) FROM results
             WHERE year = ?2 AND race = ?3 AND rank IS NOT NULL {filter}"
        );
        let mut all: Vec<&dyn ToSql> = vec![&rank, &q.year, &me.race];
        all.extend_from_slice(args);
        db.query_row(&sql, all.as_slice(), |r| Ok((r.get(0)?, r.get(1)?)))
    };
    let mut lines: Vec<(String, u32, u32)> = vec![];
    let (_, total) = position("", &[])?;
    lines.push((format!("{race} race"), rank, total));
    let band = me.age.map(|a| match a {
        ..20 => (10, 19, "under 20".to_string()),
        70.. => (70, 95, "70+".to_string()),
        _ => (a / 5 * 5, a / 5 * 5 + 4, format!("{}-{}", a / 5 * 5, a / 5 * 5 + 4)),
    });
    if let Some(gender_rank) = me.gender_rank {
        let (_, n) = position("AND gender = ?4", &[&me.gender])?;
        lines.push((format!("  {people}"), gender_rank, n));
        if let Some((lo, hi, label)) = &band {
            let (p, n) = position("AND gender = ?4 AND age BETWEEN ?5 AND ?6", &[&me.gender, lo, hi])?;
            lines.push((format!("  {people} aged {label}"), p, n));
        }
    }
    if let Some((from, to)) = q.born {
        let (p, n) = position("AND year_of_birth BETWEEN ?4 AND ?5", &[&from, &to])?;
        lines.push((format!("  born {from}-{to}"), p, n));
        if me.gender_rank.is_some() {
            let (p, n) = position("AND gender = ?4 AND year_of_birth BETWEEN ?5 AND ?6", &[&me.gender, &from, &to])?;
            lines.push((format!("  {people} born {from}-{to}"), p, n));
        }
    }
    if let Some(general) = me.general_rank {
        let n: u32 = db.query_row("SELECT count(general_rank) FROM results WHERE year = ?1", [q.year], |r| r.get(0))?;
        lines.push(("both races".into(), general, n));
    }
    if let Some(general_gender) = me.general_gender_rank {
        let n: u32 = db.query_row(
            "SELECT count(general_gender_rank) FROM results WHERE year = ?1 AND gender = ?2",
            params![q.year, me.gender],
            |r| r.get(0),
        )?;
        lines.push((format!("  {people}, both races"), general_gender, n));
    }
    println!("{:<34}  {:>8}  {:>8}  {:>5}", "Ranking", "Position", "Of", "Top");
    for (label, p, n) in &lines {
        let top = 100.0 * f64::from(*p) / f64::from(*n);
        // Under 1%, round up to a tenth: 2nd of 4168 is in the top 0.1%.
        let top = if top < 1.0 { format!("{:.1}%", (top * 10.0).ceil() / 10.0) } else { format!("{top:.0}%") };
        println!("{label:<34}  {p:>8}  {n:>8}  {top:>5}");
    }

    if let (Some((lo, hi, label)), Some(mine)) = (&band, me.seconds) {
        let times: Vec<u32> = db
            .prepare(
                "SELECT seconds FROM results WHERE year = ?1 AND race = ?2 AND gender = ?3
                 AND age BETWEEN ?4 AND ?5 AND seconds IS NOT NULL ORDER BY seconds",
            )?
            .query_map(params![q.year, me.race, me.gender, lo, hi], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        if let Some(&median) = times.get(times.len() / 2) {
            let diff = i64::from(mine) - i64::from(median);
            let side = if diff <= 0 { "faster" } else { "slower" };
            println!(
                "\nMedian of {people} aged {label} in the {race} race: {}, {} {side}",
                hms(median),
                minutes(diff.unsigned_abs() as u32)
            );
        }
    }
    let (tie_filter, tie_value): (&str, &dyn ToSql) = if me.race == "competitive" {
        ("official_time = ?3", &me.official_time)
    } else {
        ("seconds = ?3", &me.seconds)
    };
    let tied: u32 = db.query_row(
        &format!("SELECT count(*) FROM results WHERE year = ?1 AND race = ?2 AND {tie_filter}"),
        params![q.year, me.race, tie_value],
        |r| r.get(0),
    )?;
    if tied > 1 {
        let places = if tied == 2 { "place" } else { "places" };
        println!("{tied} runners share this time, so each position could be up to {} {places} better.", tied - 1);
    }

    let mut history = db.prepare(
        "SELECT year, race, official_time, real_time, rank,
                (SELECT count(rank) FROM results x WHERE x.year = r.year AND x.race = r.race)
         FROM results r WHERE runner_id = ?1 AND year != ?2 ORDER BY year",
    )?;
    let others: Vec<(u16, String, Option<String>, Option<String>, Option<u32>, u32)> = history
        .query_map(params![me.runner_id, q.year], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    if !others.is_empty() {
        println!("\nOther editions:");
        for (year, race, official, chip, rank, n) in others {
            let time = chip.or(official).unwrap_or_else(|| "-".into());
            let position = rank.map_or("-".into(), |r| format!("{r} of {n}"));
            println!("  {year}  {:<16} {time:>8}  {position}", race.replace('_', "-"));
        }
    }

    if let Some(family_id) = me.family_id {
        let (race_families, before, chance): (u32, u32, f64) = db.query_row(
            "SELECT (SELECT count(*) FROM families g WHERE g.year = f.year AND g.race = f.race),
                    (SELECT count(*) FROM families g WHERE g.year = f.year AND g.race = f.race
                     AND g.official_time < f.official_time),
                    chance
             FROM families f WHERE family_id = ?1",
            [family_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        println!(
            "\nCrossed the line with (family {} of {race_families} in the {race} race, chance {chance:.2}):",
            before + 1
        );
        let mut members = db.prepare(
            "SELECT name, year_of_birth, official_time FROM results
             WHERE family_id = ?1 AND NOT (bib = ?2 AND name = ?3) ORDER BY official_time",
        )?;
        for member in members.query_map(params![family_id, me.bib, me.name], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<u16>>(1)?, r.get::<_, Option<String>>(2)?))
        })? {
            let (name, born, time) = member?;
            let born = born.map_or(String::new(), |y| format!(", born {y}"));
            println!("  {name}{born}, {}", time.as_deref().unwrap_or("-"));
        }
    }
    Ok(())
}

fn hms(s: u32) -> String {
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// A gap such as "5:41", or "1:02:10" from an hour.
fn minutes(s: u32) -> String {
    if s < 3600 { format!("{}:{:02}", s / 60, s % 60) } else { hms(s) }
}
