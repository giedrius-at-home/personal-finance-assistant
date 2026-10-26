//! The canonical model every computation works on: own accounts and
//! positive-amount transactions with a direction, as the Python service had.

use jiff::Zoned;
use jiff::civil::Date;
use rust_decimal::Decimal;

use finance_types::report::{Currency, UnitMoney};

use crate::text::CountryEvidence;

/// Direction of a positive transaction amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Inflow,
    Outflow,
    /// Money back for earlier spending (owner-marked returns).
    Refund,
    Transfer,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inflow => "inflow",
            Self::Outflow => "outflow",
            Self::Refund => "refund",
            Self::Transfer => "transfer",
        }
    }
}

/// One of the owner's accounts, with its booked balance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    /// `firefly:<ledger id>`, the id the gateway tables use.
    pub account_id: String,
    pub ledger_id: i64,
    pub name: String,
    pub balance: UnitMoney,
    pub include_net_worth: bool,
    pub active: bool,
    pub iban: Option<String>,
}

impl Account {
    pub fn currency(&self) -> &Currency {
        &self.balance.currency
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tx {
    /// `firefly:<ledger id>`.
    pub transaction_id: String,
    pub ledger_id: i64,
    /// The own account the money left (outflow, transfer) or reached (inflow).
    pub account_id: String,
    /// The other own account of a transfer.
    pub counterparty_account_id: Option<String>,
    pub kind: Kind,
    /// Always positive.
    pub amount: Decimal,
    pub currency: Currency,
    pub occurred_at: Zoned,
    /// The day a card was used, when the bank booked it later.
    pub purchased_on: Option<Date>,
    pub description: Option<String>,
    pub counterparty_name: Option<String>,
    pub group_id: Option<String>,
    pub category: Option<String>,
    /// The bill (subscription) this payment belongs to.
    pub subscription_id: Option<String>,
    pub source_country: Option<CountryEvidence>,
    pub destination_country: Option<CountryEvidence>,
    /// Why a transfer is known to be between own accounts.
    pub transfer_evidence: Option<&'static str>,
}

impl Tx {
    /// The date that decides the transaction's month: the card purchase date
    /// when known, otherwise the local booking date.
    pub fn effective_on(&self) -> Date {
        self.purchased_on.unwrap_or_else(|| self.occurred_at.date())
    }

    /// The booking date in the owner's time zone.
    pub fn booked_on(&self) -> Date {
        self.occurred_at.date()
    }

    pub fn money(&self) -> UnitMoney {
        UnitMoney {
            amount: self.amount,
            currency: self.currency.clone(),
        }
    }

    /// The key `transaction_details` and `review_deferrals` use:
    /// `["firefly_iii", "transaction", "<id>"]`.
    pub fn source_key(&self) -> String {
        source_key(self.ledger_id)
    }
}

pub fn source_key(ledger_id: i64) -> String {
    format!("[\"firefly_iii\", \"transaction\", \"{ledger_id}\"]")
}

/// `firefly:<id>` → `<id>`.
pub fn ledger_id(canonical: &str) -> Option<i64> {
    canonical.strip_prefix("firefly:")?.parse().ok()
}
