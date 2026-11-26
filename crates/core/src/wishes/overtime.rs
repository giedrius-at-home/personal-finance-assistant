//! Overtime pay: what an extra hour a week brings home in a month.
//!
//! Pure arithmetic, no I/O. An extra hour is paid at the payslip's plain
//! hourly rate (holiday pay is already in the salary and overtime earns no
//! premium) and taxed at the payslip's special rate, which is the marginal
//! rate: the bracket's tax plus the tax credits that shrink as pay grows.

use finance_types::gateway::OwnerSettings;
use rust_decimal::Decimal;

/// Every hour a week has: the owner may try any number of extra hours, so
/// the scenarios run to what the week holds, not to what the law allows.
const WEEK_HOURS: u32 = 168;
/// Read when the contract's hours are not set.
const FULL_TIME_WEEK: u32 = 40;

/// What the settings say about pay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pay {
    pub hourly_gross: Decimal,
    pub tax_percent: Decimal,
    pub contract_hours_week: Option<Decimal>,
    pub salary: Option<Decimal>,
}

impl Pay {
    /// `None` until both the hourly rate and the tax on extra pay are set:
    /// a net figure without the tax would promise about twice too much.
    pub fn from_settings(settings: &OwnerSettings) -> Option<Self> {
        Some(Self {
            hourly_gross: settings.hourly_rate_eur?,
            tax_percent: settings.overtime_tax_percent?,
            contract_hours_week: settings.contract_hours_week,
            salary: settings.monthly_salary_eur,
        })
    }

    /// What one extra hour brings home, unrounded.
    fn net(&self) -> Decimal {
        self.hourly_gross * (Decimal::ONE_HUNDRED - self.tax_percent) / Decimal::ONE_HUNDRED
    }

    pub fn hourly_net(&self) -> Decimal {
        self.net().round_dp(2)
    }

    /// Net extra in an average month (52 weeks over 12 months) for
    /// `hours_week` extra hours every week.
    pub fn extra_monthly(&self, hours_week: u32) -> Decimal {
        (self.net() * Decimal::from(hours_week) * Decimal::from(52) / Decimal::from(12)).round_dp(2)
    }

    /// The extra hours a week left after the contract's, at least one.
    pub fn max_hours_week(&self) -> u32 {
        let contract = self
            .contract_hours_week
            .unwrap_or_else(|| Decimal::from(FULL_TIME_WEEK));
        let room = (Decimal::from(WEEK_HOURS) - contract).floor();
        u32::try_from(room.mantissa()).unwrap_or(0).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn pay(contract: Option<&str>) -> Pay {
        Pay {
            hourly_gross: d("23.59"),
            tax_percent: d("50.47"),
            contract_hours_week: contract.map(d),
            salary: Some(d("3127.27")),
        }
    }

    #[test]
    fn an_extra_hour_is_taxed_at_the_special_rate() {
        let pay = pay(Some("36"));
        // 23.59 × (1 − 0.5047).
        assert_eq!(pay.hourly_net(), d("11.68"));
        // Four hours a week: 11.684127 × 4 × 52 / 12.
        assert_eq!(pay.extra_monthly(4), d("202.52"));
        assert_eq!(pay.extra_monthly(0), d("0"));
    }

    #[test]
    fn the_extra_hours_run_to_the_end_of_the_week() {
        assert_eq!(pay(Some("36")).max_hours_week(), 132);
        assert_eq!(pay(Some("38.5")).max_hours_week(), 129);
        assert_eq!(pay(None).max_hours_week(), 128);
        assert_eq!(pay(Some("168")).max_hours_week(), 1);
    }

    #[test]
    fn pay_needs_the_rate_and_the_tax() {
        let mut settings = OwnerSettings::default();
        assert_eq!(Pay::from_settings(&settings), None);
        settings.hourly_rate_eur = Some(d("23.59"));
        assert_eq!(Pay::from_settings(&settings), None);
        settings.overtime_tax_percent = Some(d("50.47"));
        assert_eq!(
            Pay::from_settings(&settings).unwrap().hourly_net(),
            d("11.68")
        );
    }
}
