//! Circular cash flow (single-entity journal variant).
//!
//! The perpetrator creates a cycle of 3 manual JEs that move cash through
//! suspense accounts, making it appear a receivable has been collected.
//!
//! Stages:
//!   0 — Fake collection (1 day): Dr 512000 (Bank), Cr 471000 (Suspense).
//!   1 — AR clearance (1–3 days later): Dr 471000 (Suspense), Cr 411000 (AR).
//!   2 — Reversal (15–30 days later): Dr 654000 (Bad Debt), Cr 512000 (Bank).
//!
//! Each cycle instance generates all 3 JEs in a single advance call,
//! scheduled at appropriate offsets. Multiple cycles may run per scheme instance.

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
pub struct CircularCashFlowScheme {
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
    cycles_completed: u32,
    max_cycles: u32,
    days_since_last_cycle: u32,
}

impl CircularCashFlowScheme {
    pub fn new(scheme_id: Uuid, perpetrator_id: impl Into<String>) -> Self {
        let stages = vec![
            SchemeStage::new(1, "operation", 8, (dec!(2000), dec!(30000)), (2, 5), AnomalyDetectionDifficulty::Hard)
                .with_description("Execute fake-collection → AR-clearance → bad-debt cycles")
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
            status: SchemeStatus::NotStarted,
            detection_status: SchemeDetectionStatus::Undetected,
            detection_probability: 0.0,
            cycles_completed: 0,
            max_cycles: 4,
            days_since_last_cycle: 0,
        }
    }
}

impl FraudScheme for CircularCashFlowScheme {
    fn scheme_type(&self) -> SchemeType { SchemeType::CircularCashFlow }
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

        if context.audit_in_progress && rng.random::<f64>() < 0.9 {
            self.status = SchemeStatus::Paused;
            return actions;
        }
        self.status = SchemeStatus::Active;

        if self.cycles_completed >= self.max_cycles {
            self.status = SchemeStatus::Completed;
            return actions;
        }

        // Start a new cycle roughly once per 45-60 days.
        let should_cycle = self.days_since_last_cycle >= 30 && rng.random::<f64>() < 0.15;

        if !should_cycle {
            self.days_since_last_cycle += 1;
            return actions;
        }

        let stage = self.stages[0].clone();
        let amount = context
            .sample_amount_from_fingerprint(rng, Some("4XXX"))
            .unwrap_or_else(|| stage.random_amount(rng));

        let cycle_ref = format!(
            "CCF-{:06X}-{:02}",
            (self.scheme_id.as_u128() & 0xFFFFFF) as u32,
            self.cycles_completed
        );

        // Step 1: Fake cash receipt (Dr Bank, Cr Suspense) — today.
        let step1 = SchemeAction::new(
            self.scheme_id,
            stage.stage_number,
            SchemeActionType::FakeCashReceipt,
            context.current_date,
        )
        .with_scheme_type(self.scheme_type())
        .with_amount(amount)
        .with_user(&self.perpetrator_id)
        .with_difficulty(stage.detection_difficulty)
        .with_reference(&cycle_ref)
        .with_description(format!("Fake cash receipt cycle #{}", self.cycles_completed + 1));
        actions.push(step1);

        // Step 2: AR clearance (Dr Suspense, Cr AR) — 1-3 days later.
        let ar_delay = rng.random_range(1i64..=3);
        let step2 = SchemeAction::new(
            self.scheme_id,
            stage.stage_number,
            SchemeActionType::ClearARViaSuspense,
            context.current_date + chrono::Duration::days(ar_delay),
        )
        .with_scheme_type(self.scheme_type())
        .with_amount(amount)
        .with_user(&self.perpetrator_id)
        .with_difficulty(stage.detection_difficulty)
        .with_reference(&cycle_ref)
        .with_description(format!("AR clearance via suspense cycle #{}", self.cycles_completed + 1));
        actions.push(step2);

        // Step 3: Concealed reversal (Dr Bad Debt, Cr Bank) — 15-30 days later.
        let reversal_delay = rng.random_range(15i64..=30);
        let step3 = SchemeAction::new(
            self.scheme_id,
            stage.stage_number,
            SchemeActionType::ConcealAsBadDebt,
            context.current_date + chrono::Duration::days(reversal_delay),
        )
        .with_scheme_type(self.scheme_type())
        .with_amount(amount)
        .with_user(&self.perpetrator_id)
        .with_difficulty(AnomalyDetectionDifficulty::Moderate)
        .with_reference(&cycle_ref)
        .with_technique(ConcealmentTechnique::AccountMisclassification)
        .with_description(format!("Bad debt write-off concealment cycle #{}", self.cycles_completed + 1));
        actions.push(step3);

        self.total_impact += amount;
        self.cycles_completed += 1;
        self.days_since_last_cycle = 0;
        self.detection_probability = (self.detection_probability + 0.05).min(0.8);

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
