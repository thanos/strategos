use chrono::{Datelike, Utc};

use crate::models::MoneyAmount;

/// Budget forecast based on current spend and time elapsed this month.
#[derive(Debug, Clone)]
pub struct BudgetForecast {
    /// Current spend this period.
    pub current_spend: MoneyAmount,
    /// Budget limit for the period.
    pub limit: MoneyAmount,
    /// Day of the month (1-based).
    pub day_of_month: u32,
    /// Total days in the month.
    pub days_in_month: u32,
    /// Projected end-of-month spend at current burn rate.
    pub projected_spend: MoneyAmount,
    /// Estimated days remaining until budget exhaustion.
    /// None if burn rate is zero or budget is unlimited.
    pub days_until_exhaustion: Option<u32>,
    /// Daily burn rate in cents.
    pub daily_burn_rate: MoneyAmount,
    /// Whether the projected spend exceeds the limit.
    pub projected_overspend: bool,
}

impl BudgetForecast {
    /// Compute a forecast from current spend and limit.
    pub fn compute(current_spend: MoneyAmount, limit: MoneyAmount) -> Self {
        let now = Utc::now();
        let day_of_month = now.day();
        let days_in_month = days_in_current_month();

        let daily_burn_rate = if day_of_month > 0 {
            MoneyAmount::from_cents(current_spend.cents / day_of_month as i64)
        } else {
            MoneyAmount::ZERO
        };

        let projected_spend = MoneyAmount::from_cents(
            daily_burn_rate.cents * days_in_month as i64,
        );

        let days_until_exhaustion = if daily_burn_rate.cents > 0 {
            let remaining = limit.cents - current_spend.cents;
            if remaining <= 0 {
                Some(0)
            } else {
                Some((remaining / daily_burn_rate.cents) as u32)
            }
        } else {
            None
        };

        let projected_overspend = projected_spend > limit;

        Self {
            current_spend,
            limit,
            day_of_month,
            days_in_month,
            projected_spend,
            days_until_exhaustion,
            daily_burn_rate,
            projected_overspend,
        }
    }
}

fn days_in_current_month() -> u32 {
    let now = Utc::now();
    let year = now.year();
    let month = now.month();

    if month == 12 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .and_then(|next| next.pred_opt())
    .map(|last| last.day())
    .unwrap_or(30)
}

impl std::fmt::Display for BudgetForecast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Day {}/{} of month",
            self.day_of_month, self.days_in_month
        )?;
        writeln!(f, "Current spend: {}", self.current_spend)?;
        writeln!(f, "Daily burn rate: {}/day", self.daily_burn_rate)?;
        writeln!(
            f,
            "Projected EOM spend: {}",
            self.projected_spend
        )?;
        if self.projected_overspend {
            writeln!(f, "WARNING: projected to exceed budget of {}", self.limit)?;
        }
        if let Some(days) = self.days_until_exhaustion {
            if days == 0 {
                writeln!(f, "Budget EXHAUSTED")?;
            } else {
                writeln!(f, "Days until exhaustion: {}", days)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forecast_zero_spend() {
        let forecast = BudgetForecast::compute(MoneyAmount::ZERO, MoneyAmount::from_dollars(100.0));
        assert_eq!(forecast.daily_burn_rate, MoneyAmount::ZERO);
        assert_eq!(forecast.projected_spend, MoneyAmount::ZERO);
        assert!(!forecast.projected_overspend);
        assert!(forecast.days_until_exhaustion.is_none());
    }

    #[test]
    fn forecast_moderate_spend() {
        let forecast = BudgetForecast::compute(
            MoneyAmount::from_dollars(30.0),
            MoneyAmount::from_dollars(100.0),
        );
        assert!(forecast.daily_burn_rate.cents > 0);
        assert!(forecast.days_until_exhaustion.is_some());
        assert!(forecast.days_until_exhaustion.unwrap() > 0);
    }

    #[test]
    fn forecast_over_budget() {
        let forecast = BudgetForecast::compute(
            MoneyAmount::from_dollars(110.0),
            MoneyAmount::from_dollars(100.0),
        );
        assert!(forecast.projected_overspend);
        assert_eq!(forecast.days_until_exhaustion, Some(0));
    }

    #[test]
    fn forecast_display() {
        let forecast = BudgetForecast::compute(
            MoneyAmount::from_dollars(50.0),
            MoneyAmount::from_dollars(100.0),
        );
        let display = format!("{}", forecast);
        assert!(display.contains("Current spend:"));
        assert!(display.contains("Daily burn rate:"));
    }

    #[test]
    fn days_in_month_is_reasonable() {
        let days = days_in_current_month();
        assert!(days >= 28);
        assert!(days <= 31);
    }
}
