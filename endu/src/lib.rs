//! Minimal client for the public endu.net results API.
//!
//! Flow: [`Client::group_id`] finds an event from its URL slug, [`Client::editions`]
//! lists its editions, [`Client::settings`] describes the races of one edition and
//! [`Client::results`] downloads one ranking using several threads. Rankings no
//! longer served live may still exist as an XLS or PDF export: see
//! [`Client::downloads`] and [`Client::export`].

use calamine::{Data, Reader, Xls};
use pdf_extract::{MediaBox, OutputDev, OutputError, Transform};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::Cursor;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::Duration;

const BASE_URL: &str = "https://www.endu.net/api";
/// Largest page size the API accepts.
const PAGE_SIZE: u32 = 100;

/// Export headers (XLS, then PDF) and the API column each one maps to. XLS
/// `COGNOME` and `NOME` are joined into `name`; other headers are kept as they are.
const EXPORT_COLUMNS: [(&str, &str); 16] = [
    ("POS_ASSOLUTA", "absolutePosition"),
    ("PETTORALE", "bib"),
    ("TEAM", "teamName"),
    ("NAZIONALITA", "nationality"),
    ("CATEGORIA", "category"),
    ("POS_CAT", "positionInCategory"),
    ("POS_SESSO", "positionInGender"),
    ("TEMPO_UFFICIALE", "officialTime"),
    ("DISTACCO", "gapTime"),
    ("pett.", "bib"),
    ("atleta", "name"),
    ("sex", "gender"),
    ("team", "teamName"),
    ("naz", "nationality"),
    ("race time", "officialTime"),
    ("real time", "realTime"),
];

#[derive(Debug)]
pub enum Error {
    Http(ureq::Error),
    Xls(calamine::XlsError),
    Pdf(OutputError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(e) => e.fmt(f),
            Self::Xls(e) => write!(f, "xls: {e}"),
            Self::Pdf(e) => write!(f, "pdf: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Self::Http(e)
    }
}

impl From<calamine::XlsError> for Error {
    fn from(e: calamine::XlsError) -> Self {
        Self::Xls(e)
    }
}

impl From<OutputError> for Error {
    fn from(e: OutputError) -> Self {
        Self::Pdf(e)
    }
}

#[derive(Debug, Deserialize)]
pub struct Edition {
    pub id: u64,
    pub name: String,
    pub date: DateRange,
}

#[derive(Debug, Deserialize)]
pub struct DateRange {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub edition_id: u64,
    pub races: Vec<Race>,
}

#[derive(Debug, Deserialize)]
pub struct Race {
    pub id: u64,
    pub name: String,
    pub categories: Vec<Category>,
}

#[derive(Debug, Deserialize)]
pub struct Category {
    pub id: u64,
    pub name: String,
    pub options: Vec<RankingOption>,
}

/// One ranking list of a category, e.g. "Generale" or "Categoria SM".
#[derive(Debug, Deserialize)]
pub struct RankingOption {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub columns: Vec<String>,
    /// `general`, `general_male`, `general_female`; absent for per-category lists.
    #[serde(rename = "type")]
    pub kind: Option<String>,
}

/// The query parameters that identify one ranking list.
#[derive(Debug, Clone, Copy)]
pub struct Ranking {
    pub edition_id: u64,
    pub race_id: u64,
    pub category_id: u64,
    pub option_id: u64,
}

/// One finisher. `columns` holds the displayed fields (`bib`, `name`,
/// `officialTime`, ...), keyed by the API's column names.
#[derive(Debug, Deserialize)]
pub struct Entry {
    pub position: u64,
    pub columns: BTreeMap<String, String>,
}

/// Exported files of one ranking list.
#[derive(Debug, Deserialize)]
pub struct Download {
    pub pdf: Option<String>,
    pub xls: Option<String>,
}

#[derive(Deserialize)]
struct Page {
    items: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroupRef {
    edition_group_id: u64,
}

#[derive(Deserialize)]
struct Downloads {
    #[serde(default)]
    options: HashMap<u64, Download>,
}

impl Edition {
    pub fn year(&self) -> u16 {
        self.date.from.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0)
    }
}

impl Settings {
    /// The overall ranking of `race`, which lists every finisher.
    pub fn general_ranking(&self, race: &Race) -> Option<Ranking> {
        race.categories.iter().find_map(|c| {
            let o = c.options.iter().find(|o| o.kind.as_deref() == Some("general"))?;
            Some(Ranking {
                edition_id: self.edition_id,
                race_id: race.id,
                category_id: c.id,
                option_id: o.id,
            })
        })
    }
}

/// endu's exports store UTF-16 text that calamine returns one byte per char
/// ("P\0O\0S\0..."): those bytes are decoded again as UTF-16LE.
fn cell_text(cell: &Data) -> String {
    let s = cell.to_string();
    if !s.contains('\0') {
        return s.trim().into();
    }
    let bytes: Option<Vec<u8>> = s.chars().map(|c| u8::try_from(c).ok()).collect();
    let s = match bytes {
        Some(b) if b.len() % 2 == 0 => {
            let units: Vec<u16> = b.chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
            String::from_utf16_lossy(&units)
        }
        _ => s.replace('\0', ""),
    };
    s.trim().into()
}

pub struct Client {
    agent: ureq::Agent,
    concurrency: usize,
}

impl Client {
    /// `concurrency` is the number of pages fetched in parallel by [`Client::results`].
    pub fn new(concurrency: usize) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self { agent, concurrency: concurrency.max(1) }
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T, Error> {
        Ok(self
            .agent
            .get(format!("{BASE_URL}{path}"))
            .query_pairs(query.iter().map(|(k, v)| (*k, v.as_str())))
            .call()?
            .body_mut()
            .read_json()?)
    }

    /// The event group behind an endu.net URL, e.g. `polimirunspring` for
    /// `endu.net/events/polimirunspring`.
    pub fn group_id(&self, slug: &str) -> Result<u64, Error> {
        let group: GroupRef = self.get("/events/groups", &[("slug", slug.into())])?;
        Ok(group.edition_group_id)
    }

    pub fn editions(&self, group_id: u64) -> Result<Vec<Edition>, Error> {
        self.get(&format!("/events/groups/{group_id}"), &[])
    }

    /// `None` when the edition has no results.
    pub fn settings(&self, edition_id: u64) -> Result<Option<Settings>, Error> {
        match self.get(&format!("/results/events/{edition_id}/settings"), &[]) {
            Err(Error::Http(ureq::Error::StatusCode(404))) => Ok(None),
            r => r.map(Some),
        }
    }

    /// One page (1-based) of a ranking; empty past the last page.
    pub fn page(&self, r: Ranking, page: u32) -> Result<Vec<Entry>, Error> {
        let query = [
            ("editionId", r.edition_id),
            ("raceId", r.race_id),
            ("categoryId", r.category_id),
            ("optionId", r.option_id),
            ("page", page.into()),
            ("pageSize", PAGE_SIZE.into()),
        ]
        .map(|(k, v)| (k, v.to_string()));
        let page: Page = self.get("/results", &query)?;
        Ok(page.items)
    }

    /// Every entry of a ranking, in ranking order.
    ///
    /// The API reports no total, so workers claim page numbers from a shared
    /// counter until they hit an empty page.
    pub fn results(&self, r: Ranking) -> Result<Vec<Entry>, Error> {
        let next = AtomicU32::new(1);
        let pages = Mutex::new(Vec::new());

        thread::scope(|s| {
            let workers: Vec<_> = (0..self.concurrency)
                .map(|_| {
                    s.spawn(|| -> Result<(), Error> {
                        loop {
                            let n = next.fetch_add(1, Ordering::Relaxed);
                            let items = self.page(r, n)?;
                            if items.is_empty() {
                                return Ok(());
                            }
                            pages.lock().unwrap().push((n, items));
                        }
                    })
                })
                .collect();
            workers.into_iter().try_for_each(|w| w.join().unwrap())
        })?;

        let mut pages = pages.into_inner().unwrap();
        pages.sort_by_key(|(n, _)| *n);
        Ok(pages.into_iter().flat_map(|(_, items)| items).collect())
    }

    /// Exported files of an edition's rankings, by option id.
    pub fn downloads(&self, edition_id: u64) -> Result<HashMap<u64, Download>, Error> {
        let d: Downloads = self.get(&format!("/results/events/{edition_id}/downloads"), &[])?;
        Ok(d.options)
    }

    /// The rows of an exported ranking (see [`Client::downloads`]) as entries
    /// shaped like [`Client::results`] ones, read from the XLS when there is
    /// one, else from the PDF. Exports carry fewer fields than the live results.
    pub fn export(&self, download: &Download) -> Result<Vec<Entry>, Error> {
        match (&download.xls, &download.pdf) {
            (Some(url), _) => read_xls(self.fetch(url)?),
            (None, Some(url)) => read_pdf(&self.fetch(url)?),
            (None, None) => Ok(Vec::new()),
        }
    }

    fn fetch(&self, url: &str) -> Result<Vec<u8>, Error> {
        Ok(self.agent.get(url).call()?.body_mut().read_to_vec()?)
    }
}

fn api_column(header: String) -> String {
    let api = EXPORT_COLUMNS.iter().find(|(h, _)| *h == header);
    api.map_or(header, |(_, api)| api.to_string())
}

fn read_xls(bytes: Vec<u8>) -> Result<Vec<Entry>, Error> {
    let mut book = Xls::new(Cursor::new(bytes))?;
    let Some(sheet) = book.worksheet_range_at(0) else { return Ok(Vec::new()) };
    let sheet = sheet?;

    let mut rows = sheet.rows();
    let Some(header) = rows.next() else { return Ok(Vec::new()) };
    let header: Vec<String> = header.iter().map(|h| api_column(cell_text(h))).collect();

    Ok(rows
        .map(|cells| header.iter().cloned().zip(cells.iter().map(cell_text)))
        .map(BTreeMap::from_iter)
        .filter(|columns: &BTreeMap<_, _>| columns.values().any(|v| !v.is_empty()))
        .enumerate()
        .map(|(i, mut columns)| {
            let surname = columns.remove("COGNOME").unwrap_or_default();
            let first = columns.remove("NOME").unwrap_or_default();
            columns.insert("name".into(), format!("{surname} {first}").trim().into());
            Entry { position: i as u64 + 1, columns }
        })
        .collect())
}

/// endu's PDF reports are tables whose header row (`pett.`, `atleta`, ...) is
/// repeated on every page and gives the column positions. A line without a
/// bib continues the previous runner's cells: long names wrap onto the next
/// line, sometimes on the next page.
fn read_pdf(bytes: &[u8]) -> Result<Vec<Entry>, Error> {
    let doc = pdf_extract::Document::load_mem(bytes).map_err(OutputError::from)?;
    let mut glyphs = PdfGlyphs::default();
    pdf_extract::output_doc(&doc, &mut glyphs)?;

    let mut entries: Vec<Entry> = Vec::new();
    for page in glyphs.0 {
        let lines = pdf_lines(page);
        let is_header = |line: &[Glyph]| header_cells(line).first().is_some_and(|(_, h)| h == "pett.");
        let Some(h) = lines.iter().position(|l| is_header(l)) else { continue };
        let columns: Vec<(f64, String)> =
            header_cells(&lines[h]).into_iter().map(|(x, name)| (x, api_column(name))).collect();

        for line in &lines[h + 1..] {
            let mut cells: Vec<Vec<&Glyph>> = vec![Vec::new(); columns.len()];
            for g in line {
                // Cells start at, or just right of, their header.
                cells[columns.iter().rposition(|(x, _)| *x <= g.x + 2.0).unwrap_or(0)].push(g);
            }
            let row: BTreeMap<String, String> =
                columns.iter().zip(cells).map(|((_, name), gs)| (name.clone(), glyph_text(gs))).collect();

            let bib = row.get("bib").map_or("", String::as_str);
            if !bib.is_empty() && bib.bytes().all(|b| b.is_ascii_digit()) {
                entries.push(Entry { position: entries.len() as u64 + 1, columns: row });
            } else if bib.is_empty() {
                if let Some(last) = entries.last_mut() {
                    for (name, more) in row.into_iter().filter(|(_, v)| !v.is_empty()) {
                        let cell = last.columns.entry(name).or_default();
                        cell.push_str(if cell.is_empty() { "" } else { " " });
                        cell.push_str(&more);
                    }
                }
            } else {
                // The page footer ("Cronometraggio a cura di: ...") ends the table.
                break;
            }
        }
    }
    Ok(entries)
}

struct Glyph {
    x: f64,
    end: f64,
    y: f64,
    text: String,
}

/// The glyphs of each page, positioned.
#[derive(Default)]
struct PdfGlyphs(Vec<Vec<Glyph>>);

impl OutputDev for PdfGlyphs {
    fn begin_page(&mut self, _: u32, _: &MediaBox, _: Option<(f64, f64, f64, f64)>) -> Result<(), OutputError> {
        self.0.push(Vec::new());
        Ok(())
    }

    fn output_character(&mut self, trm: &Transform, width: f64, _: f64, font_size: f64, c: &str) -> Result<(), OutputError> {
        let size = font_size * ((trm.m11 + trm.m21) * (trm.m12 + trm.m22)).abs().sqrt();
        if let Some(page) = self.0.last_mut() {
            page.push(Glyph { x: trm.m31, end: trm.m31 + width * size, y: trm.m32, text: c.into() });
        }
        Ok(())
    }

    fn end_page(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn begin_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_word(&mut self) -> Result<(), OutputError> {
        Ok(())
    }

    fn end_line(&mut self) -> Result<(), OutputError> {
        Ok(())
    }
}

/// Groups a page's glyphs into lines, top to bottom, each sorted left to right.
fn pdf_lines(mut page: Vec<Glyph>) -> Vec<Vec<Glyph>> {
    page.sort_by(|a, b| b.y.total_cmp(&a.y));
    let mut lines: Vec<Vec<Glyph>> = Vec::new();
    for g in page {
        match lines.last_mut() {
            Some(line) if (line[0].y - g.y).abs() < 2.0 => line.push(g),
            _ => lines.push(vec![g]),
        }
    }
    for line in &mut lines {
        line.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
    lines
}

/// The words of a header line and where each starts.
fn header_cells(line: &[Glyph]) -> Vec<(f64, String)> {
    let mut cells: Vec<Vec<&Glyph>> = Vec::new();
    for g in line {
        match cells.last_mut() {
            Some(cell) if g.x - cell.last().unwrap().end < 3.0 => cell.push(g),
            _ => cells.push(vec![g]),
        }
    }
    cells.into_iter().map(|cell| (cell[0].x, glyph_text(cell))).collect()
}

/// Text of glyphs sorted left to right, with a space wherever they are apart.
fn glyph_text<'a>(glyphs: impl IntoIterator<Item = &'a Glyph>) -> String {
    let mut text = String::new();
    let mut end = f64::MIN;
    for g in glyphs {
        if g.x - end > 1.0 {
            text.push(' ');
        }
        text.push_str(&g.text);
        end = g.end;
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
