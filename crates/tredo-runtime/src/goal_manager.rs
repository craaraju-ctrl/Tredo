use serde::{Deserialize, Serialize};
use tredo_autonomous::state::SharedState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    pub id: String,
    pub description: String,
    pub goal_type: GoalType,
    pub priority: u8,
    pub status: GoalStatus,
    pub current_progress: f64,
    pub deadline: Option<chrono::DateTime<chrono::Utc>>,
    pub sub_goals: Vec<Goal>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_updated: chrono::DateTime<chrono::Utc>,
    pub success_criteria: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GoalType {
    Return { period: TimePeriod, target_pct: f64 },
    RiskLimit { max_drawdown_pct: f64 },
    Learning { topic: String },
    Capability { feature: String },
    Avoidance { condition: String },
    Exploration { description: String, probe_count: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum GoalStatus {
    Active,
    Achieved,
    Failed,
    Suspended,
    Decomposed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TimePeriod {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalEvent {
    pub goal_id: String,
    pub event_type: GoalEventType,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GoalEventType {
    Created,
    ProgressMade { from: f64, to: f64 },
    Achieved,
    Failed { reason: String },
    Suspended { reason: String },
    Resumed,
    Decomposed { into: Vec<String> },
    RePrioritized { old: u8, new: u8 },
}

pub struct GoalManager {
    state: SharedState,
    goals: Vec<Goal>,
    history: Vec<GoalEvent>,
}

impl GoalManager {
    pub fn new(state: SharedState) -> Self {
        let mut mgr = Self {
            state,
            goals: Vec::new(),
            history: Vec::new(),
        };
        mgr.seed_default_goals();
        mgr
    }

    fn seed_default_goals(&mut self) {
        self.goals.push(Goal {
            id: "goal-no-ruin".to_string(),
            description: "Never lose more than 15% in a single day".to_string(),
            goal_type: GoalType::RiskLimit { max_drawdown_pct: 0.15 },
            priority: 10,
            status: GoalStatus::Active,
            current_progress: 0.0,
            deadline: None,
            sub_goals: vec![],
            created_at: chrono::Utc::now(),
            last_updated: chrono::Utc::now(),
            success_criteria: "Daily DD < 15% at all times".to_string(),
        });
        self.goals.push(Goal {
            id: "goal-learn".to_string(),
            description: "Accumulate at least 20 high-quality trade episodes per regime".to_string(),
            goal_type: GoalType::Learning { topic: "regime_adaptation".to_string() },
            priority: 5,
            status: GoalStatus::Active,
            current_progress: 0.0,
            deadline: None,
            sub_goals: vec![],
            created_at: chrono::Utc::now(),
            last_updated: chrono::Utc::now(),
            success_criteria: "20+ episodes with regret < 0.3 per regime".to_string(),
        });
    }

    pub async fn update_progress(&mut self) {
        for goal in self.goals.iter_mut() {
            if goal.status != GoalStatus::Active {
                continue;
            }
            let new_progress = match &goal.goal_type {
                GoalType::Return { period: _, target_pct } => {
                    let portfolio = self.state.portfolio.read().await;
                    let current = portfolio.daily_pnl_pct;
                    drop(portfolio);
                    (current / target_pct).clamp(0.0, 1.0)
                }
                GoalType::RiskLimit { max_drawdown_pct } => {
                    let portfolio = self.state.portfolio.read().await;
                    let dd = portfolio.max_drawdown_today.abs() / portfolio.total_equity.max(1.0);
                    drop(portfolio);
                    if dd > *max_drawdown_pct {
                        goal.status = GoalStatus::Failed;
                        self.history.push(GoalEvent {
                            goal_id: goal.id.clone(),
                            event_type: GoalEventType::Failed {
                                reason: format!("DD {:.2}% > max {:.2}%", dd * 100.0, max_drawdown_pct * 100.0),
                            },
                            timestamp: chrono::Utc::now(),
                            note: "Risk limit breached".to_string(),
                        });
                    }
                    dd / max_drawdown_pct
                }
                GoalType::Learning { topic: _ } => {
                    let store = &self.state.episode_store;
                    let n = store.load_recent_closed_trades(50, None).unwrap_or_default().len();
                    (n as f64 / 20.0).min(1.0)
                }
                GoalType::Capability { feature } => match feature.as_str() {
                    _ => 0.0,
                },
                GoalType::Avoidance { condition: _ } => 1.0,
                GoalType::Exploration { description: _, probe_count } => {
                    let store = &self.state.episode_store;
                    let n = store.load_recent_closed_trades(50, None).unwrap_or_default().len();
                    (n as f64 / *probe_count as f64).min(1.0)
                }
            };
            if (new_progress - goal.current_progress).abs() > 0.01 {
                let old = goal.current_progress;
                goal.current_progress = new_progress;
                goal.last_updated = chrono::Utc::now();
                self.history.push(GoalEvent {
                    goal_id: goal.id.clone(),
                    event_type: GoalEventType::ProgressMade { from: old, to: new_progress },
                    timestamp: chrono::Utc::now(),
                    note: "Periodic update".to_string(),
                });
            }
            if new_progress >= 1.0 && goal.status == GoalStatus::Active {
                goal.status = GoalStatus::Achieved;
                self.history.push(GoalEvent {
                    goal_id: goal.id.clone(),
                    event_type: GoalEventType::Achieved,
                    timestamp: chrono::Utc::now(),
                    note: goal.description.clone(),
                });
            }
        }
    }

    pub fn add_goal(&mut self, goal: Goal) {
        self.goals.push(goal);
    }

    pub fn active_goals(&self) -> Vec<&Goal> {
        self.goals.iter().filter(|g| g.status == GoalStatus::Active).collect()
    }

    pub fn summary(&self) -> String {
        let active = self.goals.iter().filter(|g| g.status == GoalStatus::Active).count();
        let achieved = self.goals.iter().filter(|g| g.status == GoalStatus::Achieved).count();
        let failed = self.goals.iter().filter(|g| g.status == GoalStatus::Failed).count();
        format!("Goals: {} active, {} achieved, {} failed", active, achieved, failed)
    }
}
