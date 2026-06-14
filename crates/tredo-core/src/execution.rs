use crate::{DisciplineRules, MemoryStore, TradeSetup};

pub struct ExecutionEngine {
    pub initial_balance: f64,
    pub memory: MemoryStore,
}

impl ExecutionEngine {
    pub fn new(initial_balance: f64, memory: MemoryStore) -> Self {
        Self {
            initial_balance,
            memory,
        }
    }

    pub async fn execute_setup(
        &mut self,
        setup: TradeSetup,
        _rules: &DisciplineRules,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        println!("[ExecutionEngine] Executing setup for {}", setup.symbol);
        Ok(true)
    }
}
