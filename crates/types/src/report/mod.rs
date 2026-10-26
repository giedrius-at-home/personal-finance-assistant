//! The dashboard report every screen reads: the Rust port of the Python
//! service's `DashboardView`. Field names match its JSON so the two can be
//! compared field by field.

pub mod allocation;
pub mod budget;
pub mod cash;
pub mod catalog;
pub mod plans;
pub mod reporting;
pub mod review;
pub mod subscriptions;
pub mod trends;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A currency and its number of minor digits (EUR → 2).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Currency {
    pub code: String,
    pub minor_unit: u8,
}

impl Currency {
    pub fn new(code: &str) -> Self {
        let minor_unit = match code {
            "JPY" | "KRW" | "ISK" | "HUF" | "CLP" | "VND" => 0,
            "BHD" | "KWD" | "OMR" | "JOD" | "TND" => 3,
            _ => 2,
        };
        Self {
            code: code.to_owned(),
            minor_unit,
        }
    }

    pub fn eur() -> Self {
        Self::new("EUR")
    }
}

/// An exact amount with its currency unit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitMoney {
    pub amount: Decimal,
    pub currency: Currency,
}
