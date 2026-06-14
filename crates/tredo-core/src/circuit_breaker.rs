// Circuit breaker for external API calls (Ollama, Binance, CoinGecko, etc.)
// Prevents cascading failures when an API is down.

use std::time::{Duration, Instant};

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CircuitState {
    Closed,   // Normal operation
    Open,     // Failing - reject calls
    HalfOpen, // Testing if recovery happened
}

/// Circuit breaker for a specific service
pub struct CircuitBreaker {
    name: String,
    state: CircuitState,
    failure_count: usize,
    success_count: usize,
    last_failure_time: Option<Instant>,

    // Configurable thresholds
    failure_threshold: usize,
    success_threshold: usize,
    timeout: Duration,
}

impl CircuitBreaker {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            state: CircuitState::Closed,
            failure_count: 0,
            success_count: 0,
            last_failure_time: None,
            failure_threshold: 5, // Open after 5 consecutive failures
            success_threshold: 3, // Close after 3 successes in half-open
            timeout: Duration::from_secs(30), // Try again after 30 seconds
        }
    }

    /// Returns true if the call should proceed
    pub fn can_execute(&mut self) -> bool {
        match self.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if timeout has passed
                if let Some(last_failure) = self.last_failure_time {
                    if last_failure.elapsed() > self.timeout {
                        // Transition to half-open
                        self.state = CircuitState::HalfOpen;
                        self.success_count = 0;
                        eprintln!("[CircuitBreaker] {}: transitioning to HALF_OPEN", self.name);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => {
                // Allow one test call
                self.success_count < 1
            }
        }
    }

    /// Record a successful call
    pub fn record_success(&mut self) {
        match self.state {
            CircuitState::Closed => {
                // Reset failure count on success
                self.failure_count = 0;
            }
            CircuitState::HalfOpen => {
                self.success_count += 1;
                if self.success_count >= self.success_threshold {
                    self.state = CircuitState::Closed;
                    self.failure_count = 0;
                    eprintln!(
                        "[CircuitBreaker] {}: circuit CLOSED after recovery",
                        self.name
                    );
                }
            }
            CircuitState::Open => {
                // Should not happen, but handle gracefully
                self.state = CircuitState::HalfOpen;
                self.success_count = 1;
            }
        }
    }

    /// Record a failed call
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.last_failure_time = Some(Instant::now());

        match self.state {
            CircuitState::Closed => {
                if self.failure_count >= self.failure_threshold {
                    self.state = CircuitState::Open;
                    eprintln!(
                        "[CircuitBreaker] {}: circuit OPENED after {} failures",
                        self.name, self.failure_count
                    );
                }
            }
            CircuitState::HalfOpen => {
                // Any failure in half-open goes back to open
                self.state = CircuitState::Open;
                eprintln!(
                    "[CircuitBreaker] {}: circuit OPENED after half-open failure",
                    self.name
                );
            }
            CircuitState::Open => {
                // Already open, do nothing
            }
        }
    }

    /// Get current state
    pub fn get_state(&self) -> CircuitState {
        self.state
    }

    /// Get name
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Manages multiple circuit breakers for different services
pub struct CircuitBreakerManager {
    breakers: std::collections::HashMap<String, CircuitBreaker>,
}

impl CircuitBreakerManager {
    pub fn new() -> Self {
        let mut managers = Self {
            breakers: std::collections::HashMap::new(),
        };

        // Initialize default breakers
        managers
            .breakers
            .insert("ollama".to_string(), CircuitBreaker::new("ollama"));
        managers
            .breakers
            .insert("binance".to_string(), CircuitBreaker::new("binance"));
        managers
            .breakers
            .insert("coingecko".to_string(), CircuitBreaker::new("coingecko"));
        managers
            .breakers
            .insert("yahoo".to_string(), CircuitBreaker::new("yahoo"));

        managers
    }

    /// Get a circuit breaker by name
    pub fn get(&mut self, name: &str) -> Option<&mut CircuitBreaker> {
        self.breakers.get_mut(name)
    }

    /// Execute a function with circuit breaker protection
    /// Returns Ok result, or Err with a string message if circuit is open
    pub fn execute<F, T>(&mut self, service: &str, op: F) -> Result<T, String>
    where
        F: FnOnce() -> Result<T, String>,
    {
        let breaker = self
            .breakers
            .get_mut(service)
            .expect("Circuit breaker not found for service");

        if !breaker.can_execute() {
            let msg = format!(
                "[CircuitBreaker] {}: call rejected - circuit is OPEN",
                service
            );
            eprintln!("{}", msg);
            return Err(msg);
        }

        match op() {
            Ok(result) => {
                breaker.record_success();
                Ok(result)
            }
            Err(e) => {
                breaker.record_failure();
                Err(e)
            }
        }
    }
}

impl Default for CircuitBreakerManager {
    fn default() -> Self {
        Self::new()
    }
}
