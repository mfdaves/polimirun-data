-- Tables written by `polimirun results --out file.db`. These objects, and the
-- views in views.sql, are replaced on every write; anything else in the file
-- is kept.

DROP VIEW IF EXISTS general;
DROP VIEW IF EXISTS competitive;
DROP VIEW IF EXISTS non_competitive;
DROP VIEW IF EXISTS yearly;
DROP VIEW IF EXISTS retention;
DROP VIEW IF EXISTS loyalty;
DROP VIEW IF EXISTS nations;
DROP VIEW IF EXISTS surnames;
DROP VIEW IF EXISTS time_distribution;
DROP VIEW IF EXISTS gender_gap;
DROP VIEW IF EXISTS age_groups;
DROP VIEW IF EXISTS progression;
DROP VIEW IF EXISTS progression_summary;
DROP VIEW IF EXISTS start_delay;
DROP VIEW IF EXISTS finish_flow;
DROP VIEW IF EXISTS clubs;
DROP VIEW IF EXISTS data_quality;
DROP TABLE IF EXISTS results;
DROP TABLE IF EXISTS runners;
DROP TABLE IF EXISTS families;

CREATE TABLE results (
    edition_id INTEGER NOT NULL,
    year INTEGER NOT NULL,
    runner_id INTEGER NOT NULL,
    family_id INTEGER,
    race TEXT NOT NULL,
    bib TEXT,
    name TEXT,
    surname TEXT,
    gender TEXT,
    year_of_birth INTEGER,
    age INTEGER,
    team TEXT,
    nationality TEXT,
    country TEXT,
    category TEXT,
    official_time TEXT,
    real_time TEXT,
    seconds INTEGER,
    start_delay INTEGER,
    age_grade REAL,
    rank INTEGER,
    gender_rank INTEGER,
    general_rank INTEGER,
    general_gender_rank INTEGER
);
CREATE INDEX results_runner ON results (runner_id);
CREATE INDEX results_race ON results (year, race);

-- One row per person, filled from results.
CREATE TABLE runners (
    runner_id INTEGER PRIMARY KEY,
    name TEXT,
    gender TEXT,
    year_of_birth INTEGER,
    country TEXT,
    editions INTEGER,
    first_year INTEGER,
    last_year INTEGER
);

CREATE TABLE families (
    family_id INTEGER PRIMARY KEY,
    edition_id INTEGER NOT NULL,
    year INTEGER NOT NULL,
    race TEXT NOT NULL,
    surname TEXT,
    members INTEGER,
    official_time TEXT,
    chance REAL
);
