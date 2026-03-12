//! CSV reading via Polars: robust parsing, type inference, and encoding.

use std::path::Path;

use polars::prelude::*;

use crate::error::{FingerprintError, FingerprintResult};

/// Read a CSV file into a Polars DataFrame.
/// Uses Polars for robust delimiter handling, schema inference, and encoding.
/// `delimiter`: None = Polars default (comma), Some(b';') for semicolon, etc.
#[allow(dead_code)]
pub fn read_csv_polars(
    path: &Path,
    has_header: bool,
    delimiter: Option<u8>,
) -> FingerprintResult<DataFrame> {
    let path_buf = path.to_path_buf();
    let opts = CsvReadOptions::default()
        .with_has_header(has_header)
        .with_infer_schema_length(Some(10000))y
        .map_parse_options(|p| {
            if let Some(sep) = delimiter {
                CsvParseOptions::default().with_separator(sep)
            } else {
                p
            }
        });

    let df = opts
        .try_into_reader_with_file_path(Some(path_buf))
        .map_err(|e| FingerprintError::Polars(e.to_string()))?
        .finish()
        .map_err(|e| FingerprintError::Polars(e.to_string()))?;

    Ok(df)
}

/// Convert a Polars DataFrame to (column names, Vec<Vec<String>>) for existing pipelines.
pub fn dataframe_to_columns(df: &DataFrame) -> (Vec<String>, Vec<Vec<String>>) {
    let headers: Vec<String> = df.get_column_names().iter().map(|s| s.to_string()).collect();
    let mut columns: Vec<Vec<String>> = Vec::with_capacity(headers.len());

    for col in df.columns() {
        let col_str: Vec<String> = column_to_string_vec(col);
        columns.push(col_str);
    }

    (headers, columns)
}

fn column_to_string_vec(col: &Column) -> Vec<String> {
    if let Some(s) = col.as_series() {
        series_to_string_vec(s)
    } else {
        Vec::new()
    }
}

fn series_to_string_vec(s: &Series) -> Vec<String> {
    if let Ok(ca) = s.str() {
        return ca.iter().map(|o| o.unwrap_or("").to_string()).collect();
    }
    if let Ok(casted) = s.cast(&DataType::String) {
        if let Ok(ca) = casted.str() {
            return ca.iter().map(|o| o.unwrap_or("").to_string()).collect();
        }
    }
    Vec::new()
}

/// Read CSV with Polars and return (headers, columns) and total row count.
pub fn read_csv_into_columns(
    path: &Path,
    has_header: bool,
    delimiter: Option<u8>,
    max_rows: Option<usize>,
) -> FingerprintResult<(Vec<String>, Vec<Vec<String>>, u64)> {
    let opts = CsvReadOptions::default()
        .with_has_header(has_header)
        .with_infer_schema_length(Some(10000))
        .map_parse_options(|p| {
            if let Some(sep) = delimiter {
                CsvParseOptions::default().with_separator(sep)
            } else {
                p
            }
        })
        .with_n_rows(max_rows);

    let path_buf = path.to_path_buf();
    let df = opts
        .try_into_reader_with_file_path(Some(path_buf))
        .map_err(|e| FingerprintError::Polars(e.to_string()))?
        .finish()
        .map_err(|e| FingerprintError::Polars(e.to_string()))?;

    let total_rows = df.height() as u64;
    let (headers, columns) = dataframe_to_columns(&df);
    Ok((headers, columns, total_rows))
}
