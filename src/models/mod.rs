pub mod event;
pub mod policy;
pub mod project;
pub mod task;
pub mod usage;

use std::fmt;
use std::ops::{Add, Sub};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// ID newtypes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UsageId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EventId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActionId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BackendId(pub String);

impl ProjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl UsageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl EventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl ActionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl BackendId {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BackendId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// MoneyAmount — fixed-precision monetary value (integer cents)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct MoneyAmount {
    pub cents: i64,
}

impl MoneyAmount {
    pub const ZERO: MoneyAmount = MoneyAmount { cents: 0 };

    pub fn from_cents(cents: i64) -> Self {
        Self { cents }
    }

    pub fn from_dollars(dollars: f64) -> Self {
        Self {
            cents: (dollars * 100.0).round() as i64,
        }
    }

    pub fn as_dollars(&self) -> f64 {
        self.cents as f64 / 100.0
    }

    /// Returns the percentage of `self` relative to `total`.
    /// Returns 0 if total is zero.
    pub fn percentage_of(&self, total: MoneyAmount) -> u8 {
        if total.cents == 0 {
            return 0;
        }
        let pct = (self.cents as f64 / total.cents as f64 * 100.0).round() as i64;
        pct.clamp(0, 255) as u8
    }
}

impl Add for MoneyAmount {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            cents: self.cents + rhs.cents,
        }
    }
}

impl Sub for MoneyAmount {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            cents: self.cents - rhs.cents,
        }
    }
}

impl fmt::Display for MoneyAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.cents < 0 { "-" } else { "" };
        let abs = self.cents.unsigned_abs();
        write!(f, "{}${}.{:02}", sign, abs / 100, abs % 100)
    }
}

// ---------------------------------------------------------------------------
// Common enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskType {
    DeepCodeReasoning,
    Planning,
    Review,
    CommitPreparation,
    Summarization,
    BacklogTriage,
    LowCostDrafting,
    PrivateLocalTask,
    Experimental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrivacyLevel {
    Public,
    Private,
    LocalOnly,
}

impl Priority {
    /// Returns a numeric rank for priority ordering (lower = higher priority).
    /// Critical=0, High=1, Normal=2, Low=3.
    pub fn rank(&self) -> u8 {
        match self {
            Priority::Critical => 0,
            Priority::High => 1,
            Priority::Normal => 2,
            Priority::Low => 3,
        }
    }
}

impl Default for Priority {
    fn default() -> Self {
        Self::Normal
    }
}

impl Default for PrivacyLevel {
    fn default() -> Self {
        Self::Public
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_amount_from_dollars() {
        let m = MoneyAmount::from_dollars(12.50);
        assert_eq!(m.cents, 1250);
        assert!((m.as_dollars() - 12.50).abs() < f64::EPSILON);
    }

    #[test]
    fn money_amount_arithmetic() {
        let a = MoneyAmount::from_cents(1000);
        let b = MoneyAmount::from_cents(250);
        assert_eq!((a + b).cents, 1250);
        assert_eq!((a - b).cents, 750);
    }

    #[test]
    fn money_amount_display() {
        assert_eq!(MoneyAmount::from_cents(1250).to_string(), "$12.50");
        assert_eq!(MoneyAmount::from_cents(5).to_string(), "$0.05");
        assert_eq!(MoneyAmount::from_cents(0).to_string(), "$0.00");
        assert_eq!(MoneyAmount::from_cents(-350).to_string(), "-$3.50");
    }

    #[test]
    fn money_amount_percentage() {
        let spent = MoneyAmount::from_cents(7500);
        let total = MoneyAmount::from_cents(10000);
        assert_eq!(spent.percentage_of(total), 75);
    }

    #[test]
    fn money_amount_percentage_zero_total() {
        let spent = MoneyAmount::from_cents(100);
        let total = MoneyAmount::ZERO;
        assert_eq!(spent.percentage_of(total), 0);
    }

    #[test]
    fn money_amount_ordering() {
        let a = MoneyAmount::from_cents(100);
        let b = MoneyAmount::from_cents(200);
        assert!(a < b);
        assert!(b > a);
        assert_eq!(a, MoneyAmount::from_cents(100));
    }

    #[test]
    fn backend_id_display() {
        let id = BackendId::new("claude");
        assert_eq!(id.to_string(), "claude");
        assert_eq!(id.as_str(), "claude");
    }

    #[test]
    fn task_type_serialization_roundtrip() {
        let tt = TaskType::DeepCodeReasoning;
        let json = serde_json::to_string(&tt).unwrap();
        let parsed: TaskType = serde_json::from_str(&json).unwrap();
        assert_eq!(tt, parsed);
    }

    #[test]
    fn priority_default_is_normal() {
        assert_eq!(Priority::default(), Priority::Normal);
    }

    #[test]
    fn privacy_default_is_public() {
        assert_eq!(PrivacyLevel::default(), PrivacyLevel::Public);
    }

    #[test]
    fn id_uniqueness() {
        let a = ProjectId::new();
        let b = ProjectId::new();
        assert_ne!(a, b);
    }
}
