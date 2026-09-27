# polimirun-data

Downloads the results of [Polimirun](https://www.endu.net/events/polimirunspring), the
Politecnico di Milano run, from endu.net for every edition since 2016, and turns them into
a single table you can rank, filter and query. Output is a terminal table, CSV, JSON or a
SQLite database.

Why it exists: endu.net shows one race, one page at a time. The non-competitive race, which
is most of the field, has no ranking at all. The 2022 and 2023 results are no longer served
by the API and only survive as XLS and PDF exports. This tool collects all of it and ranks
everyone the same way.

## Personal use only

The results belong to endu.net. Its "Download the results" dialog states:

> Information, data and images, including results and rankings, may be used for personal
> use only, therefore their commercial use and redistribution, even partial, in any way or
> form without express authorization is prohibited. Downloads are tracked.

This tool is for looking at the results yourself. Don't publish, share or sell what it
produces, in whole or in part, without endu.net's authorization. For any other use, ask
endu.net.

The output also contains names, birth years and times of real people. This repository
holds only code and no endu.net data; `.gitignore` excludes `.db` and `.csv` files.

## Build

Needs Rust 1.88 or newer.

```
cargo build --release
```

The binary is `target/release/polimirun`.

## Usage

```
polimirun editions                                    # list the editions on endu.net
polimirun results --year 2026 --ranking non-competitive --gender F --top 10
polimirun results --name rossi                        # a name, across every year
polimirun results --ranking competitive --out competitive.csv
polimirun results --out polimirun.db                  # everything, as SQLite
```

`results` options:

| Option | Meaning |
|---|---|
| `--year Y` | Only this edition. Repeatable. Default: every year. |
| `--ranking R` | `general` (both races, default), `competitive` or `non-competitive`. |
| `--gender M\|F` | Only this gender. |
| `--name TEXT` | Names containing `TEXT`, case-insensitive. |
| `--top N` | First N rows of each edition. |
| `--out FILE` | Write `.csv`, `.json` or `.db` (SQLite) instead of printing a table. |
| `-j N` | Result pages downloaded in parallel. Default 8. |

A full download (all editions, about 72,500 rows) takes 5 to 15 seconds. With `-j 1` it
takes about a minute. Above 4 there is little gain.

## Output

Every finisher is one row, with the same columns for both races:

| Column | |
|---|---|
| `edition_id`, `year` | endu.net edition id and its year. |
| `runner_id` | The same person across years, see below. |
| `race` | `competitive` or `non_competitive`. |
| `bib`, `name`, `gender`, `year_of_birth`, `team`, `nationality`, `category` | As published. Gender is `M`, `F` or empty. |
| `official_time`, `real_time` | `H:MM:SS`. Official is gun time, real is chip time. Non-time values such as `DSQ` are kept as published. |
| `seconds` | Chip time in seconds, or official time when there is no chip time. |
| `age_grade` | `seconds` as a percentage of the 10 km standard for the runner's age and gender, see below. |
| `rank`, `gender_rank` | Position in the runner's own race. |
| `general_rank`, `general_gender_rank` | Position across both races of the edition. |

### How ranks are computed

- **Competitive race**: by official time. This reproduces the published ranking exactly.
- **Non-competitive race**: by chip time. endu.net publishes no ranking for it.
- **General**: both races together, by chip time (official time where there is none).
- Equal times keep endu's order, so there are no shared positions.
- Runners without a valid time (`DSQ`, `00:00:00`) have no rank. Runners with unknown gender
  have no gender ranks.

### Runner identity

`runner_id` links the rows of the same person across editions: same name, ignoring accents,
punctuation and word order, and same year of birth. Rows without a year of birth (the
2022 and 2023 non-competitive exports) join the only runner with that name. If there is no
such runner, or more than one, they share one id per name. Names of a single word get
their own id.

This is a heuristic. A typo in a name or birth year splits one person in two, and two
people with the same name and birth year become one. Ids are numbered on every export, so
don't store them elsewhere.

### Age grading

`age_grade` compares runners of different ages and genders: the 10 km standard time for
the runner's age and gender divided by their time. 100 matches the standard; road-running
convention calls 90 and above world class, 80 national, 70 regional and 60 local class.

The standards are Alan Jones's 2025 road age standards, with single-age bests by Tom
Bernhard, approved on 2025-01-10 by the USATF Masters Long Distance Running Council. They
come from
[AlanLyttonJones/Age-Grade-Tables](https://github.com/AlanLyttonJones/Age-Grade-Tables)
(`2025 Files/MaleRoadStd2025.xlsx` and `FemaleRoadStd2025.xlsx`, version 2025-07-27,
10 km column), released under CC0 1.0, and are copied into
[`polimirun/src/age_grade.rs`](polimirun/src/age_grade.rs).

- Age is the race year minus the year of birth, so it can be one year too high; the
  grade is then slightly generous.
- The course is taken to be exactly 10 km.
- There is no grade without a gender or with an impossible age (outside 10 to 95), which
  includes every 2022 and 2023 non-competitive runner.
- A grade above 100 beats the standard for that age, which in practice means a wrong
  birth year or someone running on another person's bib. A full download on 2026-09-27
  had two.

### SQLite

| Object | |
|---|---|
| `results` | One row per finisher, the columns above. |
| `runners` | One row per person: `runner_id`, `name` (latest spelling), `gender`, `year_of_birth`, `editions`, `first_year`, `last_year`. |
| `general`, `competitive`, `non_competitive` | Views over `results`, sorted by the matching rank. |

Writing to an existing file replaces these objects and leaves anything else in it alone.

Some queries:

```sql
-- Top 10 women of the 2026 non-competitive race
SELECT rank, gender_rank, bib, name, real_time
FROM non_competitive WHERE year = 2026 AND gender = 'F' LIMIT 10;

-- One person across the years
SELECT year, race, official_time, real_time, rank
FROM results WHERE runner_id = 123 ORDER BY year;

-- Best age-graded results of 2026, leaving out impossible ones
SELECT name, gender, 2026 - year_of_birth AS age, real_time, age_grade, race
FROM results WHERE year = 2026 AND age_grade <= 100
ORDER BY age_grade DESC LIMIT 10;

-- How many runners came back the next edition
WITH editions AS (
    SELECT year, lead(year) OVER (ORDER BY year) AS next_year
    FROM (SELECT DISTINCT year FROM results)
)
SELECT e.year || ' -> ' || e.next_year AS editions,
       count(DISTINCT a.runner_id) AS runners,
       count(DISTINCT b.runner_id) AS came_back
FROM editions e
JOIN results a ON a.year = e.year
LEFT JOIN results b ON b.runner_id = a.runner_id AND b.year = e.next_year
WHERE e.next_year IS NOT NULL
GROUP BY e.year;
```

## Coverage

Numbers from a download on 2026-09-27.

| Year | Finishers | Competitive | Non-competitive | Source |
|---|---|---|---|---|
| 2016 | 2,346 | – | 2,346 | API |
| 2017 | 5,430 | 336 | 5,094 | API |
| 2018 | 7,755 | 553 | 7,202 | API |
| 2019 | 6,167 | 499 | 5,668 | API |
| 2020 | – | – | – | none on endu.net |
| 2021 | 2,557 | 145 | 2,412 | API |
| 2022 | 4,756 | 194 | 4,562 | XLS (competitive), PDF (non-competitive) |
| 2023 | 8,048 | 560 | 7,488 | XLS (competitive), PDF (non-competitive) |
| 2024 | 9,925 | 545 | 9,380 | API |
| 2025 | 13,782 | 967 | 12,815 | API |
| 2026 | 11,789 | 865 | 10,924 | API |

- 2016 has a single race. endu.net lists it alphabetically with positions that don't
  follow time, so it is treated as non-competitive and ranked by time.
- 2022 and 2023: the XLS exports have no year of birth or chip time; gender is taken from
  the category (`SM`, `SF45`, ...). The PDF exports only have bib, name, nationality and
  official time.

## Known data issues

These come from the source and are left as published:

- Nationality codes mix standards: Spain is `ESP` or `SPA`, China `CHN` or `CIN`. A few are
  truncated (`EL`, `MÉX`).
- In 2017 and 2019 almost every runner is `ITA`, probably a default value.
- Some non-competitive birth years are impossible (the race year, 1900, `2`). They are
  ignored when matching runners.
- Nobody checks non-competitive times, and a few look implausible. Nothing is filtered.
- Two age grades are above 100 (a runner listed as 90 years old at 48:31, another as 63 at
  32:43): wrong birth years or swapped bibs.
- In 2023 bib 12969 appears twice.

## Layout

- `endu/`: a small client for endu.net's public results API, with nothing specific to
  Polimirun. It also reads endu's XLS and PDF exports into the same shape as live results.
- `polimirun/`: the row schema, rankings, runner identity and the CLI.

endu.net endpoints used (all public, no authentication):

| Endpoint | Returns |
|---|---|
| `GET /api/events/groups?slug={slug}` | The event group id for an `endu.net/events/{slug}` page. Polimirun is `polimirunspring`, group 6212. |
| `GET /api/events/groups/{groupId}` | The editions of an event. |
| `GET /api/results/events/{editionId}/settings` | Races, categories and ranking lists of an edition. 404 when there are no results. |
| `GET /api/results?editionId&raceId&categoryId&optionId&page&pageSize` | One page of a ranking. `pageSize` is at most 100. There is no total count: keep paging until a page is empty. |
| `GET /api/results/events/{editionId}/downloads` | PDF and XLS links for each ranking list. |

endu's XLS exports store text as UTF-16 in a way the `calamine` reader returns byte by byte,
so `endu` decodes it again. The PDFs are read by glyph position, using the header row
repeated on every page.
