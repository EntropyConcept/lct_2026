//! Service norms: bundled XLSX, or DISPATCH_NORMS pointing to XLSX or JSON.
//! JSON is the serialized Vec<Norm> shape; all fields are required, minutes are
//! nonnegative integers, and service must equal technical + documents.
use crate::model::Result;
use calamine::{Data, Reader, Xlsx};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, io::Cursor, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Norm {
    pub key: String,
    pub name: String,
    pub travel: u32,
    pub technical: u32,
    pub documents: u32,
    pub total: u32,
    pub service: u32,
}

const KEYS: [&str; 4] = ["connection", "emergency", "equipment", "local"];
const HEADERS: [(&str, &str); 5] = [
    ("Название работы", "name"),
    ("Дорога до клиента/ТКД, мин.", "road_minutes"),
    ("Технические работы, мин.", "technical"),
    ("Документы, мин.", "documents"),
    ("Базовый норматив, мин.", "total"),
];

pub fn load() -> Result<Vec<Norm>> {
    match std::env::var_os("DISPATCH_NORMS") {
        None => match std::fs::read("norms.xlsx") {
            Ok(bytes) => from_xlsx(&bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                from_xlsx(include_bytes!("../norms.xlsx"))
            }
            Err(e) => Err(format!("Norms norms.xlsx: {e}")),
        },
        Some(path) => {
            let path = Path::new(&path);
            let bytes =
                std::fs::read(path).map_err(|e| format!("Norms {}: {e}", path.display()))?;
            match path.extension().and_then(|v| v.to_str()) {
                Some(ext) if ext.eq_ignore_ascii_case("xlsx") => from_xlsx(&bytes),
                Some(ext) if ext.eq_ignore_ascii_case("json") => from_json(&bytes),
                _ => Err("DISPATCH_NORMS must point to an .xlsx or .json file".into()),
            }
        }
    }
}

/// Exact CSV type mapping; equipment shares the connection skill, not its norm.
pub fn csv_key(value: &str) -> Result<&'static str> {
    match value {
        "Подключение" => Ok("connection"),
        "Глобальная проблема" => Ok("emergency"),
        "Дозаказ" => Ok("equipment"),
        "Локальная заявка" => Ok("local"),
        _ => Err(format!("Unknown CSV job type: {value}")),
    }
}

fn workbook_key(value: &str) -> Result<&'static str> {
    match value {
        "Подключение клиентов Базовая" => Ok("connection"),
        "Аварий на ТКД" => Ok("emergency"),
        "Дозаказ оборудования" => Ok("equipment"),
        "Локальная заявка/ремонт у клиента" => Ok("local"),
        _ => Err(format!("Unknown workbook work name: {value}")),
    }
}

fn validate(norms: Vec<Norm>) -> Result<Vec<Norm>> {
    let mut seen = HashSet::new();
    for n in &norms {
        if !KEYS.contains(&n.key.as_str()) || !seen.insert(n.key.as_str()) {
            return Err(format!("Unknown/duplicate norm key: {}", n.key));
        }
        if n.name.trim().is_empty()
            || n.technical.checked_add(n.documents) != Some(n.service)
            || n.travel.checked_add(n.service) != Some(n.total)
            || !(1..=1440).contains(&n.service)
        {
            return Err(format!("Invalid norm {}: require total = travel + technical + documents, service = technical + documents in 1..=1440, and a name", n.key));
        }
    }
    for key in KEYS {
        if !seen.contains(key) {
            return Err(format!("Missing norm: {key}"));
        }
    }
    Ok(norms)
}

fn from_json(bytes: &[u8]) -> Result<Vec<Norm>> {
    validate(serde_json::from_slice(bytes).map_err(|e| format!("Norms JSON: {e}"))?)
}

fn minutes(cell: &Data) -> Result<u32> {
    match cell {
        Data::Int(n) => u32::try_from(*n).map_err(|_| "Invalid integer minutes".into()),
        Data::Float(n)
            if n.is_finite() && *n >= 0.0 && *n <= u32::MAX as f64 && n.fract() == 0.0 =>
        {
            Ok(*n as u32)
        }
        _ => Err(format!(
            "Expected nonnegative integer minutes, got {cell:?}"
        )),
    }
}

fn from_xlsx(bytes: &[u8]) -> Result<Vec<Norm>> {
    let mut workbook = Xlsx::new(Cursor::new(bytes)).map_err(|e| format!("Norms XLSX: {e}"))?;
    let range = workbook
        .worksheet_range_at(0)
        .ok_or("Norms XLSX has no worksheet")?
        .map_err(|e| format!("Norms XLSX: {e}"))?;
    from_range(&range)
}

fn from_range(range: &calamine::Range<Data>) -> Result<Vec<Norm>> {
    let mut rows = range.rows();
    let headers = rows.next().ok_or("Norms worksheet is empty")?;
    if headers.len() != HEADERS.len() {
        return Err("Norms worksheet must have exactly five columns".into());
    }
    let mut columns = Vec::new();
    for (russian, english) in HEADERS {
        let matches: Vec<_> = headers
            .iter()
            .enumerate()
            .filter(|(_, cell)| matches!(cell, Data::String(s) if s == russian || s == english))
            .map(|(i, _)| i)
            .collect();
        if matches.len() != 1 {
            return Err(format!(
                "Missing/duplicate norms header: {russian} ({english})"
            ));
        }
        columns.push(matches[0]);
    }
    let mut norms = Vec::new();
    for (line, row) in rows.enumerate() {
        if row.iter().all(|cell| matches!(cell, Data::Empty)) {
            continue;
        }
        let parse_row = || -> Result<Norm> {
            let Data::String(name) = &row[columns[0]] else {
                return Err("Norm name must be text".into());
            };
            let key = workbook_key(name)?.to_owned();
            let travel = minutes(&row[columns[1]])?;
            let technical = minutes(&row[columns[2]])?;
            let documents = minutes(&row[columns[3]])?;
            let total = minutes(&row[columns[4]])?;
            let service = technical
                .checked_add(documents)
                .ok_or("Service minutes overflow")?;
            Ok(Norm {
                key,
                name: name.clone(),
                travel,
                technical,
                documents,
                total,
                service,
            })
        };
        norms.push(parse_row().map_err(|e| format!("Norms row {}: {e}", line + 2))?);
    }
    validate(norms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundled() -> Vec<Norm> {
        from_xlsx(include_bytes!("../norms.xlsx")).unwrap()
    }

    #[test]
    fn provided_workbook_and_json_roundtrip() {
        let norms = bundled();
        for (n, (key, technical, documents, total, service)) in norms.iter().zip([
            ("connection", 60, 10, 90, 70),
            ("emergency", 80, 0, 100, 80),
            ("equipment", 10, 10, 40, 20),
            ("local", 30, 0, 50, 30),
        ]) {
            assert_eq!(
                (
                    n.key.as_str(),
                    n.travel,
                    n.technical,
                    n.documents,
                    n.total,
                    n.service
                ),
                (key, 20, technical, documents, total, service)
            );
        }
        assert_eq!(
            from_json(&serde_json::to_vec(&norms).unwrap())
                .unwrap()
                .len(),
            4
        );
        for (label, key) in [
            ("Подключение", "connection"),
            ("Дозаказ", "equipment"),
            ("Локальная заявка", "local"),
            ("Глобальная проблема", "emergency"),
        ] {
            assert_eq!(csv_key(label).unwrap(), key);
        }
        assert!(csv_key("подключение").is_err());
    }

    #[test]
    fn rejects_invalid_norms() {
        let base = bundled();
        let mut missing = base.clone();
        missing.pop();
        assert!(validate(missing).is_err());
        let mut duplicate = base.clone();
        duplicate.push(base[0].clone());
        assert!(validate(duplicate).is_err());
        for (technical, documents, total, service) in [
            (60, 10, 91, 70),
            (60, 10, 90, 90),
            (0, 0, 20, 0),
            (1441, 0, 1461, 1441),
            (u32::MAX, 1, 20, 0),
        ] {
            let mut bad = base.clone();
            bad[0].technical = technical;
            bad[0].documents = documents;
            bad[0].total = total;
            bad[0].service = service;
            assert!(validate(bad).is_err());
        }
        for cell in [
            Data::Int(-1),
            Data::Float(1.5),
            Data::Float(f64::NAN),
            Data::Float(f64::INFINITY),
            Data::Float(u32::MAX as f64 + 1.0),
            Data::String("20".into()),
            Data::Bool(true),
            Data::Empty,
        ] {
            assert!(minutes(&cell).is_err());
        }
        let json = serde_json::to_string(&base).unwrap();
        for bad in [
            json.replacen("\"travel\":20", "\"travel\":-1", 1),
            json.replacen("\"travel\":20", "\"travel\":1.5", 1),
        ] {
            assert!(from_json(bad.as_bytes()).is_err());
        }
        let mut workbook = Xlsx::new(Cursor::new(include_bytes!("../norms.xlsx"))).unwrap();
        let range = workbook.worksheet_range_at(0).unwrap().unwrap();
        for replacement in [Data::Empty, Data::String(HEADERS[1].0.into())] {
            let mut bad = range.clone();
            bad.set_value((0, 0), replacement);
            assert!(from_range(&bad).is_err());
        }
        for (position, replacement) in [
            ((1, 0), Data::String("Unknown".into())),
            ((1, 0), Data::String(base[1].name.clone())),
            ((1, 1), Data::Float(-1.0)),
            ((1, 2), Data::Float(1.5)),
            ((1, 4), Data::Float(91.0)),
        ] {
            let mut bad = range.clone();
            bad.set_value(position, replacement);
            assert!(from_range(&bad).is_err());
        }
    }
}
