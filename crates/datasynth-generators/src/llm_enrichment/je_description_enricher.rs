//! LLM-powered journal entry description enrichment (e.g. FEC libellé).
//!
//! Generates realistic, consistent header_text and line_text for journal entries
//! so that FEC "Libellé de l'écriture comptable" is meaningful. For each entry the
//! LLM receives full context (document type, reference, date, all lines with account
//! and amount) and returns one header and one description per line, consistent
//! within the entry.

use std::sync::Arc;

use datasynth_core::error::SynthError;
use datasynth_core::llm::{LlmProvider, LlmRequest};
use datasynth_core::models::{ChartOfAccounts, JournalEntry};
use rust_decimal::Decimal;

/// Enriches journal entry header_text and line_text using an LLM.
///
/// Each entry is sent with full context so the model can produce consistent
/// descriptions (e.g. for document-flow entries sharing the same business event).
pub struct JournalEntryLlmEnricher {
    provider: Arc<dyn LlmProvider>,
}

impl JournalEntryLlmEnricher {
    /// Create a new enricher with the given LLM provider.
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// Build context string for one journal entry (for the LLM prompt).
    /// Includes document type, source, process, reference, date, currency, company, and each line (account + description + DR/CR + amount).
    fn entry_context(entry: &JournalEntry, coa: &ChartOfAccounts, in_french: bool) -> String {
        let doc_type = entry.header.document_type.as_str();
        let ref_str = entry
            .header
            .reference
            .as_deref()
            .unwrap_or("—");
        let date = entry.header.posting_date.format("%Y-%m-%d").to_string();
        let currency = entry.header.currency.as_str();
        let company = entry.header.company_code.as_str();
        let source = format!("{}", entry.header.source);
        let process = entry
            .header
            .business_process
            .as_ref()
            .map(|p| format!("{:?}", p))
            .unwrap_or_else(|| "—".to_string());
        let mut lines_ctx = String::new();
        for (i, line) in entry.lines.iter().enumerate() {
            let account_desc = coa
                .get_account(&line.gl_account)
                .map(|a| a.short_description.as_str())
                .unwrap_or(line.gl_account.as_str());
            let amount = if line.debit_amount > Decimal::ZERO {
                line.debit_amount
            } else {
                line.credit_amount
            };
            let line_fmt = if in_french {
                let dc = if line.debit_amount > Decimal::ZERO {
                    "débit"
                } else {
                    "crédit"
                };
                format!("compte {} ({}): {} {}", line.gl_account, account_desc, dc, amount)
            } else {
                let dc = if line.debit_amount > Decimal::ZERO { "DR" } else { "CR" };
                format!("{} {} {}", account_desc, dc, amount)
            };
            if i > 0 {
                lines_ctx.push_str("; ");
            }
            lines_ctx.push_str(&line_fmt);
        }
        if in_french {
            format!(
                "Type de pièce: {}. Origine: {}. Processus: {}. Référence: {}. Date: {}. Société: {}. Devise: {}. Lignes: {}",
                doc_type, source, process, ref_str, date, company, currency, lines_ctx
            )
        } else {
            format!(
                "Document type: {}. Source: {}. Process: {}. Reference: {}. Date: {}. Company: {}. Currency: {}. Lines: {}",
                doc_type, source, process, ref_str, date, company, currency, lines_ctx
            )
        }
    }

    /// Build the prompt for one journal entry; response format: first line = header, then one line per JE line.
    fn build_request_for_entry(
        entry: &JournalEntry,
        coa: &ChartOfAccounts,
        seed: u64,
        response_in_french: bool,
    ) -> LlmRequest {
        let context = Self::entry_context(entry, coa, response_in_french);
        let n_lines = entry.lines.len();
        let (_lang_instruction, prompt, system) = if response_in_french {
            let lang_instruction = " Réponds UNIQUEMENT en français (France), pour un FEC (Fichier des Écritures Comptables). Les libellés doivent être courts, réalistes et conformes à la pratique comptable française (factures fournisseurs, virements, rapprochements, écritures manuelles, etc.).";
            let prompt = format!(
                "Génère un libellé comptable réaliste pour cette écriture (colonne \"Libellé de l'écriture comptable\" du FEC). \
                 Contexte: {}.{}\
                 Donne sur la première ligne un libellé court pour l'écriture (en-tête). \
                 Puis exactement {} lignes supplémentaires, une par ligne comptable, cohérentes avec l'en-tête. \
                 Pas de numérotation, pas de puces. Uniquement les {} lignes de texte, sans préambule ni explication.",
                context, lang_instruction, n_lines, n_lines + 1
            );
            let system = "Tu es un générateur de données comptables françaises. Réponds UNIQUEMENT par les lignes demandées en français: première ligne = libellé de l'écriture (court, style FEC), lignes suivantes = un libellé court par ligne comptable, cohérents entre eux. Pour les écritures manuelles utilise des libellés du type \"Avoir fournisseur X\", \"Rapprochement bancaire\", \"Régularisation stock\", \"Facture n°...\", etc. Aucun texte avant ou après les lignes."
                .to_string();
            (lang_instruction, prompt, system)
        } else {
            let lang_instruction = "";
            let prompt = format!(
                "Generate a realistic accounting description for this journal entry (for FEC libellé). \
                 Context: {}.{}\
                 Return on the first line a short header description (one phrase). \
                 Then exactly {} more lines, one per journal line, consistent with the header. \
                 No numbering, no bullets. Only the {} lines of text.",
                context, lang_instruction, n_lines, n_lines + 1
            );
            let system = "You are an accounting data generator. Return only the requested lines: first line = entry header description, next lines = one short description per journal line, consistent with each other. For manual entries use realistic narrative or approval-style libellés (e.g. adjustment reason, approval ref). No extra text."
                .to_string();
            (lang_instruction, prompt, system)
        };
        LlmRequest::new(prompt)
            .with_system(system)
            .with_max_tokens(512)
            .with_temperature(0.5)
            .with_seed(seed)
    }

    /// Parse response: first line = header, rest = line_texts in order.
    fn parse_response(response: &str, expected_lines: usize) -> (String, Vec<String>) {
        let all: Vec<String> = response
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let header = all.first().cloned().unwrap_or_else(|| "Journal entry".to_string());
        let line_texts: Vec<String> = all
            .iter()
            .skip(1)
            .take(expected_lines)
            .cloned()
            .collect();
        (header, line_texts)
    }

    /// Enrich a slice of journal entries in place, in batches (complete_batch per chunk).
    ///
    /// Uses `batch_size` requests per provider.complete_batch call. Entries are
    /// mutated with header_text and line line_text. Shortfalls use a simple fallback.
    /// When `response_in_french` is true (e.g. French GAAP / FEC), descriptions must be in French.
    /// If `progress` is provided, it is called with the number of entries processed after each chunk.
    pub fn enrich_entries(
        &self,
        entries: &mut [JournalEntry],
        coa: &ChartOfAccounts,
        batch_size: usize,
        seed: u64,
        response_in_french: bool,
        mut progress: Option<&mut dyn FnMut(usize)>,
    ) -> Result<(usize, usize), SynthError> {
        if entries.is_empty() {
            return Ok((0, 0));
        }

        let mut total_entries_ok = 0usize;
        let mut total_lines_ok = 0usize;

        for (chunk_start, chunk) in entries.chunks_mut(batch_size).enumerate() {
            let batch_seed = seed.wrapping_add(chunk_start as u64 * 10000);
            let requests: Vec<LlmRequest> = chunk
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    Self::build_request_for_entry(
                        entry,
                        coa,
                        batch_seed.wrapping_add(i as u64),
                        response_in_french,
                    )
                })
                .collect();

            match self.provider.complete_batch(&requests) {
                Ok(responses) => {
                    for (entry, response) in chunk.iter_mut().zip(responses.iter()) {
                        let (header_text, line_texts) =
                            Self::parse_response(&response.content, entry.lines.len());
                        entry.header.header_text = Some(header_text.clone());
                        total_entries_ok += 1;
                        for (line, text) in entry.lines.iter_mut().zip(line_texts.iter()) {
                            line.line_text = Some(text.clone());
                            total_lines_ok += 1;
                        }
                        // If we got fewer line_texts than lines, fill rest with header
                        for line in entry.lines.iter_mut().skip(line_texts.len()) {
                            line.line_text = Some(header_text.clone());
                            total_lines_ok += 1;
                        }
                    }
                    if let Some(ref mut prog) = progress {
                        prog(total_entries_ok);
                    }
                }
                Err(_) => {
                    // Fallback: use document type + reference as description
                    for entry in chunk.iter_mut() {
                        let fallback = format!(
                            "{} - {}",
                            entry.header.document_type,
                            entry
                                .header
                                .reference
                                .as_deref()
                                .unwrap_or("n/a")
                        );
                        entry.header.header_text = Some(fallback.clone());
                        for line in &mut entry.lines {
                            line.line_text = Some(fallback.clone());
                            total_lines_ok += 1;
                        }
                        total_entries_ok += 1;
                    }
                    if let Some(ref mut prog) = progress {
                        prog(total_entries_ok);
                    }
                }
            }
        }

        Ok((total_entries_ok, total_lines_ok))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use datasynth_core::llm::MockLlmProvider;
    use datasynth_core::models::{
        AccountSubType, AccountType, ChartOfAccounts, CoAComplexity, GLAccount, IndustrySector,
        JournalEntryHeader, JournalEntryLine,
    };
    use uuid::Uuid;

    fn sample_coa() -> ChartOfAccounts {
        let mut coa = ChartOfAccounts::new(
            "TEST".to_string(),
            "Test CoA".to_string(),
            "US".to_string(),
            IndustrySector::Manufacturing,
            CoAComplexity::Small,
        );
        coa.add_account(GLAccount::new(
            "2900".to_string(),
            "GR/IR Clearing".to_string(),
            AccountType::Asset,
            AccountSubType::OtherAssets,
        ));
        coa.add_account(GLAccount::new(
            "2000".to_string(),
            "Accounts Payable".to_string(),
            AccountType::Liability,
            AccountSubType::AccountsPayable,
        ));
        coa
    }

    fn sample_entry() -> JournalEntry {
        let doc_id = Uuid::nil();
        let mut header = JournalEntryHeader::with_deterministic_id(
            "C001".to_string(),
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            doc_id,
        );
        header.document_type = "KR".to_string();
        header.reference = Some("VI:INV-001".to_string());
        let mut entry = JournalEntry::new(header);
        entry.add_line(JournalEntryLine::debit(
            doc_id,
            1,
            "2900".to_string(),
            rust_decimal_macros::dec!(1000),
        ));
        entry.add_line(JournalEntryLine::credit(
            doc_id,
            2,
            "2000".to_string(),
            rust_decimal_macros::dec!(1000),
        ));
        entry
    }

    #[test]
    fn test_enrich_entries_sets_header_and_line_text() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = JournalEntryLlmEnricher::new(provider);
        let coa = sample_coa();
        let mut entries = vec![sample_entry()];
        let (n_entries, n_lines) = enricher
            .enrich_entries(&mut entries, &coa, 10, 100, false, None)
            .unwrap();
        assert!(n_entries >= 1);
        assert!(n_lines >= 2);
        assert!(entries[0].header.header_text.is_some());
        assert!(entries[0].lines[0].line_text.is_some());
    }

    #[test]
    fn test_enrich_entries_empty() {
        let provider = Arc::new(MockLlmProvider::new(42));
        let enricher = JournalEntryLlmEnricher::new(provider);
        let coa = sample_coa();
        let mut entries: Vec<JournalEntry> = vec![];
        let (n_entries, n_lines) = enricher
            .enrich_entries(&mut entries, &coa, 10, 100, false, None)
            .unwrap();
        assert_eq!(n_entries, 0);
        assert_eq!(n_lines, 0);
    }
}
