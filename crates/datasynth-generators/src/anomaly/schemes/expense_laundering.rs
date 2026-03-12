//! Expense laundering (entropy fan-out) fraud scheme.
//!
//! Fictitious micro-expenses are submitted under **existing vendor identities** so the
//! resulting journal entries are indistinguishable from legitimate P2P activity.
//!
//! v3.0 changes (anti-leak hardening):
//! - Vendors are sampled from `context.available_counterparties` (real master data) —
//!   no more `SHL-` prefixed shell vendor IDs.
//! - Invoice references use the same `VI:INV-{hash}-{n}` format as the P2P generator.
//! - Stage 0 is a silent setup phase (no JE emitted): it picks target vendors.
//! - Stage 1: every micro-invoice emits a **two-step wash**:
//!     Step 1 → `SuspenseStaging`  (Dr Expense + VAT, Cr AP)
//!     Step 2 → `SuspenseClearance` (Dr AP, Cr Bank)
//!   Vendors are rotated at the midpoint.
//!   Amount scaling: base_max = max(50.0, annual_revenue / 100_000.0);
//!                   amount ∈ [base_max × 0.01, base_max × 0.10]
//! - Stage 2: one `ShellVendorSettlement` per vendor + single `ConsolidationWire`.

use chrono::{Duration, NaiveDate};
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

/// Expense laundering scheme: micro-expenses to shell vendors via suspense wash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpenseLaunderingScheme {
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
    /// Target vendor IDs sampled from existing master data.
    target_vendors: Vec<String>,
    /// Whether target vendors have been selected from available counterparties.
    vendors_selected: bool,
    /// Index into target_vendors for current rotation.
    current_vendor_index: usize,
    /// Whether Stage 2 settlement has been emitted.
    settlement_emitted: bool,
}

impl ExpenseLaunderingScheme {
    pub fn new(scheme_id: Uuid, perpetrator_id: impl Into<String>) -> Self {
        let stages = vec![
            SchemeStage::new(
                1,
                "shell_vendors",
                2,
                (dec!(0), dec!(0)),
                (0, 2),
                AnomalyDetectionDifficulty::Expert,
            )
            .with_description("Create network of unverified shell vendors")
            .with_technique(ConcealmentTechnique::FalseDocumentation)
            .with_technique(ConcealmentTechnique::Collusion),
            SchemeStage::new(
                2,
                "micro_expenses",
                8,
                (dec!(50), dec!(950)),
                (10, 50),
                AnomalyDetectionDifficulty::Hard,
            )
            .with_description("Micro-expenses via suspense-account two-step wash to shell vendors")
            .with_technique(ConcealmentTechnique::TransactionSplitting)
            .with_technique(ConcealmentTechnique::AccountMisclassification),
            SchemeStage::new(
                3,
                "settlement",
                2,
                (dec!(500), dec!(50000)),
                (1, 3),
                AnomalyDetectionDifficulty::Moderate,
            )
            .with_description("Settle AP to each shell vendor + consolidation wire")
            .with_technique(ConcealmentTechnique::TimingExploitation)
            .with_technique(ConcealmentTechnique::DataAlteration),
        ];

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
            target_vendors: Vec::new(),
            vendors_selected: false,
            current_vendor_index: 0,
            settlement_emitted: false,
        }
    }

    /// Computes a micro-expense amount. When fingerprint configs exist, samples from 6XXX
    /// and scales by 0.01–0.10 so micro amounts stay in-distribution (not easy outliers).
    /// Otherwise uses annual revenue scaling: base_max = max(50, annual_revenue/100k), amount ∈ [base_max×0.01, base_max×0.10].
    fn sample_micro_amount(&self, context: &SchemeContext, rng: &mut dyn rand::RngCore) -> Decimal {
        if let Some(base) = context.sample_amount_from_fingerprint(rng, Some("6XXX")) {
            let factor = Decimal::from_f64_retain(rng.random_range(0.01..=0.10)).unwrap_or(dec!(0.05));
            return (base * factor).round_dp(2).max(dec!(1));
        }
        let base_max: f64 = context
            .annual_revenue
            .and_then(|r: Decimal| r.try_into().ok())
            .map(|r: f64| (r / 100_000.0_f64).max(50.0))
            .unwrap_or(50.0);
        let lo = base_max * 0.01;
        let hi = base_max * 0.10;
        let v = rng.random_range(lo..=hi);
        Decimal::from_f64_retain(v)
            .unwrap_or(dec!(50))
            .round_dp(2)
            .max(dec!(1))
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

impl FraudScheme for ExpenseLaunderingScheme {
    fn scheme_type(&self) -> SchemeType {
        SchemeType::ExpenseLaundering
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

        match self.current_stage_index {
            // Stage 0: silently select target vendors from existing master data.
            // No JE is emitted — this avoids leaking identifiable shell vendor IDs.
            0 => {
                if !self.vendors_selected {
                    let n_vendors = 4usize;
                    if context.available_counterparties.len() >= n_vendors {
                        // Deterministic shuffle seeded from scheme_id so picks are reproducible.
                        let mut candidates = context.available_counterparties.clone();
                        let seed = self.scheme_id.as_u128() as u64;
                        let mut pick_rng = <rand_chacha::ChaCha8Rng as rand::SeedableRng>::seed_from_u64(seed);
                        use rand::seq::SliceRandom;
                        candidates.shuffle(&mut pick_rng);
                        self.target_vendors = candidates.into_iter().take(n_vendors).collect();
                    } else {
                        // Fallback: use whatever counterparties are available.
                        self.target_vendors = context.available_counterparties.clone();
                    }
                    self.vendors_selected = true;
                    self.stage_transaction_count += 1;
                    self.detection_probability = (self.detection_probability + 0.005).min(0.9);
                }
            }

            // Stage 1: two-step wash per micro-invoice using existing vendors
            1 => {
                let target_count = stage.random_transaction_count(rng);
                let should_transact = self.stage_transaction_count < target_count
                    && self.days_since_last_transaction >= 1
                    && rng.random::<f64>() < 0.40;

                if should_transact {
                    let amount = self.sample_micro_amount(context, rng);

                    // Rotate vendor at midpoint of stage
                    if self.stage_transaction_count == target_count / 2
                        && !self.target_vendors.is_empty()
                    {
                        self.current_vendor_index =
                            (self.current_vendor_index + 1) % self.target_vendors.len();
                    }

                    let vendor = if self.target_vendors.is_empty() {
                        context.available_counterparties.first()
                            .cloned()
                            .unwrap_or_else(|| "V-000001".to_string())
                    } else {
                        self.target_vendors[self.current_vendor_index % self.target_vendors.len()]
                            .clone()
                    };

                    // Use a normal invoice reference format matching the P2P generator
                    let inv_ref = format!(
                        "VI:INV-{:08x}-{}",
                        self.scheme_id.as_u128() as u32,
                        self.stage_transaction_count
                    );

                    // Step 1: Invoice booking (Dr Expense + VAT, Cr AP)
                    let mut stage1_action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::SuspenseStaging,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(amount)
                    .with_counterparty(&vendor)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_reference(&inv_ref)
                    .with_description(format!(
                        "Vendor invoice: Dr Expense+VAT, Cr AP — vendor {}",
                        vendor
                    ));
                    for t in &stage.concealment_techniques {
                        stage1_action = stage1_action.with_technique(*t);
                    }
                    actions.push(stage1_action);

                    // Step 2: Payment (Dr AP, Cr Bank) — 1-2 days later
                    let clear_date = context.current_date
                        + chrono::Duration::days(rng.random_range(1i64..=2i64));
                    let mut stage2_action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::SuspenseClearance,
                        clear_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(amount)
                    .with_counterparty(&vendor)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_reference(&inv_ref)
                    .with_description(format!(
                        "Vendor payment: Dr AP, Cr Bank — vendor {}",
                        vendor
                    ));
                    for t in &stage.concealment_techniques {
                        stage2_action = stage2_action.with_technique(*t);
                    }
                    actions.push(stage2_action);

                    self.stage_transaction_count += 1;
                    self.days_since_last_transaction = 0;
                    self.detection_probability = (self.detection_probability + 0.01).min(0.9);
                } else {
                    self.days_since_last_transaction += 1;
                }
            }

            // Stage 2: Settlement per vendor + ConsolidationWire
            2 => {
                if !self.settlement_emitted {
                    for (idx, vendor_id) in self.target_vendors.clone().iter().enumerate() {
                        let settle_amount = context
                            .sample_amount_from_fingerprint(rng, Some("6XXX"))
                            .unwrap_or_else(|| stage.random_amount(rng));
                        let action_date =
                            context.current_date + Duration::days(idx as i64);
                        let action = SchemeAction::new(
                            self.scheme_id,
                            stage.stage_number,
                            SchemeActionType::ShellVendorSettlement,
                            action_date,
                        )
                        .with_scheme_type(self.scheme_type())
                        .with_amount(settle_amount)
                        .with_counterparty(vendor_id)
                        .with_user(&self.perpetrator_id)
                        .with_difficulty(stage.detection_difficulty)
                        .with_description(format!(
                            "Settle AP outstanding to vendor {} (Dr AP, Cr Bank)",
                            vendor_id
                        ))
                        .with_technique(ConcealmentTechnique::TimingExploitation);
                        actions.push(action);
                    }

                    // Single ConsolidationWire aggregating proceeds
                    let wire_candidate = context
                        // Use fingerprint distribution so wire amounts are in-distribution (not easy outliers)
                        .sample_amount_from_fingerprint(rng, Some("6XXX"))
                        .unwrap_or_else(|| stage.random_amount(rng));
                    let wire_amount = self.total_impact.max(wire_candidate);
                    let wire_action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::ConsolidationWire,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(wire_amount)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_description(
                        "Consolidation wire: aggregate proceeds (Dr 581000, Cr Bank)"
                            .to_string(),
                    )
                    .with_technique(ConcealmentTechnique::DataAlteration);
                    actions.push(wire_action);

                    self.settlement_emitted = true;
                    self.stage_transaction_count += 1;
                    self.detection_probability = (self.detection_probability + 0.03).min(0.9);
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
    fn test_vendors_selected_from_counterparties() {
        let mut scheme = ExpenseLaunderingScheme::new(Uuid::nil(), "EMP001");
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        let vendors: Vec<String> = (0..10).map(|i| format!("V-{:06}", i)).collect();
        let context = SchemeContext::new(
            NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(),
            "1000",
        )
        .with_counterparties(vendors.clone());
        let _ = scheme.advance(&context, &mut rng);
        assert!(!scheme.target_vendors.is_empty());
        for v in &scheme.target_vendors {
            assert!(
                vendors.contains(v),
                "target vendor {} should come from available counterparties",
                v
            );
            assert!(
                !v.starts_with("SHL-"),
                "vendor {} must not have SHL- prefix",
                v
            );
        }
    }

    #[test]
    fn test_stage0_emits_no_actions() {
        let mut scheme = ExpenseLaunderingScheme::new(Uuid::nil(), "EMP001");
        let mut rng = ChaCha8Rng::seed_from_u64(5);
        let vendors: Vec<String> = (0..10).map(|i| format!("V-{:06}", i)).collect();
        let context = SchemeContext::new(
            NaiveDate::from_ymd_opt(2024, 1, 5).unwrap(),
            "1000",
        )
        .with_counterparties(vendors);
        let actions = scheme.advance(&context, &mut rng);
        assert!(
            actions.is_empty(),
            "Stage 0 should not emit any journal-entry-producing actions"
        );
    }

    #[test]
    fn test_stage1_two_step_wash() {
        let mut scheme = ExpenseLaunderingScheme::new(Uuid::nil(), "EMP001");
        scheme.current_stage_index = 1;
        scheme.start_date = Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap());
        scheme.status = SchemeStatus::Active;
        scheme.target_vendors = vec!["V-000001".to_string(), "V-000002".to_string()];
        scheme.vendors_selected = true;
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        let context =
            SchemeContext::new(NaiveDate::from_ymd_opt(2024, 3, 5).unwrap(), "1000");
        let mut found_staging = false;
        let mut found_clearance = false;
        for _ in 0..50 {
            let actions = scheme.advance(&context, &mut rng);
            for a in &actions {
                if a.action_type == SchemeActionType::SuspenseStaging {
                    found_staging = true;
                    assert!(
                        !a.counterparty.as_ref().unwrap().starts_with("SHL-"),
                        "counterparty should not have SHL- prefix"
                    );
                }
                if a.action_type == SchemeActionType::SuspenseClearance {
                    found_clearance = true;
                }
            }
        }
        assert!(
            found_staging && found_clearance,
            "Both SuspenseStaging and SuspenseClearance must be emitted"
        );
    }

    #[test]
    fn test_reference_format_no_shl_prefix() {
        let mut scheme = ExpenseLaunderingScheme::new(Uuid::nil(), "EMP001");
        scheme.current_stage_index = 1;
        scheme.start_date = Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap());
        scheme.status = SchemeStatus::Active;
        scheme.target_vendors = vec!["V-000001".to_string()];
        scheme.vendors_selected = true;
        let mut rng = ChaCha8Rng::seed_from_u64(99);
        let context =
            SchemeContext::new(NaiveDate::from_ymd_opt(2024, 3, 5).unwrap(), "1000");
        for _ in 0..50 {
            let actions = scheme.advance(&context, &mut rng);
            for a in &actions {
                if let Some(ref r) = a.reference {
                    assert!(
                        r.starts_with("VI:INV-"),
                        "reference should use VI:INV- format, got {}",
                        r
                    );
                    assert!(
                        !r.contains("SHL"),
                        "reference must not contain SHL, got {}",
                        r
                    );
                }
            }
        }
    }

    #[test]
    fn test_amount_scaling_with_annual_revenue() {
        let mut scheme = ExpenseLaunderingScheme::new(Uuid::nil(), "EMP001");
        scheme.current_stage_index = 1;
        scheme.start_date = Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap());
        scheme.status = SchemeStatus::Active;
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let context = SchemeContext::new(
            NaiveDate::from_ymd_opt(2024, 3, 5).unwrap(),
            "1000",
        )
        .with_annual_revenue(dec!(10_000_000));
        // base_max = 10_000_000 / 100_000 = 100
        // amount ∈ [1.0, 10.0]
        let amount = scheme.sample_micro_amount(&context, &mut rng);
        assert!(
            amount >= dec!(1) && amount <= dec!(10) + dec!(1),
            "amount {} out of expected range for 10M revenue",
            amount
        );
    }
}
