//! Multi-stage fraud scheme framework.
//!
//! This module provides realistic multi-stage fraud schemes that evolve over time,
//! including embezzlement, revenue manipulation, kickback, and RIP-GNN pathology lab schemes.

mod circular_funding;
mod embezzlement;
mod expense_laundering;
mod intercompany_wash_trades;
mod kickback;
mod phantom_warehousing;
mod revenue_manipulation;
mod scheme;
mod shadow_payroll;
mod smurfing;
mod triad_bypass;

pub use circular_funding::CircularFundingScheme;
pub use embezzlement::GradualEmbezzlementScheme;
pub use expense_laundering::ExpenseLaunderingScheme;
pub use intercompany_wash_trades::IntercompanyWashTradeScheme;
pub use kickback::VendorKickbackScheme;
pub use phantom_warehousing::PhantomWarehousingScheme;
pub use revenue_manipulation::RevenueManipulationScheme;
pub use scheme::{
    FraudScheme, SchemeAction, SchemeActionType, SchemeContext, SchemeStage, SchemeStatus,
    SchemeTransactionRef,
};
pub use shadow_payroll::ShadowPayrollScheme;
pub use smurfing::SmurfingScheme;
pub use triad_bypass::TriadBypassScheme;
