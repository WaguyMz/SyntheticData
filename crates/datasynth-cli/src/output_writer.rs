//! Comprehensive output writer for all generated data.
//!
//! Writes all generated data from the EnhancedGenerationResult to files
//! in the output directory. Uses CSV for flat tabular data (journal entry
//! lines) and JSON for types with nested structures (Vecs, sub-structs).

use std::io::Write;
use std::path::Path;
use std::collections::HashSet;

use datasynth_config::schema::{
    ForensicLlmOutputConfig, GnnSplitStrategy, GnnTrainingOutputConfig, OutputConfig,
};
use datasynth_runtime::enhanced_orchestrator::EnhancedGenerationResult;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use tracing::{info, warn};

/// Write a JSON file for any serializable slice. Skips empty slices.
///
/// Streams JSON directly to a buffered file writer instead of allocating
/// the entire JSON string in memory (Phase 3 I/O optimization).
fn write_json<T: serde::Serialize>(
    data: &[T],
    path: &Path,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if data.is_empty() {
        return Ok(());
    }
    let file = std::fs::File::create(path)?;
    let writer = std::io::BufWriter::with_capacity(256 * 1024, file);
    serde_json::to_writer_pretty(writer, data)?;
    info!(
        "  {} written: {} records -> {}",
        label,
        data.len(),
        path.display()
    );
    Ok(())
}

/// Write journal entry lines as a flat CSV file.
///
/// This extracts the key fields from both the header and each line item to
/// produce a single flat CSV that can be loaded directly into dataframes.
fn write_journal_entries_csv(
    result: &EnhancedGenerationResult,
    output_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.journal_entries.is_empty() {
        return Ok(());
    }

    let path = output_dir.join("journal_entries.csv");
    let file = std::fs::File::create(&path)?;
    let mut w = std::io::BufWriter::with_capacity(256 * 1024, file);

    // Write header
    writeln!(
        w,
        "document_id,company_code,fiscal_year,fiscal_period,posting_date,document_date,\
         document_type,currency,exchange_rate,reference,header_text,created_by,source,\
         business_process,ledger,is_fraud,is_anomaly,\
         line_number,gl_account,debit_amount,credit_amount,local_amount,\
         cost_center,profit_center,line_text,\
         auxiliary_account_number,auxiliary_account_label,lettrage,lettrage_date"
    )?;

    for je in &result.journal_entries {
        let h = &je.header;
        for line in &je.lines {
            let lettrage_date_str = line
                .lettrage_date
                .map(|d| d.to_string())
                .unwrap_or_default();
            writeln!(
                w,
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                h.document_id,
                csv_escape(&h.company_code),
                h.fiscal_year,
                h.fiscal_period,
                h.posting_date,
                h.document_date,
                csv_escape(&h.document_type),
                csv_escape(&h.currency),
                h.exchange_rate,
                csv_opt_str(&h.reference),
                csv_opt_str(&h.header_text),
                csv_escape(&h.created_by),
                h.source,
                h.business_process
                    .map(|bp| format!("{bp:?}"))
                    .unwrap_or_default(),
                csv_escape(&h.ledger),
                h.is_fraud,
                h.is_anomaly,
                line.line_number,
                csv_escape(&line.gl_account),
                line.debit_amount,
                line.credit_amount,
                line.local_amount,
                csv_opt_str(&line.cost_center),
                csv_opt_str(&line.profit_center),
                csv_opt_str(&line.line_text),
                csv_opt_str(&line.auxiliary_account_number),
                csv_opt_str(&line.auxiliary_account_label),
                csv_opt_str(&line.lettrage),
                lettrage_date_str,
            )?;
        }
    }

    w.flush()?;
    let total_lines: usize = result.journal_entries.iter().map(|je| je.lines.len()).sum();
    info!(
        "  Journal entries CSV written: {} entries, {} line items -> {}",
        result.journal_entries.len(),
        total_lines,
        path.display()
    );
    Ok(())
}

/// Escape a string for CSV output by quoting if it contains commas or quotes.
fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Format an Option<String> for CSV output (empty string for None).
fn csv_opt_str(opt: &Option<String>) -> String {
    match opt {
        Some(s) => csv_escape(s),
        None => String::new(),
    }
}

/// Write forensic LLM-oriented views (header/line tables) without any train/val/test split.
fn write_forensic_llm_output(
    result: &EnhancedGenerationResult,
    output_dir: &Path,
    cfg: &ForensicLlmOutputConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    if !cfg.enabled || result.journal_entries.is_empty() {
        return Ok(());
    }

    let dir = output_dir.join(&cfg.subdirectory);
    std::fs::create_dir_all(&dir)?;
    info!("Writing forensic LLM output to: {}", dir.display());

    // Header-level table: one row per journal entry.
    let header_path = dir.join("je_header.csv");
    let header_file = std::fs::File::create(&header_path)?;
    let mut hw = std::io::BufWriter::with_capacity(256 * 1024, header_file);

    writeln!(
        hw,
        "document_id,company_code,fiscal_year,fiscal_period,posting_date,document_date,\
         document_type,currency,exchange_rate,reference,header_text,created_by,source,\
         business_process,ledger,is_fraud,is_anomaly"
    )?;

    for je in &result.journal_entries {
        let h = &je.header;
        writeln!(
            hw,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            h.document_id,
            csv_escape(&h.company_code),
            h.fiscal_year,
            h.fiscal_period,
            h.posting_date,
            h.document_date,
            csv_escape(&h.document_type),
            csv_escape(&h.currency),
            h.exchange_rate,
            csv_opt_str(&h.reference),
            csv_opt_str(&h.header_text),
            csv_escape(&h.created_by),
            h.source,
            h.business_process
                .map(|bp| format!("{bp:?}"))
                .unwrap_or_default(),
            csv_escape(&h.ledger),
            h.is_fraud,
            h.is_anomaly,
        )?;
    }
    hw.flush()?;

    // Line-level table: one row per journal entry line.
    let line_path = dir.join("je_line.csv");
    let line_file = std::fs::File::create(&line_path)?;
    let mut lw = std::io::BufWriter::with_capacity(256 * 1024, line_file);

    writeln!(
        lw,
        "document_id,line_number,company_code,gl_account,debit_amount,credit_amount,local_amount,\
         cost_center,profit_center,line_text,auxiliary_account_number,auxiliary_account_label,\
         lettrage,lettrage_date,is_fraud,is_anomaly"
    )?;

    for je in &result.journal_entries {
        let h = &je.header;
        for line in &je.lines {
            let lettrage_date_str = line
                .lettrage_date
                .map(|d| d.to_string())
                .unwrap_or_default();
            writeln!(
                lw,
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                h.document_id,
                line.line_number,
                csv_escape(&h.company_code),
                csv_escape(&line.gl_account),
                line.debit_amount,
                line.credit_amount,
                line.local_amount,
                csv_opt_str(&line.cost_center),
                csv_opt_str(&line.profit_center),
                csv_opt_str(&line.line_text),
                csv_opt_str(&line.auxiliary_account_number),
                csv_opt_str(&line.auxiliary_account_label),
                csv_opt_str(&line.lettrage),
                lettrage_date_str,
                h.is_fraud,
                h.is_anomaly,
            )?;
        }
    }
    lw.flush()?;

    // Master data and labels for SQL-style provisioning.
    write_forensic_employees_csv(result, &dir)?;
    write_forensic_vendors_csv(result, &dir)?;
    write_forensic_customers_csv(result, &dir)?;
    write_forensic_anomaly_labels_csv(result, &dir)?;
    write_forensic_sql_provisioning(result, &dir)?;

    Ok(())
}

fn write_forensic_employees_csv(
    result: &EnhancedGenerationResult,
    dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.master_data.employees.is_empty() {
        return Ok(());
    }
    let path = dir.join("employees.csv");
    let file = std::fs::File::create(&path)?;
    let mut w = std::io::BufWriter::with_capacity(256 * 1024, file);

    writeln!(
        w,
        "employee_id,user_id,display_name,first_name,last_name,email,company_code,\
         department_id,cost_center,manager_id,status,hire_date,termination_date,creation_date,location,\
         payroll_bank_name,payroll_bank_country,payroll_account_number,payroll_routing_code,\
         is_fraud_actor"
    )?;

    for e in &result.master_data.employees {
        let hire_date = e.hire_date.map(|d| d.to_string()).unwrap_or_default();
        let term_date = e
            .termination_date
            .map(|d| d.to_string())
            .unwrap_or_default();
        let creation_date = e
            .creation_date
            .map(|d| d.to_string())
            .unwrap_or_default();
        let (bank_name, bank_country, account_number, routing_code) = match &e.bank_account {
            Some(acc) => (
                csv_escape(&acc.bank_name),
                csv_escape(&acc.bank_country),
                csv_escape(&acc.account_number),
                csv_escape(&acc.routing_code),
            ),
            None => (String::new(), String::new(), String::new(), String::new()),
        };

        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            csv_escape(&e.employee_id),
            csv_escape(&e.user_id),
            csv_escape(&e.display_name),
            csv_escape(&e.first_name),
            csv_escape(&e.last_name),
            csv_escape(&e.email),
            csv_escape(&e.company_code),
            csv_opt_str(&e.department_id),
            csv_opt_str(&e.cost_center),
            csv_opt_str(&e.manager_id),
            format!("{:?}", e.status),
            hire_date,
            term_date,
            creation_date,
            csv_opt_str(&e.location),
            bank_name,
            bank_country,
            account_number,
            routing_code,
            e.is_fraud_actor,
        )?;
    }
    w.flush()?;
    Ok(())
}

fn write_forensic_vendors_csv(
    result: &EnhancedGenerationResult,
    dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.master_data.vendors.is_empty() {
        return Ok(());
    }
    let path = dir.join("vendors.csv");
    let file = std::fs::File::create(&path)?;
    let mut w = std::io::BufWriter::with_capacity(256 * 1024, file);

    writeln!(
        w,
        "vendor_id,name,country,account_number,tax_id,currency,reconciliation_account,\
         auxiliary_gl_account,is_intercompany,behavior,payment_terms,is_fraud_actor,\
         bank_account_count,primary_bank_name,primary_bank_country,primary_account_number,primary_routing_code"
    )?;

    for v in &result.master_data.vendors {
        let (bank_count, bank_name, bank_country, account_number, routing_code) =
            if v.bank_accounts.is_empty() {
                (0usize, String::new(), String::new(), String::new(), String::new())
            } else {
                let primary = v
                    .bank_accounts
                    .iter()
                    .find(|b| b.is_primary)
                    .unwrap_or(&v.bank_accounts[0]);
                (
                    v.bank_accounts.len(),
                    csv_escape(&primary.bank_name),
                    csv_escape(&primary.bank_country),
                    csv_escape(&primary.account_number),
                    csv_escape(&primary.routing_code),
                )
            };

        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            csv_escape(&v.vendor_id),
            csv_escape(&v.name),
            csv_escape(&v.country),
            csv_opt_str(&v.account_number),
            csv_opt_str(&v.tax_id),
            csv_escape(&v.currency),
            csv_opt_str(&v.reconciliation_account),
            csv_opt_str(&v.auxiliary_gl_account),
            v.is_intercompany,
            format!("{:?}", v.behavior),
            v.payment_terms.code(),
            v.is_fraud_actor,
            bank_count,
            bank_name,
            bank_country,
            account_number,
            routing_code,
        )?;
    }
    w.flush()?;
    Ok(())
}

fn write_forensic_customers_csv(
    result: &EnhancedGenerationResult,
    dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.master_data.customers.is_empty() {
        return Ok(());
    }
    let path = dir.join("customers.csv");
    let file = std::fs::File::create(&path)?;
    let mut w = std::io::BufWriter::with_capacity(256 * 1024, file);

    writeln!(
        w,
        "customer_id,name,country,account_number,tax_id,currency,reconciliation_account,\
         auxiliary_gl_account,is_intercompany,credit_rating,is_fraud_actor,\
         bank_account_count,primary_bank_name,primary_bank_country,primary_account_number,primary_routing_code"
    )?;

    for c in &result.master_data.customers {
        let (bank_count, bank_name, bank_country, account_number, routing_code) =
            if c.bank_accounts.is_empty() {
                (0usize, String::new(), String::new(), String::new(), String::new())
            } else {
                let primary = c
                    .bank_accounts
                    .iter()
                    .find(|b| b.is_primary)
                    .unwrap_or(&c.bank_accounts[0]);
                (
                    c.bank_accounts.len(),
                    csv_escape(&primary.bank_name),
                    csv_escape(&primary.bank_country),
                    csv_escape(&primary.account_number),
                    csv_escape(&primary.routing_code),
                )
            };

        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            csv_escape(&c.customer_id),
            csv_escape(&c.name),
            csv_escape(&c.country),
            csv_opt_str(&c.account_number),
            csv_opt_str(&c.tax_id),
            csv_escape(&c.currency),
            csv_opt_str(&c.reconciliation_account),
            csv_opt_str(&c.auxiliary_gl_account),
            c.is_intercompany,
            format!("{:?}", c.credit_rating),
            c.is_fraud_actor,
            bank_count,
            bank_name,
            bank_country,
            account_number,
            routing_code,
        )?;
    }
    w.flush()?;
    Ok(())
}

fn write_forensic_anomaly_labels_csv(
    result: &EnhancedGenerationResult,
    dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if result.anomaly_labels.labels.is_empty() {
        return Ok(());
    }
    let path = dir.join("anomaly_labels.csv");
    let file = std::fs::File::create(&path)?;
    let mut w = std::io::BufWriter::with_capacity(256 * 1024, file);

    writeln!(
        w,
        "anomaly_id,anomaly_type,document_id,document_type,company_code,anomaly_date,\
         detection_timestamp,confidence,severity,description,monetary_impact,is_injected"
    )?;

    for lbl in &result.anomaly_labels.labels {
        let impact = lbl
            .monetary_impact
            .map(|d| d.to_string())
            .unwrap_or_default();
        writeln!(
            w,
            "{},{},{},{},{},{},{},{},{},{},{},{}",
            csv_escape(&lbl.anomaly_id),
            format!("{:?}", lbl.anomaly_type),
            csv_escape(&lbl.document_id),
            csv_escape(&lbl.document_type),
            csv_escape(&lbl.company_code),
            lbl.anomaly_date,
            lbl.detection_timestamp,
            lbl.confidence,
            lbl.severity,
            csv_escape(&lbl.description),
            impact,
            lbl.is_injected,
        )?;
    }
    w.flush()?;
    Ok(())
}

/// Write a companion SQL file with DDL + COPY commands to load the forensic
/// CSVs into a relational database (e.g., PostgreSQL, DuckDB).
fn write_forensic_sql_provisioning(
    result: &EnhancedGenerationResult,
    dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = dir.join("forensic_llm.sql");
    let file = std::fs::File::create(&path)?;
    let mut w = std::io::BufWriter::with_capacity(128 * 1024, file);

    writeln!(
        w,
        "-- Forensic LLM schema and bulk-load commands.\n\
         -- Run from the directory containing the CSVs (forensic_llm/).\n"
    )?;

    if !result.journal_entries.is_empty() {
        // Header table + staging to tolerate duplicate document_ids, then dedupe
        writeln!(
            w,
            "CREATE TABLE IF NOT EXISTS je_header (\n\
             \tdocument_id UUID PRIMARY KEY,\n\
             \tcompany_code TEXT,\n\
             \tfiscal_year INT,\n\
             \tfiscal_period INT,\n\
             \tposting_date DATE,\n\
             \tdocument_date DATE,\n\
             \tdocument_type TEXT,\n\
             \tcurrency TEXT,\n\
             \texchange_rate NUMERIC,\n\
             \treference TEXT,\n\
             \theader_text TEXT,\n\
             \tcreated_by TEXT,\n\
             \tsource TEXT,\n\
             \tbusiness_process TEXT,\n\
             \tledger TEXT,\n\
             \tis_fraud BOOLEAN,\n\
             \tis_anomaly BOOLEAN\n\
             );\n\n\
             CREATE TEMP TABLE staging_je_header (\n\
             \tdocument_id UUID,\n\
             \tcompany_code TEXT,\n\
             \tfiscal_year INT,\n\
             \tfiscal_period INT,\n\
             \tposting_date DATE,\n\
             \tdocument_date DATE,\n\
             \tdocument_type TEXT,\n\
             \tcurrency TEXT,\n\
             \texchange_rate NUMERIC,\n\
             \treference TEXT,\n\
             \theader_text TEXT,\n\
             \tcreated_by TEXT,\n\
             \tsource TEXT,\n\
             \tbusiness_process TEXT,\n\
             \tledger TEXT,\n\
             \tis_fraud BOOLEAN,\n\
             \tis_anomaly BOOLEAN\n\
             );\n\
             \\copy staging_je_header FROM 'je_header.csv' CSV HEADER;\n\
             INSERT INTO je_header\n\
             SELECT DISTINCT ON (document_id)\n\
             \tdocument_id, company_code, fiscal_year, fiscal_period, posting_date, document_date,\n\
             \tdocument_type, currency, exchange_rate, reference, header_text, created_by, source,\n\
             \tbusiness_process, ledger, is_fraud, is_anomaly\n\
             FROM staging_je_header\n\
             ORDER BY document_id;\n"
        )?;

        // Line table + staging; insert only rows whose document_id exists in je_header
        writeln!(
            w,
            "CREATE TABLE IF NOT EXISTS je_line (\n\
             \tdocument_id UUID REFERENCES je_header(document_id),\n\
             \tline_number INT,\n\
             \tcompany_code TEXT,\n\
             \tgl_account TEXT,\n\
             \tdebit_amount NUMERIC,\n\
             \tcredit_amount NUMERIC,\n\
             \tlocal_amount NUMERIC,\n\
             \tcost_center TEXT,\n\
             \tprofit_center TEXT,\n\
             \tline_text TEXT,\n\
             \tauxiliary_account_number TEXT,\n\
             \tauxiliary_account_label TEXT,\n\
             \tlettrage TEXT,\n\
             \tlettrage_date DATE,\n\
             \tis_fraud BOOLEAN,\n\
             \tis_anomaly BOOLEAN\n\
             );\n\n\
             CREATE TEMP TABLE staging_je_line (\n\
             \tdocument_id UUID,\n\
             \tline_number INT,\n\
             \tcompany_code TEXT,\n\
             \tgl_account TEXT,\n\
             \tdebit_amount NUMERIC,\n\
             \tcredit_amount NUMERIC,\n\
             \tlocal_amount NUMERIC,\n\
             \tcost_center TEXT,\n\
             \tprofit_center TEXT,\n\
             \tline_text TEXT,\n\
             \tauxiliary_account_number TEXT,\n\
             \tauxiliary_account_label TEXT,\n\
             \tlettrage TEXT,\n\
             \tlettrage_date DATE,\n\
             \tis_fraud BOOLEAN,\n\
             \tis_anomaly BOOLEAN\n\
             );\n\
             \\copy staging_je_line FROM 'je_line.csv' CSV HEADER;\n\
             INSERT INTO je_line\n\
             SELECT s.document_id, s.line_number, s.company_code, s.gl_account, s.debit_amount, s.credit_amount,\n\
             \ts.local_amount, s.cost_center, s.profit_center, s.line_text, s.auxiliary_account_number,\n\
             \ts.auxiliary_account_label, s.lettrage, s.lettrage_date, s.is_fraud, s.is_anomaly\n\
             FROM staging_je_line s\n\
             WHERE s.document_id IN (SELECT document_id FROM je_header);\n"
        )?;
    }

    if !result.master_data.employees.is_empty() {
        writeln!(
            w,
            "CREATE TABLE IF NOT EXISTS employees (\n\
             \temployee_id TEXT PRIMARY KEY,\n\
             \tuser_id TEXT,\n\
             \tdisplay_name TEXT,\n\
             \tfirst_name TEXT,\n\
             \tlast_name TEXT,\n\
             \temail TEXT,\n\
             \tcompany_code TEXT,\n\
             \tdepartment_id TEXT,\n\
             \tcost_center TEXT,\n\
             \tmanager_id TEXT,\n\
             \tstatus TEXT,\n\
             \thire_date DATE,\n\
             \ttermination_date DATE,\n\
             \tcreation_date DATE,\n\
             \tlocation TEXT,\n\
             \tpayroll_bank_name TEXT,\n\
             \tpayroll_bank_country TEXT,\n\
             \tpayroll_account_number TEXT,\n\
             \tpayroll_routing_code TEXT,\n\
             \tis_fraud_actor BOOLEAN\n\
             );\n"
        )?;
        writeln!(
            w,
            "\\copy employees FROM 'employees.csv' CSV HEADER;\n"
        )?;
    }

    if !result.master_data.vendors.is_empty() {
        writeln!(
            w,
            "CREATE TABLE IF NOT EXISTS vendors (\n\
             \tvendor_id TEXT PRIMARY KEY,\n\
             \tname TEXT,\n\
             \tcountry TEXT,\n\
             \taccount_number TEXT,\n\
             \ttax_id TEXT,\n\
             \tcurrency TEXT,\n\
             \treconciliation_account TEXT,\n\
             \tauxiliary_gl_account TEXT,\n\
             \tis_intercompany BOOLEAN,\n\
             \tbehavior TEXT,\n\
             \tpayment_terms TEXT,\n\
             \tis_fraud_actor BOOLEAN,\n\
             \tbank_account_count INT,\n\
             \tprimary_bank_name TEXT,\n\
             \tprimary_bank_country TEXT,\n\
             \tprimary_account_number TEXT,\n\
             \tprimary_routing_code TEXT\n\
             );\n"
        )?;
        writeln!(
            w,
            "\\copy vendors FROM 'vendors.csv' CSV HEADER;\n"
        )?;
    }

    if !result.master_data.customers.is_empty() {
        writeln!(
            w,
            "CREATE TABLE IF NOT EXISTS customers (\n\
             \tcustomer_id TEXT PRIMARY KEY,\n\
             \tname TEXT,\n\
             \tcountry TEXT,\n\
             \taccount_number TEXT,\n\
             \ttax_id TEXT,\n\
             \tcurrency TEXT,\n\
             \treconciliation_account TEXT,\n\
             \tauxiliary_gl_account TEXT,\n\
             \tis_intercompany BOOLEAN,\n\
             \tcredit_rating TEXT,\n\
             \tis_fraud_actor BOOLEAN,\n\
             \tbank_account_count INT,\n\
             \tprimary_bank_name TEXT,\n\
             \tprimary_bank_country TEXT,\n\
             \tprimary_account_number TEXT,\n\
             \tprimary_routing_code TEXT\n\
             );\n"
        )?;
        writeln!(
            w,
            "\\copy customers FROM 'customers.csv' CSV HEADER;\n"
        )?;
    }

    if !result.anomaly_labels.labels.is_empty() {
        writeln!(
            w,
            "CREATE TABLE IF NOT EXISTS anomaly_labels (\n\
             \tanomaly_id TEXT PRIMARY KEY,\n\
             \tanomaly_type TEXT,\n\
             \tdocument_id TEXT,\n\
             \tdocument_type TEXT,\n\
             \tcompany_code TEXT,\n\
             \tanomaly_date DATE,\n\
             \tdetection_timestamp TIMESTAMP,\n\
             \tconfidence DOUBLE PRECISION,\n\
             \tseverity INT,\n\
             \tdescription TEXT,\n\
             \tmonetary_impact NUMERIC,\n\
             \tis_injected BOOLEAN\n\
             );\n"
        )?;
        writeln!(
            w,
            "\\copy anomaly_labels FROM 'anomaly_labels.csv' CSV HEADER;\n"
        )?;
    }

    // Make all created tables readable by any database user (no security requirement).
    writeln!(
        w,
        "\n-- Grant read access to all forensic tables for any database user.\n\
         GRANT USAGE ON SCHEMA public TO PUBLIC;\n"
    )?;
    if !result.journal_entries.is_empty() {
        writeln!(w, "GRANT SELECT ON TABLE je_header TO PUBLIC;\n")?;
        writeln!(w, "GRANT SELECT ON TABLE je_line TO PUBLIC;\n")?;
    }
    if !result.master_data.employees.is_empty() {
        writeln!(w, "GRANT SELECT ON TABLE employees TO PUBLIC;\n")?;
    }
    if !result.master_data.vendors.is_empty() {
        writeln!(w, "GRANT SELECT ON TABLE vendors TO PUBLIC;\n")?;
    }
    if !result.master_data.customers.is_empty() {
        writeln!(w, "GRANT SELECT ON TABLE customers TO PUBLIC;\n")?;
    }
    if !result.anomaly_labels.labels.is_empty() {
        writeln!(w, "GRANT SELECT ON TABLE anomaly_labels TO PUBLIC;\n")?;
    }

    w.flush()?;
    Ok(())
}

/// Write GNN/GCN training datasets with train/validation/test splits.
fn write_gnn_training_output(
    result: &EnhancedGenerationResult,
    output_dir: &Path,
    cfg: &GnnTrainingOutputConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    if !cfg.enabled || result.journal_entries.is_empty() {
        return Ok(());
    }

    let dir = output_dir.join(&cfg.subdirectory);
    std::fs::create_dir_all(&dir)?;
    info!("Writing GNN training splits to: {}", dir.display());

    let n = result.journal_entries.len();
    if n == 0 {
        return Ok(());
    }

    // Build index list and order it according to the chosen strategy.
    let mut indices: Vec<usize> = (0..n).collect();
    match cfg.split_strategy {
        GnnSplitStrategy::ByDocument => {
            // Deterministic RNG so splits are reproducible for a given generation run.
            let mut rng = ChaCha8Rng::seed_from_u64(42);
            indices.as_mut_slice().shuffle(&mut rng);
        }
        GnnSplitStrategy::ByTime => {
            indices.sort_by_key(|&i| result.journal_entries[i].header.posting_date);
        }
    }

    // Compute split sizes; ensure they sum to n with test as the remainder.
    let train_size = ((cfg.train_ratio.max(0.0).min(1.0)) * n as f32).floor() as usize;
    let val_size = ((cfg.val_ratio.max(0.0).min(1.0)) * n as f32).floor() as usize;
    let capped_train = train_size.min(n);
    let capped_val = val_size.min(n.saturating_sub(capped_train));
    let capped_test = n.saturating_sub(capped_train + capped_val);

    let (train_idx, rest) = indices.split_at(capped_train);
    let (val_idx, test_idx) = rest.split_at(capped_val);

    // Helper to write a single split.
    let write_split = |name: &str,
                       idxs: &[usize]|
     -> Result<(), Box<dyn std::error::Error>> {
        if idxs.is_empty() {
            return Ok(());
        }
        let split_dir = dir.join(name);
        std::fs::create_dir_all(&split_dir)?;

        // Journal entries for this split.
        let split_entries: Vec<_> = idxs
            .iter()
            .map(|&i| result.journal_entries[i].clone())
            .collect();
        write_json(
            &split_entries,
            &split_dir.join("journal_entries.json"),
            &format!("GNN journal entries ({name})"),
        )?;

        // Restrict anomaly labels to documents present in this split.
        let doc_ids: HashSet<String> = split_entries
            .iter()
            .map(|je| je.header.document_id.to_string())
            .collect();
        let split_labels: Vec<_> = result
            .anomaly_labels
            .labels
            .iter()
            .cloned()
            .filter(|lbl| doc_ids.contains(&lbl.document_id))
            .collect();
        write_json(
            &split_labels,
            &split_dir.join("anomaly_labels.json"),
            &format!("GNN anomaly labels ({name})"),
        )?;

        Ok(())
    };

    write_split("train", train_idx)?;
    write_split("val", val_idx)?;
    write_split("test", test_idx)?;

    Ok(())
}

/// Write all generated data to the output directory.
///
/// This function exports every non-empty dataset from the generation result.
/// Journal entries are written as a flat CSV file (one row per line item)
/// and as a nested JSON file. Other data is written as JSON files since
/// many model types contain nested structures.
pub fn write_all_output(
    result: &EnhancedGenerationResult,
    output_dir: &Path,
    output_config: &OutputConfig,
) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(output_dir)?;
    info!("Writing comprehensive output to: {}", output_dir.display());

    // ========================================================================
    // Journal Entries (flat CSV + nested JSON)
    // ========================================================================
    if !result.journal_entries.is_empty() {
        // Write flat CSV with one row per line item (header fields repeated)
        if let Err(e) = write_journal_entries_csv(result, output_dir) {
            warn!("Failed to write journal_entries.csv: {}", e);
        }

        // Also write full journal entries as JSON for consumers that need the nested structure
        write_json(
            &result.journal_entries,
            &output_dir.join("journal_entries.json"),
            "Journal entries (JSON)",
        )?;

        // Optional forensic LLM-oriented exports (no train/val/test split).
        if output_config.output_for_forensic_llm.enabled {
            if let Err(e) = write_forensic_llm_output(
                result,
                output_dir,
                &output_config.output_for_forensic_llm,
            ) {
                warn!("Failed to write forensic LLM output: {}", e);
            }
        }

        // Optional GNN/GCN training exports with train/val/test splits.
        if output_config.output_for_gnn_training.enabled {
            if let Err(e) = write_gnn_training_output(
                result,
                output_dir,
                &output_config.output_for_gnn_training,
            ) {
                warn!("Failed to write GNN training output: {}", e);
            }
        }
    }

    // ========================================================================
    // Master Data
    // ========================================================================
    let md_dir = output_dir.join("master_data");
    if !result.master_data.vendors.is_empty()
        || !result.master_data.customers.is_empty()
        || !result.master_data.materials.is_empty()
        || !result.master_data.assets.is_empty()
        || !result.master_data.employees.is_empty()
    {
        std::fs::create_dir_all(&md_dir)?;
        info!("Writing master data...");

        write_json_safe(
            &result.master_data.vendors,
            &md_dir.join("vendors.json"),
            "Vendors",
        );
        write_json_safe(
            &result.master_data.customers,
            &md_dir.join("customers.json"),
            "Customers",
        );
        write_json_safe(
            &result.master_data.materials,
            &md_dir.join("materials.json"),
            "Materials",
        );
        write_json_safe(
            &result.master_data.assets,
            &md_dir.join("fixed_assets.json"),
            "Fixed assets",
        );
        write_json_safe(
            &result.master_data.employees,
            &md_dir.join("employees.json"),
            "Employees",
        );
    }

    // ========================================================================
    // Document Flows
    // ========================================================================
    let df_dir = output_dir.join("document_flows");
    if !result.document_flows.purchase_orders.is_empty()
        || !result.document_flows.sales_orders.is_empty()
    {
        std::fs::create_dir_all(&df_dir)?;
        info!("Writing document flows...");

        write_json_safe(
            &result.document_flows.purchase_orders,
            &df_dir.join("purchase_orders.json"),
            "Purchase orders",
        );
        write_json_safe(
            &result.document_flows.goods_receipts,
            &df_dir.join("goods_receipts.json"),
            "Goods receipts",
        );
        write_json_safe(
            &result.document_flows.vendor_invoices,
            &df_dir.join("vendor_invoices.json"),
            "Vendor invoices",
        );
        write_json_safe(
            &result.document_flows.payments,
            &df_dir.join("payments.json"),
            "Payments",
        );
        write_json_safe(
            &result.document_flows.sales_orders,
            &df_dir.join("sales_orders.json"),
            "Sales orders",
        );
        write_json_safe(
            &result.document_flows.deliveries,
            &df_dir.join("deliveries.json"),
            "Deliveries",
        );
        write_json_safe(
            &result.document_flows.customer_invoices,
            &df_dir.join("customer_invoices.json"),
            "Customer invoices",
        );

        // Note: P2P/O2C chain types do not implement Serialize, so we log
        // their counts instead. The individual documents above capture all data.
        if !result.document_flows.p2p_chains.is_empty() {
            info!(
                "  P2P chains: {} (data exported via individual document files)",
                result.document_flows.p2p_chains.len()
            );
        }
        if !result.document_flows.o2c_chains.is_empty() {
            info!(
                "  O2C chains: {} (data exported via individual document files)",
                result.document_flows.o2c_chains.len()
            );
        }
    }

    // ========================================================================
    // Subledger
    // ========================================================================
    let sl_dir = output_dir.join("subledger");
    if !result.subledger.ap_invoices.is_empty()
        || !result.subledger.ar_invoices.is_empty()
        || !result.subledger.fa_records.is_empty()
        || !result.subledger.inventory_positions.is_empty()
    {
        std::fs::create_dir_all(&sl_dir)?;
        info!("Writing subledger data...");

        write_json_safe(
            &result.subledger.ap_invoices,
            &sl_dir.join("ap_invoices.json"),
            "AP invoices",
        );
        write_json_safe(
            &result.subledger.ar_invoices,
            &sl_dir.join("ar_invoices.json"),
            "AR invoices",
        );
        write_json_safe(
            &result.subledger.fa_records,
            &sl_dir.join("fa_records.json"),
            "FA records",
        );
        write_json_safe(
            &result.subledger.inventory_positions,
            &sl_dir.join("inventory_positions.json"),
            "Inventory positions",
        );
        write_json_safe(
            &result.subledger.inventory_movements,
            &sl_dir.join("inventory_movements.json"),
            "Inventory movements",
        );
    }

    // ========================================================================
    // Audit
    // ========================================================================
    let audit_dir = output_dir.join("audit");
    if !result.audit.engagements.is_empty() {
        std::fs::create_dir_all(&audit_dir)?;
        info!("Writing audit data...");

        write_json_safe(
            &result.audit.engagements,
            &audit_dir.join("audit_engagements.json"),
            "Audit engagements",
        );
        write_json_safe(
            &result.audit.workpapers,
            &audit_dir.join("audit_workpapers.json"),
            "Audit workpapers",
        );
        write_json_safe(
            &result.audit.evidence,
            &audit_dir.join("audit_evidence.json"),
            "Audit evidence",
        );
        write_json_safe(
            &result.audit.risk_assessments,
            &audit_dir.join("audit_risk_assessments.json"),
            "Audit risk assessments",
        );
        write_json_safe(
            &result.audit.findings,
            &audit_dir.join("audit_findings.json"),
            "Audit findings",
        );
        write_json_safe(
            &result.audit.judgments,
            &audit_dir.join("audit_judgments.json"),
            "Audit judgments",
        );
    }

    // ========================================================================
    // Banking (JSON - keep existing format for backward compat)
    // ========================================================================
    let banking_dir = output_dir.join("banking");
    if !result.banking.customers.is_empty() {
        std::fs::create_dir_all(&banking_dir)?;
        info!("Writing banking data...");

        write_json_safe(
            &result.banking.customers,
            &banking_dir.join("banking_customers.json"),
            "Banking customers",
        );
        write_json_safe(
            &result.banking.accounts,
            &banking_dir.join("banking_accounts.json"),
            "Banking accounts",
        );
        write_json_safe(
            &result.banking.transactions,
            &banking_dir.join("banking_transactions.json"),
            "Banking transactions",
        );
        write_json_safe(
            &result.banking.transaction_labels,
            &banking_dir.join("aml_transaction_labels.json"),
            "AML transaction labels",
        );
        write_json_safe(
            &result.banking.customer_labels,
            &banking_dir.join("aml_customer_labels.json"),
            "AML customer labels",
        );
        write_json_safe(
            &result.banking.account_labels,
            &banking_dir.join("aml_account_labels.json"),
            "AML account labels",
        );
        write_json_safe(
            &result.banking.relationship_labels,
            &banking_dir.join("aml_relationship_labels.json"),
            "AML relationship labels",
        );
        write_json_safe(
            &result.banking.narratives,
            &banking_dir.join("aml_narratives.json"),
            "AML narratives",
        );
    }

    // ========================================================================
    // Sourcing (S2C)
    // ========================================================================
    let s2c_dir = output_dir.join("sourcing");
    if !result.sourcing.spend_analyses.is_empty() || !result.sourcing.sourcing_projects.is_empty() {
        std::fs::create_dir_all(&s2c_dir)?;
        info!("Writing sourcing (S2C) data...");

        write_json_safe(
            &result.sourcing.spend_analyses,
            &s2c_dir.join("spend_analyses.json"),
            "Spend analyses",
        );
        write_json_safe(
            &result.sourcing.sourcing_projects,
            &s2c_dir.join("sourcing_projects.json"),
            "Sourcing projects",
        );
        write_json_safe(
            &result.sourcing.qualifications,
            &s2c_dir.join("supplier_qualifications.json"),
            "Supplier qualifications",
        );
        write_json_safe(
            &result.sourcing.rfx_events,
            &s2c_dir.join("rfx_events.json"),
            "RFx events",
        );
        write_json_safe(
            &result.sourcing.bids,
            &s2c_dir.join("supplier_bids.json"),
            "Supplier bids",
        );
        write_json_safe(
            &result.sourcing.bid_evaluations,
            &s2c_dir.join("bid_evaluations.json"),
            "Bid evaluations",
        );
        write_json_safe(
            &result.sourcing.contracts,
            &s2c_dir.join("procurement_contracts.json"),
            "Procurement contracts",
        );
        write_json_safe(
            &result.sourcing.catalog_items,
            &s2c_dir.join("catalog_items.json"),
            "Catalog items",
        );
        write_json_safe(
            &result.sourcing.scorecards,
            &s2c_dir.join("supplier_scorecards.json"),
            "Supplier scorecards",
        );
    }

    // ========================================================================
    // Intercompany
    // ========================================================================
    let ic_dir = output_dir.join("intercompany");
    if !result.intercompany.matched_pairs.is_empty() {
        std::fs::create_dir_all(&ic_dir)?;
        info!("Writing intercompany data...");

        write_json_safe(
            &result.intercompany.matched_pairs,
            &ic_dir.join("ic_matched_pairs.json"),
            "IC matched pairs",
        );
        write_json_safe(
            &result.intercompany.seller_journal_entries,
            &ic_dir.join("ic_seller_journal_entries.json"),
            "IC seller journal entries",
        );
        write_json_safe(
            &result.intercompany.buyer_journal_entries,
            &ic_dir.join("ic_buyer_journal_entries.json"),
            "IC buyer journal entries",
        );
        write_json_safe(
            &result.intercompany.elimination_entries,
            &ic_dir.join("ic_elimination_entries.json"),
            "IC elimination entries",
        );
    }

    // ========================================================================
    // Financial Reporting
    // ========================================================================
    let fin_dir = output_dir.join("financial_reporting");
    if !result.financial_reporting.financial_statements.is_empty()
        || !result.financial_reporting.bank_reconciliations.is_empty()
    {
        std::fs::create_dir_all(&fin_dir)?;
        info!("Writing financial reporting data...");

        write_json_safe(
            &result.financial_reporting.financial_statements,
            &fin_dir.join("financial_statements.json"),
            "Financial statements",
        );
        write_json_safe(
            &result.financial_reporting.bank_reconciliations,
            &fin_dir.join("bank_reconciliations.json"),
            "Bank reconciliations",
        );
    }

    // ========================================================================
    // Period-Close Trial Balances
    // ========================================================================
    if !result.financial_reporting.trial_balances.is_empty() {
        let pc_dir = output_dir.join("period_close");
        std::fs::create_dir_all(&pc_dir)?;
        info!(
            "Writing {} period-close trial balances...",
            result.financial_reporting.trial_balances.len()
        );
        write_json_safe(
            &result.financial_reporting.trial_balances,
            &pc_dir.join("trial_balances.json"),
            "Period-close trial balances",
        );
    }

    // ========================================================================
    // Balance: Opening Balances + GL-Subledger Reconciliation
    // ========================================================================
    if !result.opening_balances.is_empty() || !result.subledger_reconciliation.is_empty() {
        let balance_dir = output_dir.join("balance");
        std::fs::create_dir_all(&balance_dir)?;
        info!("Writing balance data...");

        write_json_safe(
            &result.opening_balances,
            &balance_dir.join("opening_balances.json"),
            "Opening balances",
        );
        write_json_safe(
            &result.subledger_reconciliation,
            &balance_dir.join("subledger_reconciliation.json"),
            "Subledger reconciliation",
        );
    }

    // ========================================================================
    // HR (Payroll, Time Entries, Expense Reports)
    // ========================================================================
    let hr_dir = output_dir.join("hr");
    if !result.hr.payroll_runs.is_empty()
        || !result.hr.time_entries.is_empty()
        || !result.hr.expense_reports.is_empty()
    {
        std::fs::create_dir_all(&hr_dir)?;
        info!("Writing HR data...");

        write_json_safe(
            &result.hr.payroll_runs,
            &hr_dir.join("payroll_runs.json"),
            "Payroll runs",
        );
        write_json_safe(
            &result.hr.payroll_line_items,
            &hr_dir.join("payroll_line_items.json"),
            "Payroll line items",
        );
        write_json_safe(
            &result.hr.time_entries,
            &hr_dir.join("time_entries.json"),
            "Time entries",
        );
        write_json_safe(
            &result.hr.expense_reports,
            &hr_dir.join("expense_reports.json"),
            "Expense reports",
        );
    }

    // ========================================================================
    // Manufacturing
    // ========================================================================
    let mfg_dir = output_dir.join("manufacturing");
    if !result.manufacturing.production_orders.is_empty()
        || !result.manufacturing.quality_inspections.is_empty()
        || !result.manufacturing.cycle_counts.is_empty()
    {
        std::fs::create_dir_all(&mfg_dir)?;
        info!("Writing manufacturing data...");

        write_json_safe(
            &result.manufacturing.production_orders,
            &mfg_dir.join("production_orders.json"),
            "Production orders",
        );
        write_json_safe(
            &result.manufacturing.quality_inspections,
            &mfg_dir.join("quality_inspections.json"),
            "Quality inspections",
        );
        write_json_safe(
            &result.manufacturing.cycle_counts,
            &mfg_dir.join("cycle_counts.json"),
            "Cycle counts",
        );
    }

    // ========================================================================
    // Sales, KPIs, Budgets
    // ========================================================================
    let sales_dir = output_dir.join("sales_kpi_budgets");
    if !result.sales_kpi_budgets.sales_quotes.is_empty()
        || !result.sales_kpi_budgets.kpis.is_empty()
        || !result.sales_kpi_budgets.budgets.is_empty()
    {
        std::fs::create_dir_all(&sales_dir)?;
        info!("Writing sales, KPI, and budget data...");

        write_json_safe(
            &result.sales_kpi_budgets.sales_quotes,
            &sales_dir.join("sales_quotes.json"),
            "Sales quotes",
        );
        write_json_safe(
            &result.sales_kpi_budgets.kpis,
            &sales_dir.join("management_kpis.json"),
            "Management KPIs",
        );
        write_json_safe(
            &result.sales_kpi_budgets.budgets,
            &sales_dir.join("budgets.json"),
            "Budgets",
        );
    }

    // ========================================================================
    // Tax
    // ========================================================================
    let tax_dir = output_dir.join("tax");
    if !result.tax.jurisdictions.is_empty()
        || !result.tax.codes.is_empty()
        || !result.tax.tax_provisions.is_empty()
    {
        std::fs::create_dir_all(&tax_dir)?;
        info!("Writing tax data...");

        write_json_safe(
            &result.tax.jurisdictions,
            &tax_dir.join("tax_jurisdictions.json"),
            "Tax jurisdictions",
        );
        write_json_safe(
            &result.tax.codes,
            &tax_dir.join("tax_codes.json"),
            "Tax codes",
        );
        write_json_safe(
            &result.tax.tax_provisions,
            &tax_dir.join("tax_provisions.json"),
            "Tax provisions",
        );
        write_json_safe(
            &result.tax.tax_lines,
            &tax_dir.join("tax_lines.json"),
            "Tax lines",
        );
        write_json_safe(
            &result.tax.tax_returns,
            &tax_dir.join("tax_returns.json"),
            "Tax returns",
        );
        write_json_safe(
            &result.tax.withholding_records,
            &tax_dir.join("withholding_records.json"),
            "Withholding tax records",
        );
        if !result.tax.tax_anomaly_labels.is_empty() {
            write_json_safe(
                &result.tax.tax_anomaly_labels,
                &tax_dir.join("tax_anomaly_labels.json"),
                "Tax anomaly labels",
            );
        }
    }

    // ========================================================================
    // ESG
    // ========================================================================
    let esg_dir = output_dir.join("esg");
    if !result.esg.emissions.is_empty()
        || !result.esg.energy.is_empty()
        || !result.esg.diversity.is_empty()
        || !result.esg.governance.is_empty()
    {
        std::fs::create_dir_all(&esg_dir)?;
        info!("Writing ESG data...");

        write_json_safe(
            &result.esg.emissions,
            &esg_dir.join("emission_records.json"),
            "Emission records",
        );
        write_json_safe(
            &result.esg.energy,
            &esg_dir.join("energy_consumption.json"),
            "Energy consumption",
        );
        write_json_safe(
            &result.esg.water,
            &esg_dir.join("water_usage.json"),
            "Water usage",
        );
        write_json_safe(
            &result.esg.waste,
            &esg_dir.join("waste_records.json"),
            "Waste records",
        );
        write_json_safe(
            &result.esg.diversity,
            &esg_dir.join("workforce_diversity.json"),
            "Workforce diversity",
        );
        write_json_safe(
            &result.esg.pay_equity,
            &esg_dir.join("pay_equity.json"),
            "Pay equity",
        );
        write_json_safe(
            &result.esg.safety_incidents,
            &esg_dir.join("safety_incidents.json"),
            "Safety incidents",
        );
        write_json_safe(
            &result.esg.safety_metrics,
            &esg_dir.join("safety_metrics.json"),
            "Safety metrics",
        );
        write_json_safe(
            &result.esg.governance,
            &esg_dir.join("governance_metrics.json"),
            "Governance metrics",
        );
        write_json_safe(
            &result.esg.supplier_assessments,
            &esg_dir.join("supplier_esg_assessments.json"),
            "Supplier ESG assessments",
        );
        write_json_safe(
            &result.esg.materiality,
            &esg_dir.join("materiality_assessments.json"),
            "Materiality assessments",
        );
        write_json_safe(
            &result.esg.disclosures,
            &esg_dir.join("esg_disclosures.json"),
            "ESG disclosures",
        );
        write_json_safe(
            &result.esg.climate_scenarios,
            &esg_dir.join("climate_scenarios.json"),
            "Climate scenarios",
        );
        write_json_safe(
            &result.esg.anomaly_labels,
            &esg_dir.join("esg_anomaly_labels.json"),
            "ESG anomaly labels",
        );
    }

    // ========================================================================
    // Process Mining (OCPM)
    // ========================================================================
    if let Some(ref event_log) = result.ocpm.event_log {
        if !event_log.events.is_empty() || !event_log.objects.is_empty() {
            let pm_dir = output_dir.join("process_mining");
            std::fs::create_dir_all(&pm_dir)?;
            info!("Writing process mining (OCPM) data...");

            // Write the full OCEL 2.0 event log
            match serde_json::to_string_pretty(event_log) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(pm_dir.join("event_log.json"), json) {
                        warn!("Failed to write OCPM event log: {}", e);
                    } else {
                        info!(
                            "  Event log written: {} events, {} objects",
                            result.ocpm.event_count, result.ocpm.object_count
                        );
                    }
                }
                Err(e) => warn!("Failed to serialize OCPM event log: {}", e),
            }

            // Write events separately for easy consumption
            if !event_log.events.is_empty() {
                match serde_json::to_string_pretty(&event_log.events) {
                    Ok(json) => {
                        if let Err(e) = std::fs::write(pm_dir.join("events.json"), json) {
                            warn!("Failed to write OCPM events: {}", e);
                        } else {
                            info!("  Events written: {} records", event_log.events.len());
                        }
                    }
                    Err(e) => warn!("Failed to serialize OCPM events: {}", e),
                }
            }

            // Write objects separately for easy consumption
            if !event_log.objects.is_empty() {
                let objects: Vec<&_> = event_log.objects.iter().collect();
                match serde_json::to_string_pretty(&objects) {
                    Ok(json) => {
                        if let Err(e) = std::fs::write(pm_dir.join("objects.json"), json) {
                            warn!("Failed to write OCPM objects: {}", e);
                        } else {
                            info!("  Objects written: {} records", event_log.objects.len());
                        }
                    }
                    Err(e) => warn!("Failed to serialize OCPM objects: {}", e),
                }
            }

            // Write process variants if any were computed
            if !event_log.variants.is_empty() {
                let variants: Vec<&_> = event_log.variants.values().collect();
                match serde_json::to_string_pretty(&variants) {
                    Ok(json) => {
                        if let Err(e) = std::fs::write(pm_dir.join("process_variants.json"), json) {
                            warn!("Failed to write process variants: {}", e);
                        } else {
                            info!(
                                "  Process variants written: {} variants",
                                event_log.variants.len()
                            );
                        }
                    }
                    Err(e) => warn!("Failed to serialize process variants: {}", e),
                }
            }
        }
    }

    // ========================================================================
    // Chart of Accounts
    // ========================================================================
    match serde_json::to_string_pretty(&result.chart_of_accounts) {
        Ok(json) => {
            if let Err(e) = std::fs::write(output_dir.join("chart_of_accounts.json"), json) {
                warn!("Failed to write chart of accounts: {}", e);
            } else {
                info!("  Chart of accounts written");
            }
        }
        Err(e) => warn!("Failed to serialize chart of accounts: {}", e),
    }

    // ========================================================================
    // Balance Validation Summary
    // ========================================================================
    if result.balance_validation.validated {
        match serde_json::to_string_pretty(&BalanceValidationSummary::from(
            &result.balance_validation,
        )) {
            Ok(json) => {
                if let Err(e) = std::fs::write(output_dir.join("balance_validation.json"), json) {
                    warn!("Failed to write balance validation: {}", e);
                } else {
                    info!("  Balance validation summary written");
                }
            }
            Err(e) => warn!("Failed to serialize balance validation: {}", e),
        }
    }

    // ========================================================================
    // Data Quality Statistics (now serializable directly via Serialize derives)
    // ========================================================================
    {
        match serde_json::to_string_pretty(&result.data_quality_stats) {
            Ok(json) => {
                if let Err(e) = std::fs::write(output_dir.join("data_quality_stats.json"), json) {
                    warn!("Failed to write data quality stats: {}", e);
                } else {
                    info!("  Data quality stats written (full detail)");
                }
            }
            Err(e) => warn!("Failed to serialize data quality stats: {}", e),
        }
    }

    // ========================================================================
    // Internal Controls
    // ========================================================================
    if !result.internal_controls.is_empty() {
        let ctrl_dir = output_dir.join("internal_controls");
        std::fs::create_dir_all(&ctrl_dir)?;
        info!("Writing internal controls data...");

        write_json_safe(
            &result.internal_controls,
            &ctrl_dir.join("internal_controls.json"),
            "Internal controls",
        );
    }

    // ========================================================================
    // Accounting Standards
    // ========================================================================
    if !result.accounting_standards.contracts.is_empty()
        || !result.accounting_standards.impairment_tests.is_empty()
    {
        let acct_dir = output_dir.join("accounting_standards");
        std::fs::create_dir_all(&acct_dir)?;
        info!("Writing accounting standards data...");

        write_json_safe(
            &result.accounting_standards.contracts,
            &acct_dir.join("customer_contracts.json"),
            "Customer contracts",
        );
        write_json_safe(
            &result.accounting_standards.impairment_tests,
            &acct_dir.join("impairment_tests.json"),
            "Impairment tests",
        );
    }

    // ========================================================================
    // Quality Gate Results
    // ========================================================================
    if let Some(ref gate_result) = result.gate_result {
        match serde_json::to_string_pretty(gate_result) {
            Ok(json) => {
                if let Err(e) = std::fs::write(output_dir.join("quality_gate_result.json"), json) {
                    warn!("Failed to write quality gate result: {}", e);
                } else {
                    info!(
                        "  Quality gate result written (passed={})",
                        gate_result.passed
                    );
                }
            }
            Err(e) => warn!("Failed to serialize quality gate result: {}", e),
        }
    }

    // ========================================================================
    // Treasury
    // ========================================================================
    if !result.treasury.debt_instruments.is_empty()
        || !result.treasury.cash_positions.is_empty()
        || !result.treasury.hedging_instruments.is_empty()
    {
        let treasury_dir = output_dir.join("treasury");
        std::fs::create_dir_all(&treasury_dir)?;
        info!("Writing treasury data...");

        write_json_safe(
            &result.treasury.debt_instruments,
            &treasury_dir.join("debt_instruments.json"),
            "Debt instruments",
        );
        write_json_safe(
            &result.treasury.hedging_instruments,
            &treasury_dir.join("hedging_instruments.json"),
            "Hedging instruments",
        );
        write_json_safe(
            &result.treasury.hedge_relationships,
            &treasury_dir.join("hedge_relationships.json"),
            "Hedge relationships",
        );
        write_json_safe(
            &result.treasury.cash_positions,
            &treasury_dir.join("cash_positions.json"),
            "Cash positions",
        );
        write_json_safe(
            &result.treasury.cash_forecasts,
            &treasury_dir.join("cash_forecasts.json"),
            "Cash forecasts",
        );
        write_json_safe(
            &result.treasury.cash_pools,
            &treasury_dir.join("cash_pools.json"),
            "Cash pools",
        );
        write_json_safe(
            &result.treasury.cash_pool_sweeps,
            &treasury_dir.join("cash_pool_sweeps.json"),
            "Cash pool sweeps",
        );
        if !result.treasury.treasury_anomaly_labels.is_empty() {
            write_json_safe(
                &result.treasury.treasury_anomaly_labels,
                &treasury_dir.join("treasury_anomaly_labels.json"),
                "Treasury anomaly labels",
            );
        }
    }

    // ========================================================================
    // Project Accounting
    // ========================================================================
    if !result.project_accounting.projects.is_empty() {
        let pa_dir = output_dir.join("project_accounting");
        std::fs::create_dir_all(&pa_dir)?;
        info!("Writing project accounting data...");

        write_json_safe(
            &result.project_accounting.projects,
            &pa_dir.join("projects.json"),
            "Projects",
        );
        write_json_safe(
            &result.project_accounting.cost_lines,
            &pa_dir.join("cost_lines.json"),
            "Project cost lines",
        );
        write_json_safe(
            &result.project_accounting.revenue_records,
            &pa_dir.join("revenue_records.json"),
            "Project revenue records",
        );
        write_json_safe(
            &result.project_accounting.earned_value_metrics,
            &pa_dir.join("earned_value_metrics.json"),
            "Earned value metrics",
        );
        write_json_safe(
            &result.project_accounting.change_orders,
            &pa_dir.join("change_orders.json"),
            "Change orders",
        );
        write_json_safe(
            &result.project_accounting.milestones,
            &pa_dir.join("milestones.json"),
            "Project milestones",
        );
    }

    // ========================================================================
    // Graph Export Summary
    // ========================================================================
    if result.graph_export.exported {
        let graph_dir = output_dir.join("graph_export");
        std::fs::create_dir_all(&graph_dir).ok();
        match serde_json::to_string_pretty(&result.graph_export) {
            Ok(json) => {
                if let Err(e) = std::fs::write(graph_dir.join("graph_export_summary.json"), json) {
                    warn!("Failed to write graph export summary: {}", e);
                } else {
                    info!("  Graph export summary written");
                }
            }
            Err(e) => warn!("Failed to serialize graph export summary: {}", e),
        }
    }

    // ========================================================================
    // Generation Statistics
    // ========================================================================
    match serde_json::to_string_pretty(&result.statistics) {
        Ok(json) => {
            if let Err(e) = std::fs::write(output_dir.join("generation_statistics.json"), json) {
                warn!("Failed to write generation statistics: {}", e);
            } else {
                info!("  Generation statistics written");
            }
        }
        Err(e) => warn!("Failed to serialize generation statistics: {}", e),
    }

    info!("Output writing complete.");
    Ok(())
}

/// Write JSON with error handling - logs a warning on failure but does not abort.
fn write_json_safe<T: serde::Serialize>(data: &[T], path: &Path, label: &str) {
    if let Err(e) = write_json(data, path, label) {
        warn!("Failed to write {}: {}", label, e);
    }
}

/// Serializable summary of balance validation (avoids serializing the full
/// `BalanceValidationResult` which has non-Serialize validation error types).
#[derive(serde::Serialize)]
struct BalanceValidationSummary {
    validated: bool,
    is_balanced: bool,
    entries_processed: u64,
    total_debits: String,
    total_credits: String,
    accounts_tracked: usize,
    companies_tracked: usize,
    has_unbalanced_entries: bool,
    validation_error_count: usize,
}

impl BalanceValidationSummary {
    fn from(v: &datasynth_runtime::enhanced_orchestrator::BalanceValidationResult) -> Self {
        Self {
            validated: v.validated,
            is_balanced: v.is_balanced,
            entries_processed: v.entries_processed,
            total_debits: v.total_debits.to_string(),
            total_credits: v.total_credits.to_string(),
            accounts_tracked: v.accounts_tracked,
            companies_tracked: v.companies_tracked,
            has_unbalanced_entries: v.has_unbalanced_entries,
            validation_error_count: v.validation_errors.len(),
        }
    }
}
