//! Inventory manipulation (balance-sheet fraud).
//!
//! The perpetrator overstates inventory via fictitious goods receipts, then
//! writes off the "missing" inventory as spoilage/shrinkage.
//!
//! Stages:
//!   0 — Inflation (4–8 months): Dr 370000 (Inventory), Cr 603000 (COGS reversal).
//!   1 — Siphoning (2–4 months): No JE — physical theft (off-books).
//!   2 — Write-off (1–2 months): Dr 685000 (Shrinkage), Cr 370000 (Inventory).

use chrono::NaiveDate;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryManipulationScheme {
    pub scheme_id: Uuid,
    pub perpetrator_id: String,
    pub start_date: Option<NaiveDate>,
    current_stage_index: usize,
    stages: Vec<SchemeStage>,
    transactions: Vec<SchemeTransactionRef>,
    total_impact: Decimal,
    inflated_total: Decimal,
    status: SchemeStatus,
    detection_status: SchemeDetectionStatus,
    detection_probability: f64,
    stage_transaction_count: u32,
    days_since_last_transaction: u32,
}

impl InventoryManipulationScheme {
    pub fn new(scheme_id: Uuid, perpetrator_id: impl Into<String>) -> Self {
        let stages = vec![
            SchemeStage::new(1, "inflation", 6, (dec!(500), dec!(5000)), (3, 8), AnomalyDetectionDifficulty::Hard)
                .with_description("Fictitious goods receipts to inflate inventory"),
            SchemeStage::new(2, "siphoning", 3, (dec!(0), dec!(0)), (0, 0), AnomalyDetectionDifficulty::Expert)
                .with_description("Physical inventory removed (no JE)"),
            SchemeStage::new(3, "write_off", 2, (dec!(500), dec!(5000)), (1, 3), AnomalyDetectionDifficulty::Moderate)
                .with_description("Write off discrepancy as spoilage/shrinkage")
                .with_technique(ConcealmentTechnique::AccountMisclassification),
        ];

        Self {
            scheme_id,
            perpetrator_id: perpetrator_id.into(),
            start_date: None,
            current_stage_index: 0,
            stages,
            transactions: Vec::new(),
            total_impact: Decimal::ZERO,
            inflated_total: Decimal::ZERO,
            status: SchemeStatus::NotStarted,
            detection_status: SchemeDetectionStatus::Undetected,
            detection_probability: 0.0,
            stage_transaction_count: 0,
            days_since_last_transaction: 0,
        }
    }

    fn stage_end_date(&self) -> Option<NaiveDate> {
        self.start_date.map(|start| {
            let total_months: u32 = self.stages[..=self.current_stage_index]
                .iter()
                .map(|s| s.duration_months)
                .sum();
            start + chrono::Months::new(total_months)
        })
    }

    fn should_advance_stage(&self, current_date: NaiveDate) -> bool {
        if let Some(end) = self.stage_end_date() {
            current_date >= end && self.current_stage_index < self.stages.len() - 1
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

impl FraudScheme for InventoryManipulationScheme {
    fn scheme_type(&self) -> SchemeType { SchemeType::InventoryManipulation }
    fn scheme_id(&self) -> Uuid { self.scheme_id }
    fn current_stage(&self) -> &SchemeStage { &self.stages[self.current_stage_index] }
    fn current_stage_number(&self) -> u32 { self.stages[self.current_stage_index].stage_number }
    fn stages(&self) -> &[SchemeStage] { &self.stages }
    fn status(&self) -> SchemeStatus { self.status }
    fn detection_status(&self) -> SchemeDetectionStatus { self.detection_status }

    fn advance(&mut self, context: &SchemeContext, rng: &mut dyn rand::RngCore) -> Vec<SchemeAction> {
        let mut actions = Vec::new();

        if self.status == SchemeStatus::NotStarted {
            self.start_date = Some(context.current_date);
            self.status = SchemeStatus::Active;
        }

        if self.should_advance_stage(context.current_date) {
            self.advance_stage();
        }

        if context.audit_in_progress && rng.random::<f64>() < 0.85 {
            self.status = SchemeStatus::Paused;
            return actions;
        }
        self.status = SchemeStatus::Active;

        let stage = self.stages[self.current_stage_index].clone();

        match self.current_stage_index {
            0 => {
                // Inflation: fictitious goods receipts
                let target_count = stage.random_transaction_count(rng);
                let should_transact = self.stage_transaction_count < target_count
                    && self.days_since_last_transaction >= 5
                    && rng.random::<f64>() < 0.25;

                if should_transact {
                    let amount = context
                        .sample_amount_from_fingerprint(rng, Some("3XXX"))
                        .unwrap_or_else(|| stage.random_amount(rng));

                    let action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::FictitiousGoodsReceipt,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(amount)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_description(format!("Fictitious goods receipt #{}", self.stage_transaction_count + 1));

                    self.inflated_total += amount;
                    self.total_impact += amount;
                    self.stage_transaction_count += 1;
                    self.days_since_last_transaction = 0;
                    actions.push(action);
                } else {
                    self.days_since_last_transaction += 1;
                }
            }
            1 => {
                // Siphoning: no JE (physical theft, off-books)
            }
            2 => {
                // Write-off: shrinkage entries to clear inflated inventory
                let target_count = stage.random_transaction_count(rng);
                let should_transact = self.stage_transaction_count < target_count
                    && self.days_since_last_transaction >= 3
                    && rng.random::<f64>() < 0.4;

                if should_transact {
                    let amount = if self.inflated_total > Decimal::ZERO {
                        let remaining_writeoffs = target_count.saturating_sub(self.stage_transaction_count).max(1);
                        self.inflated_total / Decimal::from(remaining_writeoffs)
                    } else {
                        stage.random_amount(rng)
                    };

                    let action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::InventoryWriteOff,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(amount)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_description(format!("Inventory write-off (shrinkage) #{}", self.stage_transaction_count + 1));

                    self.inflated_total = (self.inflated_total - amount).max(Decimal::ZERO);
                    self.stage_transaction_count += 1;
                    self.days_since_last_transaction = 0;
                    actions.push(action);
                } else {
                    self.days_since_last_transaction += 1;
                }

                if let Some(end) = self.stage_end_date() {
                    if context.current_date >= end {
                        self.status = SchemeStatus::Completed;
                    }
                }
            }
            _ => {}
        }

        self.detection_probability = (self.detection_probability + 0.01).min(0.7);
        actions
    }

    fn detection_probability(&self) -> f64 { self.detection_probability }
    fn total_impact(&self) -> Decimal { self.total_impact }
    fn should_terminate(&self, context: &SchemeContext) -> bool {
        context.detection_activity > 0.7 || self.detection_status != SchemeDetectionStatus::Undetected
    }
    fn perpetrator_id(&self) -> &str { &self.perpetrator_id }
    fn start_date(&self) -> Option<NaiveDate> { self.start_date }
    fn transaction_refs(&self) -> &[SchemeTransactionRef] { &self.transactions }
    fn record_transaction(&mut self, transaction: SchemeTransactionRef) {
        self.total_impact += transaction.amount;
        self.transactions.push(transaction);
    }
}
