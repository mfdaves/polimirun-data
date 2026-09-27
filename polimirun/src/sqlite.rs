//! Writes rows to a SQLite file: the tables in `schema.sql` and the analyses
//! in `views.sql`.

use crate::{Family, Row};
use rusqlite::{Connection, params};
use std::path::Path;

const SCHEMA: &str = include_str!("schema.sql");
const VIEWS: &str = include_str!("views.sql");

/// One row per person, named and placed as in their latest edition.
const RUNNERS: &str = "
INSERT INTO runners
SELECT runner_id,
       (SELECT name FROM results x WHERE x.runner_id = r.runner_id ORDER BY year DESC LIMIT 1),
       max(nullif(gender, '')),
       max(year_of_birth),
       (SELECT country FROM results x WHERE x.runner_id = r.runner_id ORDER BY year DESC LIMIT 1),
       count(DISTINCT edition_id),
       min(year),
       max(year)
FROM results r
GROUP BY runner_id;
";

/// Replaces the tables and views in `path`; anything else in the file is kept.
pub fn save(rows: &[Row], families: &[Family], path: &Path) -> rusqlite::Result<()> {
    let mut db = Connection::open(path)?;
    let tx = db.transaction()?;
    tx.execute_batch(SCHEMA)?;
    {
        let mut insert = tx.prepare(
            "INSERT INTO results VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
             ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
        )?;
        for r in rows {
            insert.execute(params![
                r.edition_id as i64,
                r.year,
                r.runner_id,
                r.family_id,
                r.race.as_str(),
                r.bib,
                r.name,
                r.surname,
                r.gender,
                r.year_of_birth,
                r.age,
                r.team,
                r.nationality,
                r.country,
                r.category,
                r.official_time,
                r.real_time,
                r.seconds,
                r.start_delay,
                r.age_grade,
                r.rank,
                r.gender_rank,
                r.general_rank,
                r.general_gender_rank,
            ])?;
        }
        let mut insert = tx.prepare("INSERT INTO families VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")?;
        for f in families {
            insert.execute(params![
                f.family_id,
                f.edition_id as i64,
                f.year,
                f.race.as_str(),
                f.surname,
                f.members,
                f.official_time,
                f.chance,
            ])?;
        }
    }
    tx.execute_batch(RUNNERS)?;
    tx.execute_batch(VIEWS)?;
    tx.commit()
}
