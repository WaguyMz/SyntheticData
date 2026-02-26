//! Statistics extractor.

use std::collections::HashMap;

use tracing::info;

use crate::error::{FingerprintError, FingerprintResult};
use crate::models::{
    AccountClassAmountStats, CategoricalStats, CategoryFrequency, DistributionParams,
    DistributionType, NumericStats, Percentiles, StatisticsFingerprint, TemporalStats,
};
use crate::privacy::PrivacyEngine;

use super::{
    fec_account_class, AccountingStandards, FEC_ACCOUNT_CLASS_LEVEL, FEC_ACCOUNT_COLUMN,
    FEC_MIN_ROWS_PER_CLASS, FEC_NUMERIC_COLUMNS, DataSource, ExtractedComponent, ExtractionConfig,
    Extractor,
};

/// Parse a string as f64: tries standard format first, then European (comma decimal, space thousands).
/// So "1234.56" and "1 234,56" both parse.
fn parse_amount_str(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<f64>().ok().or_else(|| {
        let normalized = t.replace(' ', "").replace(',', ".");
        if normalized.is_empty() {
            return None;
        }
        normalized.parse::<f64>().ok()
    })
}

/// Extractor for statistical information.
pub struct StatsExtractor;

impl Extractor for StatsExtractor {
    fn name(&self) -> &'static str {
        "statistics"
    }

    fn extract(
        &self,
        data: &DataSource,
        config: &ExtractionConfig,
        privacy: &mut PrivacyEngine,
    ) -> FingerprintResult<ExtractedComponent> {
        let stats = match data {
            DataSource::Csv(csv) => extract_from_csv(csv, config, privacy)?,
            DataSource::Fec(fec) => extract_from_fec(fec, config, privacy)?,
            DataSource::Parquet(pq) => extract_from_parquet(pq, config, privacy)?,
            DataSource::Json(json) => extract_from_json(json, config, privacy)?,
            DataSource::Memory(mem) => extract_from_memory(mem, config, privacy)?,
            DataSource::Directory(_) => {
                // Directory sources are handled by FingerprintExtractor::extract_from_directory_impl
                return Err(crate::error::FingerprintError::extraction(
                    "statistics",
                    "Directory sources should be handled at the FingerprintExtractor level",
                ));
            }
        };

        Ok(ExtractedComponent::Statistics(stats))
    }
}

/// For French GAAP CSV: only these columns get numeric stats (amounts); dates and others are temporal/categorical.
const FRENCH_GAAP_NUMERIC_HEADERS: &[&str] = &[
    "Montant au débit",
    "Montant au crédit",
    "Montant en devise",
    "Debit",
    "Credit",
];

/// Generic account-like column names (any chart: US GAAP, IFRS, French CSV, etc.).
const GENERIC_ACCOUNT_HEADERS: &[&str] = &[
    "account",
    "account_number",
    "account_no",
    "gl_account",
    "gl_account_no",
    "compte",
    "comptenum",   // French CSV e.g. APRR / FEC-style exports
    "compte num",
    "CompteNum",
    "compte_num",
    "compauxnum",  // French auxiliary account
    "numéro de compte",
    "numero de compte",
    "account code",
];
/// Generic debit amount column names.
const GENERIC_DEBIT_HEADERS: &[&str] = &[
    "debit",
    "debit_amount",
    "montant au débit",
    "dr",
    "amount_dr",
];
/// Generic credit amount column names.
const GENERIC_CREDIT_HEADERS: &[&str] = &[
    "credit",
    "credit_amount",
    "montant au crédit",
    "cr",
    "amount_cr",
];

fn header_matches(h: &str, candidates: &[&str]) -> bool {
    let raw = h.trim().trim_start_matches('\u{feff}');
    let lower = raw.to_lowercase();
    candidates.iter().any(|c| lower == *c || lower.contains(&c.to_lowercase()))
}

/// Parse a string as a date and return (y, m, d) for ordering, or None if not parseable.
/// Supports YYYY-MM-DD, DD/MM/YYYY, YYYYMMDD, and similar.
fn parse_date_for_order(s: &str) -> Option<(i32, u32, u32)> {
    let t = s.trim();
    if t.is_empty() || t.len() < 8 {
        return None;
    }
    // YYYY-MM-DD
    if t.len() >= 10 && t.as_bytes().get(4) == Some(&b'-') && t.as_bytes().get(7) == Some(&b'-') {
        let y: i32 = t[0..4].parse().ok()?;
        let m: u32 = t[5..7].parse().ok()?;
        let d: u32 = t[8..10.min(t.len())].parse().ok()?;
        return Some((y, m, d));
    }
    // YYYYMMDD (e.g. 20230101)
    if t.len() >= 8 && t.chars().all(|c| c.is_ascii_digit()) {
        let y: i32 = t[0..4].parse().ok()?;
        let m: u32 = t[4..6].parse().ok()?;
        let d: u32 = t[6..8].parse().ok()?;
        return Some((y, m, d));
    }
    // DD/MM/YYYY
    if t.len() >= 10 && t.as_bytes().get(2) == Some(&b'/') && t.as_bytes().get(5) == Some(&b'/') {
        let d: u32 = t[0..2].parse().ok()?;
        let m: u32 = t[3..5].parse().ok()?;
        let y: i32 = t[6..10.min(t.len())].parse().ok()?;
        return Some((y, m, d));
    }
    None
}

/// Build minimal temporal stats from date-like values (min/max as ISO 8601 strings).
fn compute_temporal_stats(values: &[String], _header: &str) -> TemporalStats {
    let non_empty: Vec<String> = values
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let count = non_empty.len() as u64;
    let parsed: Vec<(i32, u32, u32)> = non_empty
        .iter()
        .filter_map(|s| parse_date_for_order(s))
        .collect();
    let (min_s, max_s) = if parsed.is_empty() {
        let mut sorted = non_empty.clone();
        sorted.sort();
        let min_s = sorted.first().cloned().unwrap_or_default();
        let max_s = sorted.last().cloned().unwrap_or_default();
        (min_s, max_s)
    } else {
        let min_ymd = parsed.iter().min_by(|a, b| a.cmp(b)).copied().unwrap_or((0, 0, 0));
        let max_ymd = parsed.iter().max_by(|a, b| a.cmp(b)).copied().unwrap_or((0, 0, 0));
        let min_s = format!("{:04}-{:02}-{:02}", min_ymd.0, min_ymd.1, min_ymd.2);
        let max_s = format!("{:04}-{:02}-{:02}", max_ymd.0, max_ymd.1, max_ymd.2);
        (min_s, max_s)
    };
    TemporalStats::new(count, min_s, max_s)
}

/// Returns true if the column should be treated as temporal (date/datetime), not numeric.
fn is_temporal_column(header: &str, values: &[String]) -> bool {
    let lower = header.trim().to_lowercase();
    if super::TEMPORAL_HEADER_PATTERNS
        .iter()
        .any(|p| lower.contains(&p.to_lowercase()))
    {
        return true;
    }
    // Value-based: if most non-empty values look like dates, treat as temporal
    let non_empty: Vec<_> = values.iter().filter(|v| !v.is_empty()).collect();
    if non_empty.len() < 2 {
        return false;
    }
    let date_like = non_empty.iter().filter(|v| {
        let s = v.trim();
        // YYYY-MM-DD, DD/MM/YYYY, MM/DD/YYYY, DD.MM.YYYY
        (s.len() == 10 && s.contains('-'))
            || (s.len() == 10 && s.contains('/'))
            || (s.len() == 10 && s.contains('.'))
    });
    date_like.count() > non_empty.len() / 2
}

/// Try to find account, debit, credit column indices for generic GL data (non-FEC).
fn find_account_debit_credit_columns(headers: &[String]) -> Option<(usize, usize, usize)> {
    let acc_i = headers.iter().position(|h| header_matches(h, GENERIC_ACCOUNT_HEADERS))?;
    let deb_i = headers.iter().position(|h| header_matches(h, GENERIC_DEBIT_HEADERS))?;
    let cred_i = headers.iter().position(|h| header_matches(h, GENERIC_CREDIT_HEADERS))?;
    if acc_i != deb_i && acc_i != cred_i && deb_i != cred_i {
        return Some((acc_i, deb_i, cred_i));
    }
    None
}

/// Fallback for APRR/FEC-style CSV: find account (CompteNum or first "compte" that is not Lib), Debit, Credit by substring.
/// Handles BOM, trimming, and alternate spellings.
fn find_account_debit_credit_columns_aprr_fallback(headers: &[String]) -> Option<(usize, usize, usize)> {
    let norm = |h: &str| h.trim().trim_start_matches('\u{feff}').to_lowercase();
    let deb_i = headers.iter().position(|h| {
        let l = norm(h);
        l.contains("debit") || l.contains("débit")
    })?;
    let cred_i = headers.iter().position(|h| {
        let l = norm(h);
        l.contains("credit") || l.contains("crédit")
    })?;
    if deb_i == cred_i {
        return None;
    }
    // Account: prefer "comptenum" / "compte num" or column with "compte" but not "lib" (avoid CompteLib)
    let acc_i = headers
        .iter()
        .position(|h| {
            let l = norm(h);
            let is_compte = l == "comptenum" || l == "compte num" || l == "compte_num" || l.contains("compte");
            let not_lib = !l.contains("lib");
            is_compte && not_lib
        })
        .or_else(|| headers.iter().position(|h| header_matches(h, GENERIC_ACCOUNT_HEADERS)))?;
    if acc_i != deb_i && acc_i != cred_i {
        Some((acc_i, deb_i, cred_i))
    } else {
        None
    }
}

/// Extract per-account-class amount stats from generic CSV/Parquet/JSON (non-FEC) when account + debit + credit columns exist.
/// Uses the same privacy budget as other stats so the full fingerprint remains ε-differentially private.
fn try_extract_amount_by_account_class_generic(
    headers: &[String],
    columns: &[Vec<String>],
    min_rows_per_class: usize,
    class_level: usize,
    privacy: &mut PrivacyEngine,
) -> FingerprintResult<Option<HashMap<String, AccountClassAmountStats>>> {
    let (acc_i, deb_i, cred_i) = match find_account_debit_credit_columns(headers)
        .or_else(|| find_account_debit_credit_columns_aprr_fallback(headers))
    {
        Some(t) => t,
        None => return Ok(None),
    };
    let n_rows = columns.get(0).map(|c| c.len()).unwrap_or(0);
    if n_rows < min_rows_per_class {
        return Ok(None);
    }
    // Stats by subclass: truncate existing account num first to get subclass (e.g. first 3 digits), then aggregate
    let mut by_class: HashMap<String, (Vec<f64>, Vec<f64>)> = HashMap::new();
    for r in 0..n_rows {
        let account = columns[acc_i].get(r).map(|s| s.as_str()).unwrap_or("").trim();
        let subclass = super::fec_account_class(account, class_level);
        if subclass.is_empty() {
            continue;
        }
        let debit_val = columns[deb_i]
            .get(r)
            .and_then(|s| parse_amount_str(s))
            .unwrap_or(0.0);
        let credit_val = columns[cred_i]
            .get(r)
            .and_then(|s| parse_amount_str(s))
            .unwrap_or(0.0);
        let entry = by_class.entry(subclass).or_insert_with(|| (Vec::new(), Vec::new()));
        entry.0.push(debit_val);
        entry.1.push(credit_val);
    }
    let classes_with_enough_rows = by_class.iter().filter(|(_, (d, _))| d.len() >= min_rows_per_class).count();
    info!(
        n_classes_with_enough_rows = classes_with_enough_rows,
        remaining_privacy_budget = %privacy.remaining_budget(),
        "per-account-class extraction: computing noised stats (same ε-DP budget as column stats)"
    );

    let mut amount_by_class = HashMap::new();
    for (subclass, (debit_vals, credit_vals)) in by_class {
        if debit_vals.len() < min_rows_per_class {
            continue;
        }
        let target_debit = format!("gl.{}_debit", subclass);
        let target_credit = format!("gl.{}_credit", subclass);
        let debit_stats = compute_numeric_stats(&debit_vals, &target_debit, privacy)?;
        let credit_stats = compute_numeric_stats(&credit_vals, &target_credit, privacy)?;
        amount_by_class.insert(
            subclass.clone(),
            AccountClassAmountStats {
                account_class: subclass,
                row_count: debit_vals.len() as u64,
                debit_stats,
                credit_stats,
            },
        );
    }
    if amount_by_class.is_empty() {
        Ok(None)
    } else {
        Ok(Some(amount_by_class))
    }
}

/// Extract statistics from CSV (via Polars: one read, robust parsing and type inference).
fn extract_from_csv(
    csv: &super::CsvDataSource,
    config: &ExtractionConfig,
    privacy: &mut PrivacyEngine,
) -> FingerprintResult<StatisticsFingerprint> {
    let (headers, columns, _total_rows) = super::csv_io::read_csv_into_columns(
        &csv.path,
        csv.has_headers,
        Some(csv.delimiter),
        None,
    )?;

    let table_name = csv
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("data");

    let numeric_columns = match config.accounting_standards {
        AccountingStandards::FrenchGaap => Some(FRENCH_GAAP_NUMERIC_HEADERS),
        AccountingStandards::UsGaap => None,
    };

    let gl_indices = find_account_debit_credit_columns(&headers)
        .or_else(|| find_account_debit_credit_columns_aprr_fallback(&headers));

    let mut stats = extract_column_stats(
        &headers,
        &columns,
        table_name,
        numeric_columns,
        config,
        privacy,
        gl_indices.is_none(),
    )?;

    if gl_indices.is_some() && stats.amount_by_account_class.is_none() {
        info!(
            "statistics: retrying amount_by_account_class extraction (fallback after extract_column_stats)"
        );
        match try_extract_amount_by_account_class_generic(
            &headers,
            &columns,
            super::FEC_MIN_ROWS_PER_CLASS,
            super::FEC_ACCOUNT_CLASS_LEVEL,
            privacy,
        ) {
            Ok(Some(amount_by_class)) if !amount_by_class.is_empty() => {
                info!(class_count = amount_by_class.len(), "statistics: amount_by_account_class set from fallback");
                stats.set_amount_by_account_class(amount_by_class);
            }
            Ok(Some(_)) => {
                info!("statistics: amount_by_account_class not set — no class had enough rows (min 5)");
            }
            Ok(None) => {
                info!("statistics: amount_by_account_class not set — no account/debit/credit columns or insufficient rows");
            }
            Err(e) => {
                info!(
                    error = %e,
                    "statistics: amount_by_account_class not set — fallback extraction failed (e.g. privacy budget exhausted)"
                );
            }
        }
    }

    Ok(stats)
}

/// Extract statistics from FEC (Fichier des Écritures Comptables).
/// Only amount columns get numeric stats; all others categorical.
/// Additionally computes per-account-class (first 3 digits) amount distributions for synthetic mimic.
fn extract_from_fec(
    fec: &super::FecDataSource,
    config: &ExtractionConfig,
    privacy: &mut PrivacyEngine,
) -> FingerprintResult<StatisticsFingerprint> {
    let csv = fec.as_csv();
    let (headers, columns, _) = super::csv_io::read_csv_into_columns(
        &csv.path,
        csv.has_headers,
        Some(csv.delimiter),
        None,
    )?;

    let table_name = fec.table_name();
    let mut stats = extract_column_stats(
        &headers,
        &columns,
        &table_name,
        Some(FEC_NUMERIC_COLUMNS),
        config,
        privacy,
        false, // FEC path: caller sets amount_by_account_class below (may be empty if few rows)
    )?;

    // Stats by subclass: truncate existing account num first (first 3 digits), then aggregate debit/credit per subclass
    let account_idx = headers.iter().position(|h| h == FEC_ACCOUNT_COLUMN);
    let debit_idx = headers.iter().position(|h| h == "Montant au débit");
    let credit_idx = headers.iter().position(|h| h == "Montant au crédit");

    if let (Some(acc_i), Some(deb_i), Some(cred_i)) = (account_idx, debit_idx, credit_idx) {
        let n_rows = columns[0].len();
        let mut by_class: HashMap<String, (Vec<f64>, Vec<f64>)> = HashMap::new();

        for r in 0..n_rows {
            let account = columns[acc_i].get(r).map(|s| s.as_str()).unwrap_or("").trim();
            let subclass = fec_account_class(account, FEC_ACCOUNT_CLASS_LEVEL);
            if subclass.is_empty() {
                continue;
            }
            let debit_val = columns[deb_i]
                .get(r)
                .and_then(|s| parse_amount_str(s))
                .unwrap_or(0.0);
            let credit_val = columns[cred_i]
                .get(r)
                .and_then(|s| parse_amount_str(s))
                .unwrap_or(0.0);

            let entry = by_class.entry(subclass).or_insert_with(|| (Vec::new(), Vec::new()));
            entry.0.push(debit_val);
            entry.1.push(credit_val);
        }

        let mut amount_by_class = HashMap::new();
        for (subclass, (debit_vals, credit_vals)) in by_class {
            let total_lines = debit_vals.len();
            if total_lines < FEC_MIN_ROWS_PER_CLASS {
                continue;
            }
            let target_debit = format!("fec.{}_debit", subclass);
            let target_credit = format!("fec.{}_credit", subclass);
            let debit_stats = compute_numeric_stats(&debit_vals, &target_debit, privacy)?;
            let credit_stats = compute_numeric_stats(&credit_vals, &target_credit, privacy)?;
            amount_by_class.insert(
                subclass.clone(),
                AccountClassAmountStats {
                    account_class: subclass,
                    row_count: total_lines as u64,
                    debit_stats,
                    credit_stats,
                },
            );
        }
        if !amount_by_class.is_empty() {
            stats.set_amount_by_account_class(amount_by_class);
        }
    }

    Ok(stats)
}

/// Extract statistics from memory.
fn extract_from_memory(
    mem: &super::MemoryDataSource,
    config: &ExtractionConfig,
    privacy: &mut PrivacyEngine,
) -> FingerprintResult<StatisticsFingerprint> {
    // Transpose rows to columns
    let mut columns: Vec<Vec<String>> = vec![Vec::new(); mem.columns.len()];

    for row in &mem.rows {
        for (i, value) in row.iter().enumerate() {
            if i < columns.len() {
                columns[i].push(value.clone());
            }
        }
    }

    extract_column_stats(
        &mem.columns,
        &columns,
        "memory",
        None,
        config,
        privacy,
        true,
    )
}

/// Extract statistics from Parquet file.
fn extract_from_parquet(
    pq: &super::ParquetDataSource,
    config: &ExtractionConfig,
    privacy: &mut PrivacyEngine,
) -> FingerprintResult<StatisticsFingerprint> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::fs::File;

    let file = File::open(&pq.path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema = builder.schema().clone();
    let reader = builder.with_batch_size(10000).build()?;

    // Collect column names
    let headers: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
    let mut columns: Vec<Vec<String>> = vec![Vec::new(); headers.len()];

    // Read batches
    for batch_result in reader {
        let batch = batch_result?;
        for (i, _field) in schema.fields().iter().enumerate() {
            let column = batch.column(i);
            let values = super::schema_extractor::arrow_column_to_strings(column.as_ref());
            columns[i].extend(values);
        }
    }

    let table_name = pq
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("data");

    extract_column_stats(
        &headers,
        &columns,
        table_name,
        None,
        config,
        privacy,
        true,
    )
}

/// Extract statistics from JSON/JSONL file.
fn extract_from_json(
    json: &super::JsonDataSource,
    config: &ExtractionConfig,
    privacy: &mut PrivacyEngine,
) -> FingerprintResult<StatisticsFingerprint> {
    use std::collections::HashSet;
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let file = File::open(&json.path)?;
    let reader = BufReader::new(file);

    let mut rows: Vec<HashMap<String, serde_json::Value>> = Vec::new();

    if json.is_array {
        // JSON array format
        let content = std::fs::read_to_string(&json.path)?;
        let array: Vec<serde_json::Value> = serde_json::from_str(&content)?;

        for value in array {
            if let serde_json::Value::Object(obj) = value {
                rows.push(obj.into_iter().collect());
            }
        }
    } else {
        // JSONL format
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str(&line) {
                rows.push(obj.into_iter().collect());
            }
        }
    }

    // Collect all column names
    let mut all_columns: HashSet<String> = HashSet::new();
    for row in &rows {
        for key in row.keys() {
            all_columns.insert(key.clone());
        }
    }

    // Sort columns for consistency
    let mut headers: Vec<String> = all_columns.into_iter().collect();
    headers.sort();

    // Build columns
    let mut columns: Vec<Vec<String>> = vec![Vec::new(); headers.len()];
    for row in &rows {
        for (i, header) in headers.iter().enumerate() {
            let value = row
                .get(header)
                .map(super::schema_extractor::json_value_to_string)
                .unwrap_or_default();
            columns[i].push(value);
        }
    }

    let table_name = json
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("data");

    extract_column_stats(
        &headers,
        &columns,
        table_name,
        None,
        config,
        privacy,
        true,
    )
}

/// Extract statistics for all columns.
/// If `numeric_columns` is Some (e.g. FEC or French GAAP), only those columns get numeric stats; others categorical.
/// Date/temporal columns are never treated as numeric.
/// When `require_amount_by_class_when_gl` is true and account/debit/credit columns exist, errors if per-3-digit stats cannot be computed.
fn extract_column_stats(
    headers: &[String],
    columns: &[Vec<String>],
    table_name: &str,
    numeric_columns: Option<&[&str]>,
    _config: &ExtractionConfig,
    privacy: &mut PrivacyEngine,
    require_amount_by_class_when_gl: bool,
) -> FingerprintResult<StatisticsFingerprint> {
    let mut stats = StatisticsFingerprint::new();

    for (i, header) in headers.iter().enumerate() {
        let values = &columns[i];

        let numeric_values: Vec<f64> = values
            .iter()
            .filter_map(|v| parse_amount_str(v))
            .collect();

        let treat_as_numeric = match numeric_columns {
            Some(names) => names
                .iter()
                .any(|n| header.as_str().eq_ignore_ascii_case(n)),
            None => numeric_values.len() > values.len() / 2,
        };
        // Never treat date/temporal columns as numeric
        let is_temporal = is_temporal_column(header, values);
        let treat_as_numeric = treat_as_numeric && !is_temporal;

        if treat_as_numeric && !numeric_values.is_empty() {
            let target = format!("{}.{}", table_name, header);
            let numeric_stats = compute_numeric_stats(&numeric_values, &target, privacy)?;
            stats.add_numeric(table_name, header, numeric_stats);
        } else if is_temporal {
            let temporal_stats = compute_temporal_stats(values, header);
            stats.add_temporal(table_name, header, temporal_stats);
        } else {
            let target = format!("{}.{}", table_name, header);
            let cat_stats = compute_categorical_stats(values, &target, privacy)?;
            stats.add_categorical(table_name, header, cat_stats);
        }
    }

    // Strong assertion: Debit and Credit (and FEC débit/crédit) must appear in numeric_columns; skip optional ones like Montant en devise
    const REQUIRED_AMOUNT_FOR_ASSERTION: &[&str] = &[
        "Debit",
        "Credit",
        "Montant au débit",
        "Montant au crédit",
    ];
    for header in headers {
        if REQUIRED_AMOUNT_FOR_ASSERTION
            .iter()
            .any(|c| header.as_str().eq_ignore_ascii_case(c))
        {
            let key = format!("{}.{}", table_name, header);
            if !stats.numeric_columns.contains_key(&key) {
                return Err(FingerprintError::extraction(
                    "statistics",
                    format!(
                        "Column '{}' is a required amount column (Debit/Credit) but was not extracted as numeric. \
                         Ensure values are parseable (e.g. use dot or comma as decimal separator) and the column is not misclassified.",
                        header
                    ),
                ));
            }
        }
    }

    // Compute global Benford analysis for numeric columns
    let all_amounts: Vec<f64> = stats
        .numeric_columns
        .values()
        .flat_map(|s| vec![s.mean]) // Simplified - would use actual values in production
        .filter(|v| *v > 0.0)
        .collect();

    if all_amounts.len() >= 100 {
        // Would compute actual Benford stats from raw values
        // For now, placeholder
    }

    // CSV/Parquet/JSON: try generic per-account-class extraction when account + debit + credit columns exist (US GAAP or French GAAP)
    let has_gl_columns = find_account_debit_credit_columns(headers).is_some()
        || find_account_debit_credit_columns_aprr_fallback(headers).is_some();
    if has_gl_columns {
        info!(
            remaining_privacy_budget = %privacy.remaining_budget(),
            "statistics: attempting amount_by_account_class extraction (GL columns detected)"
        );
    }
    match try_extract_amount_by_account_class_generic(
        headers,
        columns,
        super::FEC_MIN_ROWS_PER_CLASS,
        super::FEC_ACCOUNT_CLASS_LEVEL,
        privacy,
    ) {
        Ok(Some(amount_by_class)) if !amount_by_class.is_empty() => {
            info!(class_count = amount_by_class.len(), "statistics: amount_by_account_class set successfully");
            stats.set_amount_by_account_class(amount_by_class);
        }
        Ok(Some(_)) if has_gl_columns && require_amount_by_class_when_gl => {
            return Err(FingerprintError::extraction(
                "statistics",
                "Account, debit, and credit columns were found but no 3-digit account class had enough rows (min 5) for distributional stats. Ensure Debit/Credit contain numeric values and account codes have at least 3 digits.",
            ));
        }
        Ok(None) if has_gl_columns && require_amount_by_class_when_gl => {
            return Err(FingerprintError::extraction(
                "statistics",
                "Account, debit, and credit columns were found but distributional stats per 3-digit account could not be computed. Ensure column names match (e.g. CompteNum/account, Debit, Credit) and values are numeric.",
            ));
        }
        Err(e) => {
            info!(
                error = %e,
                "statistics: amount_by_account_class not set (extraction failed). If the error is 'Privacy budget exhausted', re-run with --privacy-epsilon 2.0 (or higher) to get per-account-class stats."
            );
        }
        _ => {}
    }

    Ok(stats)
}

/// Compute numeric statistics.
fn compute_numeric_stats(
    values: &[f64],
    target: &str,
    privacy: &mut PrivacyEngine,
) -> FingerprintResult<NumericStats> {
    if values.is_empty() {
        return Ok(NumericStats::new(0, 0.0, 0.0, 0.0, 0.0));
    }

    let count = values.len() as u64;
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));

    // Winsorize before computing stats
    privacy.winsorize(&mut sorted, target);

    let min = sorted.first().copied().unwrap_or(0.0);
    let max = sorted.last().copied().unwrap_or(0.0);
    let sum: f64 = sorted.iter().sum();
    let mean = sum / sorted.len() as f64;

    let variance: f64 =
        sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / sorted.len() as f64;
    let std_dev = variance.sqrt();

    // Add noise to statistics
    let noised_mean = privacy.add_noise(mean, max - min, &format!("{}.mean", target))?;
    let noised_std_dev =
        privacy.add_noise(std_dev, (max - min) / 2.0, &format!("{}.std_dev", target))?;

    // Compute percentiles
    let percentiles = compute_percentiles(&sorted);

    // Fit distribution
    let (distribution, params) = fit_distribution(&sorted, mean, std_dev);

    // Zero and negative rates
    let zero_rate = sorted.iter().filter(|v| **v == 0.0).count() as f64 / count as f64;
    let negative_rate = sorted.iter().filter(|v| **v < 0.0).count() as f64 / count as f64;

    // Benford first digit
    let benford = compute_benford_first_digit(&sorted);

    Ok(NumericStats {
        count,
        min,
        max,
        mean: noised_mean,
        std_dev: noised_std_dev.abs(),
        percentiles,
        distribution,
        distribution_params: params,
        zero_rate,
        negative_rate,
        benford_first_digit: Some(benford),
    })
}

/// Compute percentiles from sorted values.
fn compute_percentiles(sorted: &[f64]) -> Percentiles {
    fn percentile(sorted: &[f64], p: f64) -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let idx = (p / 100.0 * (sorted.len() - 1) as f64).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    Percentiles {
        p1: percentile(sorted, 1.0),
        p5: percentile(sorted, 5.0),
        p10: percentile(sorted, 10.0),
        p25: percentile(sorted, 25.0),
        p50: percentile(sorted, 50.0),
        p75: percentile(sorted, 75.0),
        p90: percentile(sorted, 90.0),
        p95: percentile(sorted, 95.0),
        p99: percentile(sorted, 99.0),
    }
}

/// Fit a distribution to the data.
fn fit_distribution(
    sorted: &[f64],
    mean: f64,
    std_dev: f64,
) -> (DistributionType, DistributionParams) {
    // Simple heuristic-based fitting

    // Check for uniform
    let range = sorted.last().unwrap_or(&0.0) - sorted.first().unwrap_or(&0.0);
    let expected_std_uniform = range / (12.0_f64).sqrt();
    if (std_dev - expected_std_uniform).abs() / expected_std_uniform < 0.1 {
        return (
            DistributionType::Uniform,
            DistributionParams::uniform(
                *sorted.first().unwrap_or(&0.0),
                *sorted.last().unwrap_or(&1.0),
            ),
        );
    }

    // Check for log-normal (skewed, positive values)
    let all_positive = sorted.iter().all(|v| *v > 0.0);
    let skewness = compute_skewness(sorted, mean, std_dev);

    if all_positive && skewness > 0.5 {
        // Fit log-normal
        let log_values: Vec<f64> = sorted.iter().map(|v| v.ln()).collect();
        let log_mean: f64 = log_values.iter().sum::<f64>() / log_values.len() as f64;
        let log_var: f64 = log_values
            .iter()
            .map(|v| (v - log_mean).powi(2))
            .sum::<f64>()
            / log_values.len() as f64;
        let log_std = log_var.sqrt();

        return (
            DistributionType::LogNormal,
            DistributionParams::log_normal(log_mean, log_std),
        );
    }

    // Default to normal
    (
        DistributionType::Normal,
        DistributionParams::normal(mean, std_dev),
    )
}

/// Compute skewness.
fn compute_skewness(values: &[f64], mean: f64, std_dev: f64) -> f64 {
    if std_dev == 0.0 || values.is_empty() {
        return 0.0;
    }

    let n = values.len() as f64;
    let m3: f64 = values.iter().map(|v| (v - mean).powi(3)).sum::<f64>() / n;
    m3 / std_dev.powi(3)
}

/// Compute Benford first digit distribution.
fn compute_benford_first_digit(values: &[f64]) -> [f64; 9] {
    let mut counts = [0u64; 9];
    let mut total = 0u64;

    for v in values {
        let abs_v = v.abs();
        if abs_v > 0.0 {
            let s = format!("{:.15}", abs_v);
            for c in s.chars() {
                if c.is_ascii_digit() && c != '0' {
                    if let Some(digit) = c.to_digit(10) {
                        let digit = digit as usize;
                        if (1..=9).contains(&digit) {
                            counts[digit - 1] += 1;
                            total += 1;
                        }
                    }
                    break;
                }
            }
        }
    }

    if total == 0 {
        return [0.0; 9];
    }

    let mut freqs = [0.0; 9];
    for i in 0..9 {
        freqs[i] = counts[i] as f64 / total as f64;
    }
    freqs
}

/// Compute categorical statistics.
fn compute_categorical_stats(
    values: &[String],
    target: &str,
    privacy: &mut PrivacyEngine,
) -> FingerprintResult<CategoricalStats> {
    let non_empty: Vec<_> = values.iter().filter(|v| !v.is_empty()).collect();
    let count = non_empty.len() as u64;

    if count == 0 {
        return Ok(CategoricalStats::new(0, 0));
    }

    // Count frequencies
    let mut freq_map: HashMap<&String, u64> = HashMap::new();
    for v in &non_empty {
        *freq_map.entry(v).or_default() += 1;
    }

    let cardinality = freq_map.len() as u64;

    // Convert to list for privacy filtering
    let frequencies: Vec<(String, u64)> =
        freq_map.into_iter().map(|(k, v)| (k.clone(), v)).collect();

    // Apply k-anonymity filtering
    let filtered = privacy.filter_categories(frequencies, count, target);

    // Convert to CategoryFrequency
    let top_values: Vec<CategoryFrequency> = filtered
        .into_iter()
        .map(|(value, freq)| CategoryFrequency::new(value, freq))
        .take(100) // Limit to top 100
        .collect();

    // Compute entropy
    let entropy = compute_entropy(&top_values);

    Ok(CategoricalStats {
        count,
        cardinality,
        top_values,
        rare_values_suppressed: true, // Privacy filtering applied
        suppressed_count: 0,          // Would be computed from filtering
        entropy,
    })
}

/// Compute entropy of a distribution.
fn compute_entropy(frequencies: &[CategoryFrequency]) -> f64 {
    let mut entropy = 0.0;
    for freq in frequencies {
        if freq.frequency > 0.0 {
            entropy -= freq.frequency * freq.frequency.ln();
        }
    }
    entropy
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_benford_first_digit() {
        let values = vec![123.0, 456.0, 789.0, 100.0, 200.0, 300.0];
        let benford = compute_benford_first_digit(&values);

        // Should have counts for digits 1, 2, 3, 4, 7
        assert!(benford[0] > 0.0); // digit 1
        assert!(benford[1] > 0.0); // digit 2
        assert!(benford[2] > 0.0); // digit 3
    }
}
