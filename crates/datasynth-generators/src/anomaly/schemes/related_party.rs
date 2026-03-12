//! Related-party transaction abuse.
//!
//! A manager approves transactions with a vendor they have an undisclosed
//! financial interest in. Amounts are within normal ranges — detection requires
//! cross-referencing JE metadata with master data and entity graph traversal.
//!
//! Stages:
//!   0 — Setup (1 month): Register or take over a vendor whose bank account links to perpetrator.
//!   1 — Operation (9 months): Normal-looking procurement JEs approved by perpetrator.
//!   2 — Escalation (4 months): Volume increases but individual amounts stay below thresholds.

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
pub struct RelatedPartyScheme {
    pub scheme_id: Uuid,
    pub perpetrator_id: String,
    pub related_vendor_id: String,
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
}

impl RelatedPartyScheme {
    pub fn new(
        scheme_id: Uuid,
        perpetrator_id: impl Into<String>,
        related_vendor_id: impl Into<String>,
    ) -> Self {
        let stages = vec![
            SchemeStage::new(1, "setup", 1, (dec!(0), dec!(0)), (0, 1), AnomalyDetectionDifficulty::Expert)
                .with_description("Register related vendor or link bank account"),
            SchemeStage::new(2, "operation", 9, (dec!(1000), dec!(20000)), (2, 5), AnomalyDetectionDifficulty::Expert)
                .with_description("Normal-looking procurement to related vendor")
                .with_technique(ConcealmentTechnique::Collusion),
            SchemeStage::new(3, "escalation", 4, (dec!(1000), dec!(20000)), (3, 8), AnomalyDetectionDifficulty::Hard)
                .with_description("Increasing volume to related vendor")
                .with_technique(ConcealmentTechnique::Collusion),
        ];

        Self {
            scheme_id,
            perpetrator_id: perpetrator_id.into(),
            related_vendor_id: related_vendor_id.into(),
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

impl FraudScheme for RelatedPartyScheme {
    fn scheme_type(&self) -> SchemeType { SchemeType::RelatedPartyAbuse }
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

        if context.audit_in_progress && rng.random::<f64>() < 0.7 {
            self.status = SchemeStatus::Paused;
            return actions;
        }
        self.status = SchemeStatus::Active;

        let stage = self.stages[self.current_stage_index].clone();

        match self.current_stage_index {
            0 => {
                // Setup: vendor registration — emitted as a CreateFictitiousVendor action.
                if self.stage_transaction_count == 0 {
                    let action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::CreateFictitiousVendor,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_counterparty(&self.related_vendor_id)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_description("Register related-party vendor (shared bank account with perpetrator)");

                    self.stage_transaction_count += 1;
                    actions.push(action);
                }
            }
            1 | 2 => {
                // Procurement: normal-looking JEs to the related vendor.
                let target_count = stage.random_transaction_count(rng);
                let should_transact = self.stage_transaction_count < target_count
                    && self.days_since_last_transaction >= 5
                    && rng.random::<f64>() < 0.25;

                if should_transact {
                    let amount = context
                        .sample_amount_from_fingerprint(rng, Some("6XXX"))
                        .unwrap_or_else(|| stage.random_amount(rng));

                    let action = SchemeAction::new(
                        self.scheme_id,
                        stage.stage_number,
                        SchemeActionType::RelatedPartyProcurement,
                        context.current_date,
                    )
                    .with_scheme_type(self.scheme_type())
                    .with_amount(amount)
                    .with_counterparty(&self.related_vendor_id)
                    .with_user(&self.perpetrator_id)
                    .with_difficulty(stage.detection_difficulty)
                    .with_description(format!(
                        "Procurement to related vendor {} (stage {})",
                        self.related_vendor_id, stage.stage_number
                    ));

                    self.total_impact += amount;
                    self.stage_transaction_count += 1;
                    self.days_since_last_transaction = 0;
                    actions.push(action);
                } else {
                    self.days_since_last_transaction += 1;
                }

                if self.current_stage_index == 2 {
                    if let Some(end) = self.stage_end_date() {
                        if context.current_date >= end {
                            self.status = SchemeStatus::Completed;
                        }
                    }
                }
            }
            _ => {}
        }

        self.detection_probability = (self.detection_probability + 0.005).min(0.5);
        actions
    }

    fn detection_probability(&self) -> f64 { self.detection_probability }
    fn total_impact(&self) -> Decimal { self.total_impact }
    fn should_terminate(&self, context: &SchemeContext) -> bool {
        context.detection_activity > 0.6 || self.detection_status != SchemeDetectionStatus::Undetected
    }
    fn perpetrator_id(&self) -> &str { &self.perpetrator_id }
    fn start_date(&self) -> Option<NaiveDate> { self.start_date }
    fn transaction_refs(&self) -> &[SchemeTransactionRef] { &self.transactions }
    fn record_transaction(&mut self, transaction: SchemeTransactionRef) {
        self.total_impact += transaction.amount;
        self.transactions.push(transaction);
    }
}
