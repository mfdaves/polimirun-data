-- Views written with the tables in schema.sql. Times are HH:MM:SS; percentiles
-- take the value at that position, without interpolating.

-- Rankings

CREATE VIEW general AS
SELECT * FROM results ORDER BY year DESC, general_rank NULLS LAST;

CREATE VIEW competitive AS
SELECT * FROM results WHERE race = 'competitive' ORDER BY year DESC, rank NULLS LAST;

CREATE VIEW non_competitive AS
SELECT * FROM results WHERE race = 'non_competitive' ORDER BY year DESC, rank NULLS LAST;

-- Participation

-- Per year: finishers, change against the previous edition with results,
-- shares of women and foreigners, median time and winners. The share of women
-- counts runners of known gender only: in 2022 and 2023, the competitive race.
CREATE VIEW yearly AS
WITH y AS (
    SELECT year,
           count(*) AS finishers,
           sum(race = 'competitive') AS competitive,
           sum(race = 'non_competitive') AS non_competitive,
           round(100.0 * sum(gender = 'F') / nullif(sum(gender IN ('M', 'F')), 0), 1) AS women_pct,
           count(DISTINCT nullif(country, '')) AS countries,
           round(100.0 * sum(country NOT IN ('ITA', '')) / nullif(sum(country != ''), 0), 1) AS foreign_pct
    FROM results
    GROUP BY year
),
m AS (
    SELECT year, seconds,
           row_number() OVER (PARTITION BY year ORDER BY seconds) AS i,
           count(*) OVER (PARTITION BY year) AS n
    FROM results
    WHERE seconds IS NOT NULL
)
SELECT y.year, finishers,
       round(100.0 * (finishers - lag(finishers) OVER w) / lag(finishers) OVER w, 1) AS change_pct,
       competitive, non_competitive, women_pct, countries, foreign_pct,
       (SELECT time(seconds, 'unixepoch') FROM m WHERE m.year = y.year AND i = n / 2 + 1) AS median_time,
       (SELECT official_time FROM results r
        WHERE r.year = y.year AND race = 'competitive' AND gender = 'M' AND gender_rank = 1) AS winner_men,
       (SELECT official_time FROM results r
        WHERE r.year = y.year AND race = 'competitive' AND gender = 'F' AND gender_rank = 1) AS winner_women
FROM y
WINDOW w AS (ORDER BY y.year)
ORDER BY y.year;

-- Runners of each edition who came back to the next edition with results, and
-- the share of the next edition's runners who had run any earlier one.
CREATE VIEW retention AS
WITH e AS (
    SELECT year, lead(year) OVER (ORDER BY year) AS next_year
    FROM (SELECT DISTINCT year FROM results)
),
p AS (SELECT DISTINCT year, runner_id FROM results)
SELECT e.year, e.next_year,
       count(DISTINCT a.runner_id) AS runners,
       count(DISTINCT b.runner_id) AS came_back,
       round(100.0 * count(DISTINCT b.runner_id) / count(DISTINCT a.runner_id), 1) AS retention_pct,
       (SELECT round(100.0 * sum(EXISTS (SELECT 1 FROM p x WHERE x.runner_id = z.runner_id AND x.year < z.year))
                     / count(*), 1)
        FROM p z WHERE z.year = e.next_year) AS next_returning_pct
FROM e
JOIN p a ON a.year = e.year
LEFT JOIN p b ON b.runner_id = a.runner_id AND b.year = e.next_year
WHERE e.next_year IS NOT NULL
GROUP BY e.year;

-- People by number of editions finished.
CREATE VIEW loyalty AS
SELECT editions, count(*) AS people,
       round(100.0 * count(*) / (SELECT count(*) FROM runners), 1) AS pct
FROM runners
GROUP BY editions
ORDER BY editions;

-- People by country, as of their latest edition.
CREATE VIEW nations AS
SELECT country, count(*) AS people,
       round(100.0 * count(*) / (SELECT count(*) FROM runners), 2) AS pct
FROM runners
WHERE country != ''
GROUP BY country
ORDER BY people DESC, country;

-- People by surname.
CREATE VIEW surnames AS
SELECT surname, count(DISTINCT runner_id) AS people
FROM results
WHERE surname != ''
GROUP BY surname
ORDER BY people DESC, surname;

-- Performance

-- Percentiles and shares of finishing times, per year and race ('all' is both).
CREATE VIEW time_distribution AS
WITH t AS (
    SELECT year, race, seconds FROM results WHERE seconds IS NOT NULL
    UNION ALL
    SELECT year, 'all', seconds FROM results WHERE seconds IS NOT NULL
),
r AS (
    SELECT *, row_number() OVER (PARTITION BY year, race ORDER BY seconds) AS i,
           count(*) OVER (PARTITION BY year, race) AS n
    FROM t
)
SELECT year, race, max(n) AS finishers,
       time(max(CASE WHEN i = n / 10 + 1 THEN seconds END), 'unixepoch') AS p10,
       time(max(CASE WHEN i = n / 4 + 1 THEN seconds END), 'unixepoch') AS p25,
       time(max(CASE WHEN i = n / 2 + 1 THEN seconds END), 'unixepoch') AS median,
       time(max(CASE WHEN i = n * 3 / 4 + 1 THEN seconds END), 'unixepoch') AS p75,
       time(max(CASE WHEN i = n * 9 / 10 + 1 THEN seconds END), 'unixepoch') AS p90,
       round(100.0 * sum(seconds < 2400) / count(*), 1) AS under_40_min_pct,
       round(100.0 * sum(seconds < 3000) / count(*), 1) AS under_50_min_pct,
       round(100.0 * sum(seconds < 3600) / count(*), 1) AS under_60_min_pct,
       round(100.0 * sum(seconds > 5400) / count(*), 1) AS over_90_min_pct
FROM r
GROUP BY year, race
ORDER BY year, race;

-- How much slower women are than men, in %: the competitive winners, the
-- fastest 10% and the median of each gender. In 2022 and 2023 only the
-- competitive race has gender, so those years compare competitive runners.
CREATE VIEW gender_gap AS
WITH r AS (
    SELECT year, gender, seconds,
           row_number() OVER (PARTITION BY year, gender ORDER BY seconds) AS i,
           count(*) OVER (PARTITION BY year, gender) AS n
    FROM results
    WHERE seconds IS NOT NULL AND gender IN ('M', 'F')
),
q AS (
    SELECT year, gender,
           max(CASE WHEN i = n / 10 + 1 THEN seconds END) AS p10,
           max(CASE WHEN i = n / 2 + 1 THEN seconds END) AS median
    FROM r
    GROUP BY year, gender
),
w AS (SELECT year, gender, seconds AS winner FROM results WHERE race = 'competitive' AND gender_rank = 1)
SELECT f.year,
       round(100.0 * (wf.winner - wm.winner) / wm.winner, 1) AS winners_gap_pct,
       round(100.0 * (f.p10 - m.p10) / m.p10, 1) AS fastest_10pct_gap_pct,
       round(100.0 * (f.median - m.median) / m.median, 1) AS median_gap_pct
FROM q f
JOIN q m ON m.year = f.year AND m.gender = 'M'
LEFT JOIN w wf ON wf.year = f.year AND wf.gender = 'F'
LEFT JOIN w wm ON wm.year = f.year AND wm.gender = 'M'
WHERE f.gender = 'F'
ORDER BY f.year;

-- 5-year age groups by gender: runners, median and best time, and the mean age
-- grade leaving out grades above 100, which are data errors.
CREATE VIEW age_groups AS
WITH a AS (
    SELECT year, gender, seconds, age_grade,
           CASE WHEN age < 20 THEN 15 WHEN age >= 70 THEN 70 ELSE age / 5 * 5 END AS band
    FROM results
    WHERE age IS NOT NULL AND gender IN ('M', 'F') AND seconds IS NOT NULL
),
r AS (
    SELECT *, row_number() OVER (PARTITION BY year, gender, band ORDER BY seconds) AS i,
           count(*) OVER (PARTITION BY year, gender, band) AS n
    FROM a
)
SELECT year,
       CASE band WHEN 15 THEN 'under 20' WHEN 70 THEN '70+' ELSE band || '-' || (band + 4) END AS age_group,
       gender, max(n) AS runners,
       time(max(CASE WHEN i = n / 2 + 1 THEN seconds END), 'unixepoch') AS median_time,
       time(min(seconds), 'unixepoch') AS best_time,
       round(avg(CASE WHEN age_grade <= 100 THEN age_grade END), 1) AS mean_age_grade
FROM r
GROUP BY year, band, gender
ORDER BY year, band, gender;

-- Each runner's change in chip time from one edition to their next, in
-- seconds (negative is faster). Only runners with a year of birth, so matched
-- reliably, and editions with chip time.
CREATE VIEW progression AS
WITH p AS (
    SELECT runner_id, name, year, race, seconds
    FROM results
    WHERE real_time IS NOT NULL AND year_of_birth IS NOT NULL
),
l AS (
    SELECT runner_id, name, year AS from_year, race AS from_race, seconds AS from_seconds,
           lead(year) OVER w AS to_year, lead(race) OVER w AS to_race, lead(seconds) OVER w AS to_seconds
    FROM p
    WINDOW w AS (PARTITION BY runner_id ORDER BY year)
)
SELECT runner_id, name, from_year, to_year, from_race, to_race, to_seconds - from_seconds AS change_seconds
FROM l
WHERE to_year IS NOT NULL AND to_year != from_year;

CREATE VIEW progression_summary AS
WITH x AS (
    SELECT from_race || ' -> ' || to_race AS change, change_seconds FROM progression
    UNION ALL
    SELECT 'all', change_seconds FROM progression
),
r AS (
    SELECT *, row_number() OVER (PARTITION BY change ORDER BY change_seconds) AS i,
           count(*) OVER (PARTITION BY change) AS n
    FROM x
)
SELECT change, max(n) AS pairs,
       max(CASE WHEN i = n / 2 + 1 THEN change_seconds END) AS median_change_seconds,
       round(100.0 * sum(change_seconds < 0) / count(*), 1) AS faster_pct,
       round(100.0 * sum(change_seconds <= -300) / count(*), 1) AS faster_5_min_pct,
       round(100.0 * sum(change_seconds >= 300) / count(*), 1) AS slower_5_min_pct
FROM r
GROUP BY change
ORDER BY pairs DESC;

-- Race day

-- Wait between the gun and crossing the start line, in races that publish gun time.
CREATE VIEW start_delay AS
WITH r AS (
    SELECT year, race, start_delay AS d,
           row_number() OVER (PARTITION BY year, race ORDER BY start_delay) AS i,
           count(*) OVER (PARTITION BY year, race) AS n
    FROM results
    WHERE start_delay IS NOT NULL
)
SELECT year, race, max(n) AS runners,
       time(max(CASE WHEN i = n / 2 + 1 THEN d END), 'unixepoch') AS median,
       time(max(CASE WHEN i = n * 9 / 10 + 1 THEN d END), 'unixepoch') AS p90,
       time(max(d), 'unixepoch') AS longest,
       round(100.0 * sum(d > 60) / count(*), 1) AS over_1_min_pct,
       round(100.0 * sum(d > 300) / count(*), 1) AS over_5_min_pct
FROM r
GROUP BY year, race
ORDER BY year, race;

-- Finishers per minute after the gun, in races that publish gun time.
CREATE VIEW finish_flow AS
SELECT year, race, (seconds + start_delay) / 60 AS minute_after_gun, count(*) AS finishers
FROM results
WHERE start_delay IS NOT NULL AND seconds IS NOT NULL
GROUP BY year, race, minute_after_gun
ORDER BY year, race, minute_after_gun;

-- Competitive race clubs: runners, best position and the sum of the three best
-- positions (lower is better). Names are grouped as published, in upper case;
-- RUNCARD, an individual licence, is not a club.
CREATE VIEW clubs AS
WITH c AS (
    SELECT year, upper(trim(team)) AS club, rank,
           row_number() OVER (PARTITION BY year, upper(trim(team)) ORDER BY rank) AS i
    FROM results
    WHERE race = 'competitive' AND rank IS NOT NULL
      AND upper(trim(team)) NOT IN ('', 'RUNCARD', 'RUN CARD', 'LIBERO', 'INDIVIDUALE')
)
SELECT year, club, count(*) AS runners, min(rank) AS best_position,
       CASE WHEN count(*) >= 3 THEN sum(CASE WHEN i <= 3 THEN rank END) END AS top3_score
FROM c
GROUP BY year, club
ORDER BY year DESC, runners DESC, club;

-- Data quality

CREATE VIEW data_quality AS
SELECT 'no valid time' AS issue, count(*) AS rows_affected FROM results WHERE seconds IS NULL
UNION ALL
SELECT 'no year of birth', count(*) FROM results WHERE year_of_birth IS NULL
UNION ALL
SELECT 'impossible year of birth', count(*) FROM results WHERE year_of_birth IS NOT NULL AND age IS NULL
UNION ALL
SELECT 'unknown gender', count(*) FROM results WHERE gender = ''
UNION ALL
SELECT 'age grade above 100', count(*) FROM results WHERE age_grade > 100
UNION ALL
SELECT 'same runner twice in a year', count(*)
FROM (SELECT 1 FROM results GROUP BY year, runner_id HAVING count(*) > 1)
UNION ALL
SELECT 'nationality converted to IOC code', count(*) FROM results WHERE country != nationality;
