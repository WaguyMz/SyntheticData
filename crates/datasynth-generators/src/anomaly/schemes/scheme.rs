//! Core fraud scheme trait and types.

use chrono::{Datelike, NaiveDate};
use rand::Rng;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use datasynth_core::models::{
    AnomalyDetectionDifficulty, ConcealmentTechnique, SchemeDetectionStatus, SchemeType,
};

/// A stage within a multi-stage fraud scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemeStage {
    /// Stage number (1-indexed).
    pub stage_number: u32,
    /// Name of the stage (e.g., "testing", "escalation").
    pub name: String,
    /// Description of what happens in this stage.
    pub description: String,
    /// Duration in months.
    pub duration_months: u32,
    /// Minimum transaction amount for this stage.
    pub amount_min: Decimal,
    /// Maximum transaction amount for this stage.
    pub amount_max: Decimal,
    /// Minimum number of transactions in this stage.
    pub transaction_count_min: u32,
    /// Maximum number of transactions in this stage.
    pub transaction_count_max: u32,
    /// Detection difficulty for this stage.
    pub detection_difficulty: AnomalyDetectionDifficulty,
    /// Concealment techniques typically used in this stage.
    pub concealment_techniques: Vec<ConcealmentTechnique>,
}

impl SchemeStage {
    /// Creates a new scheme stage.
    pub fn new(
        stage_number: u32,
        name: impl Into<String>,
        duration_months: u32,
        amount_range: (Decimal, Decimal),
        transaction_range: (u32, u32),
        difficulty: AnomalyDetectionDifficulty,
    ) -> Self {
        Self {
            stage_number,
            name: name.into(),
            description: String::new(),
            duration_months,
            amount_min: amount_range.0,
            amount_max: amount_range.1,
            transaction_count_min: transaction_range.0,
            transaction_count_max: transaction_range.1,
            detection_difficulty: difficulty,
            concealment_techniques: Vec::new(),
        }
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Adds a concealment technique.
    pub fn with_technique(mut self, technique: ConcealmentTechnique) -> Self {
        self.concealment_techniques.push(technique);
        self
    }

    /// Generates a random amount within the stage range.
    pub fn random_amount<R: Rng + ?Sized>(&self, rng: &mut R) -> Decimal {
        if self.amount_min == self.amount_max {
            return self.amount_min;
        }
        let min_f64: f64 = self.amount_min.try_into().unwrap_or(0.0);
        let max_f64: f64 = self.amount_max.try_into().unwrap_or(min_f64 + 1000.0);
        let value = rng.random_range(min_f64..=max_f64);
        Decimal::from_f64_retain(value).unwrap_or(self.amount_min)
    }

    /// Generates a random transaction count within the stage range.
    pub fn random_transaction_count<R: Rng + ?Sized>(&self, rng: &mut R) -> u32 {
        if self.transaction_count_min == self.transaction_count_max {
            return self.transaction_count_min;
        }
        rng.random_range(self.transaction_count_min..=self.transaction_count_max)
    }
}

/// Current status of a fraud scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SchemeStatus {
    /// Scheme has not started yet.
    #[default]
    NotStarted,
    /// Scheme is actively ongoing.
    Active,
    /// Scheme is temporarily paused (e.g., due to audit).
    Paused,
    /// Scheme has been terminated by perpetrator.
    Terminated,
    /// Scheme has been detected.
    Detected,
    /// Scheme has completed its full lifecycle.
    Completed,
}

/// Context provided to a scheme for decision-making.
#[derive(Debug, Clone)]
pub struct SchemeContext {
    /// Current date.
    pub current_date: NaiveDate,
    /// Whether an audit is currently in progress.
    pub audit_in_progress: bool,
    /// Recent detection activity level (0.0-1.0).
    pub detection_activity: f64,
    /// Available accounts for transactions.
    pub available_accounts: Vec<String>,
    /// Available vendors/customers.
    pub available_counterparties: Vec<String>,
    /// Available users who could be perpetrators.
    pub available_users: Vec<String>,
    /// Company code.
    pub company_code: String,
    /// Optional annual revenue for amount scaling (e.g. expense laundering).
    pub annual_revenue: Option<Decimal>,
    /// Candidate invoice refs for triad bypass ("document_id|vendor_id").
    pub candidate_invoice_ids: Vec<String>,
    /// Per-account-class amount configs from fingerprint so fraud amounts stay in-distribution (not easy outliers).
    pub fingerprint_amount_configs: Option<std::collections::HashMap<String, datasynth_core::AmountDistributionConfig>>,
}

impl SchemeContext {
    /// Creates a new scheme context.
    pub fn new(current_date: NaiveDate, company_code: impl Into<String>) -> Self {
        Self {
            current_date,
            audit_in_progress: false,
            detection_activity: 0.0,
            available_accounts: Vec::new(),
            available_counterparties: Vec::new(),
            available_users: Vec::new(),
            company_code: company_code.into(),
            annual_revenue: None,
            candidate_invoice_ids: Vec::new(),
            fingerprint_amount_configs: None,
        }
    }

    /// Sets audit in progress flag.
    pub fn with_audit(mut self, in_progress: bool) -> Self {
        self.audit_in_progress = in_progress;
        self
    }

    /// Sets detection activity level.
    pub fn with_detection_activity(mut self, level: f64) -> Self {
        self.detection_activity = level.clamp(0.0, 1.0);
        self
    }

    /// Sets available accounts.
    pub fn with_accounts(mut self, accounts: Vec<String>) -> Self {
        self.available_accounts = accounts;
        self
    }

    /// Sets available counterparties.
    pub fn with_counterparties(mut self, counterparties: Vec<String>) -> Self {
        self.available_counterparties = counterparties;
        self
    }

    /// Sets available users.
    pub fn with_users(mut self, users: Vec<String>) -> Self {
        self.available_users = users;
        self
    }

    /// Sets annual revenue for amount scaling.
    pub fn with_annual_revenue(mut self, revenue: Decimal) -> Self {
        self.annual_revenue = Some(revenue);
        self
    }

    /// Sets candidate invoice refs ("document_id|vendor_id") for triad bypass.
    pub fn with_candidate_invoices(mut self, ids: Vec<String>) -> Self {
        self.candidate_invoice_ids = ids;
        self
    }

    /// Sets per-class amount configs from fingerprint so scheme amounts are in-distribution (fraud not obvious outliers).
    pub fn with_fingerprint_amount_configs(
        mut self,
        configs: Option<std::collections::HashMap<String, datasynth_core::AmountDistributionConfig>>,
    ) -> Self {
        self.fingerprint_amount_configs = configs;
        self
    }

    /// Sample an amount from fingerprint-derived distribution for the given account class when available.
    /// Returns None if no fingerprint config; schemes then fall back to stage.random_amount().
    /// This keeps fraud amounts in-distribution so they are not trivial outliers.
    pub fn sample_amount_from_fingerprint<R: Rng + ?Sized>(
        &self,
        rng: &mut R,
        class: Option<&str>,
    ) -> Option<Decimal> {
        let configs = self.fingerprint_amount_configs.as_ref()?;
        let config = class
            .and_then(|c| configs.get(c))
            .or_else(|| configs.get("6XXX").or_else(|| configs.values().next()))?;
        let seed: u64 = rng.random();
        Some(datasynth_core::AmountSampler::with_config(seed, config.clone()).sample())
    }
}

/// An action generated by a scheme to be executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemeAction {
    /// Unique action ID.
    pub action_id: Uuid,
    /// Scheme ID this action belongs to.
    pub scheme_id: Uuid,
    /// Scheme type (for materialization to use typical accounts per pathology).
    #[serde(default)]
    pub scheme_type: Option<SchemeType>,
    /// Stage this action belongs to.
    pub stage: u32,
    /// Type of action.
    pub action_type: SchemeActionType,
    /// Target date for the action.
    pub target_date: NaiveDate,
    /// Amount involved (if applicable).
    pub amount: Option<Decimal>,
    /// Target account.
    pub target_account: Option<String>,
    /// Counterparty involved.
    pub counterparty: Option<String>,
    /// User to perform action.
    pub user_id: Option<String>,
    /// Description.
    pub description: String,
    /// Detection difficulty for this specific action.
    pub detection_difficulty: AnomalyDetectionDifficulty,
    /// Concealment techniques to apply.
    pub concealment_techniques: Vec<ConcealmentTechnique>,
    /// Whether this action has been executed.
    pub executed: bool,
    /// Company code in which this action should be executed.
    ///
    /// This is populated by the injector when schemes are advanced so that
    /// runtime materialization can create journal entries in the correct company.
    #[serde(default)]
    pub company_code: Option<String>,
    /// Optional reference (e.g. invoice number) to link related actions (invoice + payment).
    #[serde(default)]
    pub reference: Option<String>,
}

impl SchemeAction {
    /// Creates a new scheme action.
    pub fn new(
        scheme_id: Uuid,
        stage: u32,
        action_type: SchemeActionType,
        target_date: NaiveDate,
    ) -> Self {
        // Derive deterministic, unique action_id from scheme_id + stage + target_date
        // so that each action (same stage on different days) gets a distinct document_id.
        // Previously only scheme_id + stage was used, so all actions in a stage shared one id.
        let action_id = {
            let scheme_bytes = scheme_id.as_bytes();
            let stage_bytes = stage.to_le_bytes();
            let date_bytes = [
                target_date.year() as u8,
                (target_date.year() >> 8) as u8,
                target_date.month() as u8,
                target_date.day() as u8,
            ];
            let mut hash: u64 = 0xcbf29ce484222325;
            for &b in scheme_bytes
                .iter()
                .chain(stage_bytes.iter())
                .chain(date_bytes.iter())
            {
                hash ^= b as u64;
                hash = hash.wrapping_mul(0x100000001b3);
            }
            let bytes = hash.to_le_bytes();
            let mut uuid_bytes = [0u8; 16];
            uuid_bytes[..8].copy_from_slice(&bytes);
            uuid_bytes[8..16].copy_from_slice(&bytes);
            uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x40;
            uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
            Uuid::from_bytes(uuid_bytes)
        };
        Self {
            action_id,
            scheme_id,
            scheme_type: None,
            stage,
            action_type,
            target_date,
            amount: None,
            target_account: None,
            counterparty: None,
            user_id: None,
            description: String::new(),
            detection_difficulty: AnomalyDetectionDifficulty::Moderate,
            concealment_techniques: Vec::new(),
            executed: false,
            company_code: None,
            reference: None,
        }
    }

    /// Sets the reference (e.g. invoice number for P2P linking).
    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    /// Sets the scheme type (for materialization to pick typical accounts).
    pub fn with_scheme_type(mut self, scheme_type: SchemeType) -> Self {
        self.scheme_type = Some(scheme_type);
        self
    }

    /// Sets the amount.
    pub fn with_amount(mut self, amount: Decimal) -> Self {
        self.amount = Some(amount);
        self
    }

    /// Sets the target account.
    pub fn with_account(mut self, account: impl Into<String>) -> Self {
        self.target_account = Some(account.into());
        self
    }

    /// Sets the counterparty.
    pub fn with_counterparty(mut self, counterparty: impl Into<String>) -> Self {
        self.counterparty = Some(counterparty.into());
        self
    }

    /// Sets the user.
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user_id = Some(user.into());
        self
    }

    /// Sets the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Sets the detection difficulty.
    pub fn with_difficulty(mut self, difficulty: AnomalyDetectionDifficulty) -> Self {
        self.detection_difficulty = difficulty;
        self
    }

    /// Adds a concealment technique.
    pub fn with_technique(mut self, technique: ConcealmentTechnique) -> Self {
        self.concealment_techniques.push(technique);
        self
    }

    /// Marks the action as executed.
    pub fn mark_executed(&mut self) {
        self.executed = true;
    }
}

/// Type of action within a scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SchemeActionType {
    /// Create a fraudulent journal entry.
    CreateFraudulentEntry,
    /// Create a fraudulent payment.
    CreateFraudulentPayment,
    /// Create a fictitious vendor.
    CreateFictitiousVendor,
    /// Inflate an invoice amount.
    InflateInvoice,
    /// Make a kickback payment.
    MakeKickbackPayment,
    /// Manipulate revenue recognition.
    ManipulateRevenue,
    /// Defer expense recognition.
    DeferExpense,
    /// Release reserves.
    ReleaseReserves,
    /// Create channel stuffing transaction.
    ChannelStuff,
    /// Conceal prior fraud.
    Conceal,
    /// Cover up tracks.
    CoverUp,
    // RIP-GNN pathology lab action types
    /// Reuse an old document ID (triad bypass).
    ReuseDocumentId,
    /// Create a ghost employee.
    CreateGhostEmployee,
    /// Create a ghost bank account.
    CreateGhostBankAccount,
    /// Create a micro-expense (expense laundering).
    CreateMicroExpense,
    /// Create a shell vendor.
    CreateShellVendor,
    /// Intercompany round-trip transaction.
    IntercompanyRoundTrip,
    /// Transfer inventory to ghost location.
    InventoryTransferToGhostLocation,
    // Expense laundering
    /// Stage expense to suspense (Dr Expense + VAT, Cr 471000).
    SuspenseStaging,
    /// Clear suspense to AP (Dr 471000, Cr AP).
    SuspenseClearance,
    /// Settle shell vendor (payment).
    ShellVendorSettlement,
    /// Consolidation wire aggregating shell proceeds.
    ConsolidationWire,
    // Shadow payroll
    /// Ghost hire admin (no financial JE).
    GhostHire,
    /// Monthly payroll run.
    MonthlyPayroll,
    /// Ghost termination admin (no financial JE).
    GhostTermination,
    /// Final settlement (last payroll / severance).
    FinalSettlement,
    // Triad bypass
    /// Concealment correcting reversals.
    BypassConcealment,
}

/// Trait for fraud schemes.
pub trait FraudScheme: Send + Sync {
    /// Returns the scheme type.
    fn scheme_type(&self) -> SchemeType;

    /// Returns the unique scheme ID.
    fn scheme_id(&self) -> Uuid;

    /// Returns the current stage.
    fn current_stage(&self) -> &SchemeStage;

    /// Returns the current stage number.
    fn current_stage_number(&self) -> u32;

    /// Returns all stages.
    fn stages(&self) -> &[SchemeStage];

    /// Returns the current status.
    fn status(&self) -> SchemeStatus;

    /// Returns the detection status.
    fn detection_status(&self) -> SchemeDetectionStatus;

    /// Advances the scheme and returns actions to execute.
    fn advance(
        &mut self,
        context: &SchemeContext,
        rng: &mut dyn rand::RngCore,
    ) -> Vec<SchemeAction>;

    /// Returns the cumulative detection probability.
    fn detection_probability(&self) -> f64;

    /// Returns the total financial impact so far.
    fn total_impact(&self) -> Decimal;

    /// Returns whether the scheme should terminate.
    fn should_terminate(&self, context: &SchemeContext) -> bool;

    /// Gets the perpetrator ID.
    fn perpetrator_id(&self) -> &str;

    /// Gets the start date.
    fn start_date(&self) -> Option<NaiveDate>;

    /// Gets all transaction references.
    fn transaction_refs(&self) -> &[crate::anomaly::schemes::scheme::SchemeTransactionRef];

    /// Records a transaction.
    fn record_transaction(&mut self, transaction: SchemeTransactionRef);
}

/// Reference to a transaction within a scheme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemeTransactionRef {
    /// Document ID.
    pub document_id: String,
    /// Transaction date.
    pub date: NaiveDate,
    /// Transaction amount.
    pub amount: Decimal,
    /// Stage number.
    pub stage: u32,
    /// Anomaly ID if labeled.
    pub anomaly_id: Option<String>,
    /// Action ID that generated this transaction.
    pub action_id: Option<Uuid>,
}

impl SchemeTransactionRef {
    /// Creates a new transaction reference.
    pub fn new(
        document_id: impl Into<String>,
        date: NaiveDate,
        amount: Decimal,
        stage: u32,
    ) -> Self {
        Self {
            document_id: document_id.into(),
            date,
            amount,
            stage,
            anomaly_id: None,
            action_id: None,
        }
    }

    /// Sets the anomaly ID.
    pub fn with_anomaly(mut self, anomaly_id: impl Into<String>) -> Self {
        self.anomaly_id = Some(anomaly_id.into());
        self
    }

    /// Sets the action ID.
    pub fn with_action(mut self, action_id: Uuid) -> Self {
        self.action_id = Some(action_id);
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn test_scheme_stage() {
        let stage = SchemeStage::new(
            1,
            "testing",
            2,
            (dec!(100), dec!(500)),
            (2, 5),
            AnomalyDetectionDifficulty::Hard,
        )
        .with_description("Initial testing phase")
        .with_technique(ConcealmentTechnique::TransactionSplitting);

        assert_eq!(stage.stage_number, 1);
        assert_eq!(stage.name, "testing");
        assert_eq!(stage.duration_months, 2);
        assert_eq!(stage.detection_difficulty, AnomalyDetectionDifficulty::Hard);
        assert_eq!(stage.concealment_techniques.len(), 1);
    }

    #[test]
    fn test_scheme_stage_random_amount() {
        use rand::SeedableRng;
        use rand_chacha::ChaCha8Rng;

        let stage = SchemeStage::new(
            1,
            "test",
            2,
            (dec!(100), dec!(500)),
            (2, 5),
            AnomalyDetectionDifficulty::Moderate,
        );

        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let amount = stage.random_amount(&mut rng);

        assert!(amount >= dec!(100));
        assert!(amount <= dec!(500));
    }

    #[test]
    fn test_scheme_context() {
        let context = SchemeContext::new(NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(), "1000")
            .with_audit(true)
            .with_detection_activity(0.3)
            .with_accounts(vec!["5000".to_string(), "6000".to_string()])
            .with_users(vec!["USER001".to_string()]);

        assert!(context.audit_in_progress);
        assert!((context.detection_activity - 0.3).abs() < 0.01);
        assert_eq!(context.available_accounts.len(), 2);
    }

    #[test]
    fn test_scheme_action() {
        let action = SchemeAction::new(
            Uuid::new_v4(),
            1,
            SchemeActionType::CreateFraudulentEntry,
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
        )
        .with_amount(dec!(5000))
        .with_account("5000")
        .with_user("USER001")
        .with_description("Test fraudulent entry")
        .with_technique(ConcealmentTechnique::DocumentManipulation);

        assert_eq!(action.amount, Some(dec!(5000)));
        assert_eq!(action.target_account, Some("5000".to_string()));
        assert!(!action.executed);
    }

    #[test]
    fn test_scheme_transaction_ref() {
        let tx_ref = SchemeTransactionRef::new(
            "JE001",
            NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            dec!(1000),
            1,
        )
        .with_anomaly("ANO001");

        assert_eq!(tx_ref.document_id, "JE001");
        assert_eq!(tx_ref.anomaly_id, Some("ANO001".to_string()));
    }
}
