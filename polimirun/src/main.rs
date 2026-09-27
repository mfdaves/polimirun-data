use clap::{Parser, Subcommand, ValueEnum};
use endu::Client;
use polimirun::{POLIMIRUN_GROUP, RaceKind, Row, assign_runner_ids, edition_results};
use std::cmp::Reverse;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Download Polimirun results from endu.net.
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

const SQLITE_SCHEMA: &str = "
DROP VIEW IF EXISTS general;
DROP VIEW IF EXISTS competitive;
DROP VIEW IF EXISTS non_competitive;
DROP TABLE IF EXISTS results;
DROP TABLE IF EXISTS runners;
CREATE TABLE results (
    edition_id INTEGER NOT NULL,
    year INTEGER NOT NULL,
    runner_id INTEGER NOT NULL,
    race TEXT NOT NULL,
    bib TEXT,
    name TEXT,
    gender TEXT,
    year_of_birth INTEGER,
    team TEXT,
    nationality TEXT,
    category TEXT,
    official_time TEXT,
    real_time TEXT,
    seconds INTEGER,
    rank INTEGER,
    gender_rank INTEGER,
    general_rank INTEGER,
    general_gender_rank INTEGER
);
CREATE INDEX results_runner ON results (runner_id);
CREATE TABLE runners (
    runner_id INTEGER PRIMARY KEY,
    name TEXT,
    gender TEXT,
    year_of_birth INTEGER,
    editions INTEGER,
    first_year INTEGER,
    last_year INTEGER
);
CREATE VIEW general AS
    SELECT * FROM results ORDER BY year DESC, general_rank NULLS LAST;
CREATE VIEW competitive AS
    SELECT * FROM results WHERE race = 'competitive' ORDER BY year DESC, rank NULLS LAST;
CREATE VIEW non_competitive AS
    SELECT * FROM results WHERE race = 'non_competitive' ORDER BY year DESC, rank NULLS LAST;
";

/// One row per person, named as in their latest edition.
const SQLITE_RUNNERS: &str = "
INSERT INTO runners
SELECT runner_id,
       (SELECT name FROM results x WHERE x.runner_id = r.runner_id ORDER BY year DESC LIMIT 1),
       max(nullif(gender, '')),
       max(year_of_birth),
       count(DISTINCT edition_id),
       min(year),
       max(year)
FROM results r
GROUP BY runner_id;
";

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Editions => {
            for e in Client::new(1).editions(POLIMIRUN_GROUP)? {
                println!("{}  {}  {}  {}", e.year(), e.id, &e.date.from[..10], e.name);
            }
        }
        Cmd::Results(q) => results(q)?,
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

    match (q.out, format) {
        (Some(path), Some(format)) => {
            save(&rows, &path, format)?;
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

fn save(rows: &[Row], path: &Path, format: Format) -> Result<()> {
    match format {
        Format::Csv => {
            let mut w = csv::Writer::from_path(path)?;
            for r in rows {
                w.serialize(r)?;
            }
            w.flush()?;
        }
        Format::Json => serde_json::to_writer(BufWriter::new(File::create(path)?), rows)?,
        Format::Sqlite => {
            let mut db = rusqlite::Connection::open(path)?;
            db.execute_batch(SQLITE_SCHEMA)?;
            let tx = db.transaction()?;
            let mut insert = tx.prepare(
                "INSERT INTO results VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            )?;
            for r in rows {
                insert.execute(rusqlite::params![
                    r.edition_id as i64,
                    r.year,
                    r.runner_id,
                    r.race.as_str(),
                    r.bib,
                    r.name,
                    r.gender,
                    r.year_of_birth,
                    r.team,
                    r.nationality,
                    r.category,
                    r.official_time,
                    r.real_time,
                    r.seconds,
                    r.rank,
                    r.gender_rank,
                    r.general_rank,
                    r.general_gender_rank,
                ])?;
            }
            drop(insert);
            tx.execute_batch(SQLITE_RUNNERS)?;
            tx.commit()?;
        }
    }
    Ok(())
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
