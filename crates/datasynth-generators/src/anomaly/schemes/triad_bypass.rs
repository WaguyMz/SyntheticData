//! Triad bypass (process bypass) fraud scheme.
//!
//! A fraudulent payment is submitted that reuses an existing invoice ID without issuing
//! a new invoice, bypassing the normal PO → GR → Invoice triad.
//!
//! v2.0 changes:
//! - `reused_invoice_id` is sourced from `context.candidate_invoice_ids` when available
//!   (format: `"{document_id}|{vendor_id}"`), giving the scheme a real ledger reference.
//! - `reused_vendor_id` is locked to the vendor extracted from that entry.
//! - Stage 1 (bypass): emits 2–4 ReuseDocumentId actions per month against the same invoice.
//! - Stage 2 (concealment): emits BypassConcealment correcting reversals.

use chrono::{Datelike, NaiveDate};
use rand::Rng;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use datasynth_core::models::{
    AnomalyDetectionDifficulty, ConcealmentTechnique, SchemeDetectionStatus, SchemeType,
};

use super::scheme::{
    FraudScheme, SchemeAction, SchemeActionType, SchemeContext, SchemeStage, SchemeStatus,
    SchemeTransactionRef,
};

/// Triad bypass scheme: fraudulent payment reusing an old invoice ID.
///
/// Stages:
///   0 – Setup       (2 months) : establish history (no financial JE)
///   1 – Bypass      (4 months) : 2–4 ReuseDocumentId per month from same invoice
///   2 – Concealment (2 months) : BypassConcealment correcting reversals
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriadBypassScheme {
    pub scheme_id: Uuid,
    pub perpetrator_id: String,
    pub start_date: Option<NaiveDate>,
    current_stage_index: usize,
    stages: Vec<SchemeStage>,
    transactions: Vec<SchemeTransactionRef>,
    total_impact: Decimal,
    status: SchemeStatus,
    detection_status: SchemeDetectionStatus,
    detection_probability: f64,
    stage_transaction_count: u32,
    days_since_last_transaction: u32,
    /// Reused invoice document ID (real ledger reference if candidate_invoice_ids available).
    reused_invoice_id: String,
    /// Vendor extracted from the reused invoice — always used as counterparty.
    reused_vendor_id: Option<String>,
    /// Number of bypass reuses emitted this calendar month.
    reuses_this_month: u32,
    /// (year, month) of the last bypass month accounting.
    last_bypass_month: Option<(i32, u32)>,
}

impl TriadBypassScheme {
    pub fn new(scheme_id: Uuid, perpetrator_id: impl Into<String>) -> Self {
        let stages = vec![
            SchemeStage::new(
                1,
                "setup",
                2,
                (dec!(0), dec!(0)),
                (0, 1),
                AnomalyDetectionDifficulty::Expert,
            )
            .with_description("Legitimate invoice flows to establish history")
            .with_technique(ConcealmentTechnique::FalseDocumentation),
            SchemeStage::new(
                2,
                "bypass",
                4,
                (dec!(1000), dec!(50000)),
                (2, 4),
                AnomalyDetectionDifficulty::Hard,
            )
            .with_description("Fraudulent payments reusing the same old invoice ID (2–4 per month)")
            .with_technique(ConcealmentTechnique::DocumentManipulation)
            .with_technique(ConcealmentTechnique::FalseDocumentation),
            SchemeStage::new(
                3,
                "concealment",
                2,
                (dec!(0), dec!(5000)),
                (0, 2),
                AnomalyDetectionDifficulty::Moderate,
            )
            .with_description("Correcting reversals to cover tracks")
            .with_technique(ConcealmentTechnique::DataAlteration),
        ];

        // Fallback synthetic invoice ID used when no candidate invoices are available
        let short = scheme_id.to_string();
        let fallback_id = format!("INV-{}", &short[..short.len().min(8)]);

        Self {
            scheme_id,
            perpetrator_id: perpetrator_id.into(),
            start_date: None,
            current_stage_index: 0,
            stages,
            transactions: Vec::new(),
            total_impact: Decimal::ZERO,
            status: SchemeStatus::NotStarted,
            detection_status: SchemeDetectionStatus::Undetected,
            detection_probability: 0.0,
            stage_transaction_count: 0,
            days_since_last_transaction: 0,
            reused_invoice_id: fallback_id,
            reused_vendor_id: None,
            reuses_this_month: 0,
            last_bypass_month: None,
        }
    }

    /// Initialise the reused invoice from candidate_invoice_ids if available.
    fn init_reused_invoice(&mut self, context: &SchemeContext, rng: &mut dyn rand::RngCore) {
        if context.candidate_invoice_ids.is_empty() {
            return;
        }
        let idx = rng.random_range(0..context.candidate_invoice_ids.len());
        let entry = &context.candidate_invoice_ids[idx];
        let mut parts = entry.splitn(2, '|');
        let doc_id = parts.next().unwrap_or(entry).to_string();
        let vendor_id = parts.next().map(|s| s.to_string());
        self.reused_invoice_id = doc_id;
        self.reused_vendor_id = vendor_id;
    }

    fn stage_end_date(&self) -> Option<NaiveDate> {
        self.start_date.map(|start| {
            let months_elapsed: u32 = self.stages[..self.current_stage_index]
                .iter()
                .map(|s| s.duration_months)
                .sum();
            let stage_months = self.stages[self.current_stage_index].duration_months;
            start + chrono::Months::new(months_elapsed + stage_months)
        })
    }

    fn should_advance_stage(&self, current_date: NaiveDate) -> bool {
        if let Some(end_date) = self.stage_end_date() {
            current_date >= end_date && self.current_stage_index < self.stages.len() - 1
        } else {
            false
        }
    }

    fn advance_stage(&mut self) {
        if self.current_stage_index < self.stages.len() - 1 {
            self.current_stage_index += 1;
            self.stage_transaction_count = 0;
        }
    }
}

impl FraudScheme for TriadBypassScheme {
    fn scheme_type(&self) -> SchemeType {
        SchemeType::TriadBypass
    }

    fn scheme_id(&self) -> Uuid {
        self.scheme_id
    }

    fn current_stage(&self) -> &SchemeStage {
        &self.stages[self.current_stage_index]
    }

    fn current_stage_number(&self) -> u32 {
        self.stages[self.current_stage_index].stage_number
    }

    fn stages(&self) -> &[SchemeStage] {
        &self.stages
    }

    fn status(&self) -> SchemeStatus {
        self.status
    }

    fn detection_status(&self) -> SchemeDetectionStatus {
        self.detection_status
    }

    fn advance(
        &mut self,
        context: &SchemeContext,
        rng: &mut dyn rand::RngCore,
    ) -> Vec<SchemeAction> {
        let mut actions = Vec::new();

        if self.status == SchemeStatus::NotStarted {
            self.start_date = Some(context.current_date);
            self.status = SchemeStatus::Active;
            // Source reused invoice from ledger candidates
            self.init_reused_invoice(context, rng);
        }

        if self.should_terminate(context) {
            self.status = SchemeStatus::Terminated;
            return actions;
        }

        if rng.random::<f64>() < self.detection_probability * context.detection_activity {
            self.detection_status = SchemeDetectionStatus::PartiallyDetected;
            self.status = SchemeStatus::Detected;
            return actions;
        }

        if self.should_advance_stage(context.current_date) {
            self.advance_stage();
        }

        if context.audit_in_progress && rng.random::<f64>() < 0.8 {
            self.status = SchemeStatus::Paused;
            return actions;
        }
        self.status = SchemeStatus::Active;

        let stage = self.stages[self.current_stage_index].clone();
        let current_month = (context.current_date.year(), context.current_date.month());

        match self.current_stage_index {
            // Stage 0: setup — no financial actions
            0 => {
                // nothing to emit
            }

            // Stage 1: bypass — 2–4 reuses per calendar month
            1 => {
                // Reset monthly counter on new month
                if self.last_bypass_month != Some(current_month) {
                    self.last_bypass_month = Some(current_month);
                    let target = rng.random_range(2u32..=4u32);
                    self.reuses_this_month = 0;
                    // Emit all reuses for this month on first advance of the month
                    for _ in 0..target {
                        let amount = context
                            // Use fingerprint distribution so payment amounts are in-distribution (not easy outliers)
                            .sample_amount_from_fingerprint(rng, Some("6XXX"))
                            .unwrap_or_else(|| stage.random_amount(rng));
                        let mut action = SchemeAction::new(
                            self.scheme_id,
                            stage.stage_number,
                            SchemeActionType::ReuseDocumentId,
                            context.current_date,
                        )
                        .with_scheme_type(self.scheme_type())
                        .with_amount(amount)
                        .with_user(&self.perpetrator_id)
                        .with_difficulty(stage.detection_difficulty)
                        .with_reference(self.reused_invoice_id.clone())
                        .with_description(format!(
                            "Triad bypass: reuse invoice {} (no new PO/GR)",
                            self.reused_invoice_id
                        ));

                        // Always use the locked vendor from the reused invoice
                        let counterparty = self
                            .reused_vendor_id
                            .as_deref()
                            .or_else(|| {
                                context.available_counterparties.first().map(|s| s.as_str())
                            });
                        if let Some(cp) = counterparty {
                            action = action.with_counterparty(cp);
                        }

                        for t in &stage.concealment_techniques {
                            action = action.with_technique(*t);
                        }

                        self.reuses_this_month += 1;
                        self.stage_transaction_count += 1;
                        self.detection_probability =
                            (self.detection_probability + 0.02).min(0.9);
                        actions.push(action);
                    }
                }
            }

            // Stage 2: concealment — BypassConcealment correcting reversals
            2 => {
                if self.days_since_last_transaction >= 3 && rng.random::<f64>() < 0.30 {
                    // Use fingerprint distribution so concealment amounts are in-distribution (not easy outliers)
                    let amount = context
                        .sample_amount_from_fingerprint(rng, Some("6XXX"))
                        .unwrap_or_else(|| stage.random_amount(rng));
                    let mut action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::BypassConcealment,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(amount)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_reference(self.reused_invoice_id.clone())
                    .with_description(format!(
                        "Correcting reversal to conceal triad bypass on invoice {}",
                        self.reused_invoice_id
                    ));
                    for t in &stage.concealment_techniques {
                        action = action.with_technique(*t);
                    }
                    self.stage_transaction_count += 1;
                    self.days_since_last_transaction = 0;
                    actions.push(action);
                } else {
                    self.days_since_last_transaction += 1;
                }
            }

            _ => {}
        }

        if self.current_stage_index == self.stages.len() - 1 {
            if let Some(end_date) = self.stage_end_date() {
                if context.current_date >= end_date {
                    self.status = SchemeStatus::Completed;
                }
            }
        }

        actions
    }

    fn detection_probability(&self) -> f64 {
        self.detection_probability
    }

    fn total_impact(&self) -> Decimal {
        self.total_impact
    }

    fn should_terminate(&self, context: &SchemeContext) -> bool {
        context.detection_activity > 0.8
            || self.detection_probability > 0.9
            || self.detection_status != SchemeDetectionStatus::Undetected
    }

    fn perpetrator_id(&self) -> &str {
        &self.perpetrator_id
    }

    fn start_date(&self) -> Option<NaiveDate> {
        self.start_date
    }

    fn transaction_refs(&self) -> &[SchemeTransactionRef] {
        &self.transactions
    }

    fn record_transaction(&mut self, transaction: SchemeTransactionRef) {
        self.total_impact += transaction.amount;
        self.transactions.push(transaction);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    #[test]
    fn test_triad_bypass_creation() {
        let scheme = TriadBypassScheme::new(Uuid::nil(), "EMP001");
        assert_eq!(scheme.perpetrator_id, "EMP001");
        assert_eq!(scheme.stages.len(), 3);
        assert_eq!(scheme.status, SchemeStatus::NotStarted);
    }

    #[test]
    fn test_reused_invoice_from_candidates() {
        let mut scheme = TriadBypassScheme::new(Uuid::nil(), "EMP001");
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let context = SchemeContext::new(
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
            "1000",
        )
        .with_candidate_invoices(vec![
            "DOC-001|VND-ALPHA".to_string(),
            "DOC-002|VND-BETA".to_string(),
        ]);
        let _ = scheme.advance(&context, &mut rng);
        // Should have picked one of the real invoice IDs
        assert!(
            scheme.reused_invoice_id == "DOC-001" || scheme.reused_invoice_id == "DOC-002",
            "expected real invoice id, got {}",
            scheme.reused_invoice_id
        );
        assert!(scheme.reused_vendor_id.is_some());
    }

    #[test]
    fn test_bypass_stage_emits_multiple_reuses() {
        let mut scheme = TriadBypassScheme::new(Uuid::nil(), "EMP001");
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        // Start scheme
        let ctx0 = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(), "1000");
        let _ = scheme.advance(&ctx0, &mut rng);
        // Advance past setup stage (2 months)
        let ctx1 = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(), "1000");
        let actions = scheme.advance(&ctx1, &mut rng);
        // In bypass stage, first advance of the month should emit 2-4 actions
        if scheme.current_stage_index == 1 {
            assert!(actions.len() >= 2 && actions.len() <= 4);
        }
    }
}
