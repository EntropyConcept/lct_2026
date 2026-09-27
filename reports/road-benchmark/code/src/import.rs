use crate::model::*;
use std::path::Path;

pub fn load(path: &Path) -> Result<Scenario> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse(&path.to_string_lossy(), &text)
}
pub fn parse(name: &str, text: &str) -> Result<Scenario> {
    let s = if name.to_lowercase().ends_with(".csv") {
        import_csv(name, text)?
    } else {
        serde_json::from_str(text).map_err(|e| format!("JSON: {e}"))?
    };
    validate_input(&s)?;
    Ok(s)
}
pub fn demo() -> Scenario {
    serde_json::from_str(include_str!("../demo.json")).expect("bundled demo")
}

fn time(value: &str) -> Result<u32> {
    let hhmm = value.split_whitespace().last().ok_or("Missing time")?;
    let (h, m) = hhmm.split_once(':').ok_or("Expected HH:MM")?;
    let h: u32 = h.parse().map_err(|_| "Invalid hour")?;
    let m: u32 = m.parse().map_err(|_| "Invalid minute")?;
    if h > 23 || m > 59 {
        return Err("Time outside one day".into());
    }
    Ok(h * 60 + m)
}
fn centroid(district: &str) -> Result<Point> {
    let (lat, lon) = match district.trim_start_matches("GPON ").trim() {
        "Даниловский" => (55.704, 37.638),
        "Академический" => (55.688, 37.573),
        "Котловка" => (55.674, 37.600),
        "Зюзино" => (55.658, 37.596),
        "Хамовники" => (55.729, 37.574),
        "Нагатино - Садовники" => (55.681, 37.641),
        "Замоскворечье" => (55.734, 37.632),
        "Нагатинский Затон" => (55.684, 37.695),
        "Нагорный" => (55.675, 37.620),
        "Донской" => (55.705, 37.601),
        "Гагаринский" => (55.699, 37.558),
        "Кузьминки" => (55.704, 37.775),
        "Таганский" => (55.740, 37.668),
        "Текстильщики" => (55.707, 37.738),
        "Рязанский" => (55.725, 37.780),
        "Южнопортовый" => (55.709, 37.687),
        "Нижегородский" => (55.732, 37.721),
        "Лефортово" => (55.758, 37.704),
        "Выхино" => (55.711, 37.817),
        "Басманный" => (55.771, 37.678),
        "Домодедово" => (55.437, 37.767),
        "Орехово Борисово Южное" => (55.604, 37.731),
        "Зябликово" => (55.613, 37.752),
        "Москворечье - Сабурово" => (55.640, 37.697),
        "Бирюлево Восточное" => (55.587, 37.671),
        "Кашира" => (54.835, 38.151),
        "Братеево" => (55.637, 37.765),
        "Царицыно" => (55.621, 37.670),
        "Орехово Борисово Северное" => (55.617, 37.706),
        "Бирюлево Западное" => (55.586, 37.642),
        "Ступино" => (54.886, 38.078),
        _ => {
            return Err(format!(
                "Unknown demo district: {district}; provide a JSON scenario with real coordinates"
            ))
        }
    };
    Ok(Point { lat, lon })
}
fn import_csv(name: &str, text: &str) -> Result<Scenario> {
    let norms = crate::norms::load()?;
    let norm_source = std::env::var_os("DISPATCH_NORMS")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            if Path::new("norms.xlsx").exists() {
                "norms.xlsx".into()
            } else {
                "встроенный norms.xlsx".into()
            }
        });
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b';')
        .trim(csv::Trim::All)
        .from_reader(text.trim_start_matches('\u{feff}').as_bytes());
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let column = |name: &str| {
        headers
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| format!("Missing CSV column {name}"))
    };
    let id = column("Заявка")?;
    let kind = column("Тип заявки BK")?;
    let start = column("Начало")?;
    let end = column("Окончание")?;
    let district = column("Район")?;
    let address = column("Адрес")?;
    let mut jobs = vec![];
    let mut day = None;
    for (line, row) in reader.records().enumerate() {
        let row = row.map_err(|e| format!("CSV row {}: {e}", line + 2))?;
        if row.iter().all(|v| v.is_empty()) || row[id].to_lowercase() == "адрес офиса" {
            continue;
        }
        let date = row[start]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_owned();
        if day.as_ref().is_some_and(|d| d != &date)
            || row[end].split_whitespace().next() != Some(date.as_str())
        {
            return Err(format!(
                "CSV row {}: only one-day scenarios are supported",
                line + 2
            ));
        }
        day = Some(date);
        let key =
            crate::norms::csv_key(&row[kind]).map_err(|e| format!("CSV row {}: {e}", line + 2))?;
        let norm = norms
            .iter()
            .find(|n| n.key == key)
            .ok_or_else(|| format!("Missing norm: {key}"))?;
        let skill = match key {
            "connection" | "equipment" => Skill::Connection,
            "local" => Skill::Local,
            "emergency" => Skill::Emergency,
            _ => unreachable!("validated CSV norm key"),
        };
        let mut point = centroid(&row[district])?;
        // Explicitly fictional positions, stable across imports; NOT geocoding.
        let hash = row[address].bytes().fold(2_166_136_261u32, |h, b| {
            (h ^ b as u32).wrapping_mul(16_777_619)
        });
        point.lat += (hash % 1201) as f64 / 100_000.0 - 0.006;
        point.lon += ((hash / 1201) % 1201) as f64 / 100_000.0 - 0.006;
        jobs.push(Job {
            id: row[id].to_owned(),
            address: row[address].to_owned(),
            point,
            duration: norm.service,
            work_type: Some(key.to_owned()),
            geocode_match: None,
            window_start: time(&row[start])?,
            window_end: time(&row[end])?,
            skill,
            transport: None,
            urgent: skill == Skill::Emergency,
        });
    }
    if jobs.is_empty() {
        return Err("CSV contains no jobs".into());
    }
    let start_point = Point {
        lat: jobs.iter().map(|j| j.point.lat).sum::<f64>() / jobs.len() as f64,
        lon: jobs.iter().map(|j| j.point.lon).sum::<f64>() / jobs.len() as f64,
    };
    let engineers = (0..12)
        .map(|i| Engineer {
            id: format!("E{:02}", i + 1),
            name: format!("Инженер {:02}", i + 1),
            start: start_point,
            shift_start: 540,
            shift_end: 1439,
            skills: match i % 4 {
                0 => vec![Skill::Local, Skill::Connection, Skill::Emergency],
                1 => vec![Skill::Local, Skill::Connection],
                2 => vec![Skill::Connection, Skill::Emergency],
                _ => vec![Skill::Local],
            },
            transport: match i % 6 {
                3 => Transport::Walk,
                4 => Transport::Bicycle,
                5 => Transport::Public,
                _ => Transport::Car,
            },
            already_used: false,
        })
        .collect();
    Ok(Scenario { name: name.to_owned(), jobs, engineers, routing: None, notes: vec![
        format!("Исходные данные: CSV {name} — заявки, типы работ, адреса, районы и окна; нормативы — {norm_source}. CSV не содержит координат или параметров инженеров. Это обогащённая ДЕМОНСТРАЦИЯ, не реальные маршруты по этим адресам."),
        "Длительность на месте = технические работы + документы по нормативу; нормативная дорога исключена, переезд считается отдельно маршрутизатором. Дозаказ оборудования имеет отдельный норматив, но требует навыка подключения.".into(),
        "Координаты вымышлены при импорте вокруг центров районов; 12 синтетических инженеров, общая база в центре точек, смена 09:00–23:59. Глобальные проблемы считаются срочными. Требований транспорта в CSV нет.".into(),
        "Поля контрольного распределения (бригада, статус) не используются как ограничения или эталон оптимума. Пустые строки и подпись адреса офиса пропущены. Для реального планирования загрузите полный JSON.".into(),
    ] })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_uses_service_and_keeps_equipment_distinct() {
        let csv = "Заявка;Тип заявки BK;Начало;Окончание;Район;Адрес\n\
            1;Подключение;2026-01-01 09:00;2026-01-01 18:00;Донской;A\n\
            2;Дозаказ;2026-01-01 09:00;2026-01-01 18:00;Донской;B\n\
            3;Локальная заявка;2026-01-01 09:00;2026-01-01 18:00;Донской;C\n\
            4;Глобальная проблема;2026-01-01 09:00;2026-01-01 18:00;Донской;D\n";
        let scenario = parse("source.csv", csv).unwrap();
        let norms = crate::norms::load().unwrap();
        for (job, key) in
            scenario
                .jobs
                .iter()
                .zip(["connection", "equipment", "local", "emergency"])
        {
            let norm = norms.iter().find(|n| n.key == key).unwrap();
            assert_eq!(job.duration, norm.technical + norm.documents);
            assert_eq!(job.work_type.as_deref(), Some(key));
        }
        assert_eq!(scenario.jobs[1].skill, Skill::Connection);
        assert!(scenario.jobs[3].urgent);
        assert!(scenario.routing.is_none());
        assert!(scenario.notes.iter().any(|n| n.contains("source.csv")));
        assert!(scenario
            .notes
            .iter()
            .any(|n| n.contains("Координаты вымышлены")));
    }

    #[test]
    fn explicit_json_duration_is_unchanged() {
        let scenario = parse(
            "custom.json",
            r#"{
            "name":"custom", "engineers":[], "jobs":[{
                "id":"1", "address":"A", "point":{"lat":55.7,"lon":37.6},
                "duration":123, "window_start":540, "window_end":1080,
                "skill":"connection", "work_type":"equipment"
            }]
        }"#,
        )
        .unwrap();
        assert_eq!(scenario.jobs[0].duration, 123);
        assert_eq!(scenario.jobs[0].work_type.as_deref(), Some("equipment"));
    }
}
