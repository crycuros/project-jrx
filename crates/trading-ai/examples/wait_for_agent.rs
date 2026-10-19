//! Wait until the ReAct agent is answering.
//!
//! The bot's gate fails closed, so a bot that starts before the terminal is
//! up simply does not trade. That is safe but confusing. This is the piece
//! systemd runs before the bot: it waits for the agent, and says what it is
//! waiting for.
//!
//! ```text
//! cargo run -p trading-ai --example wait_for_agent
//! ```
//!
//! Exit codes: 0 the agent answered, 1 it did not (in time).

use std::time::{Duration, Instant};

use trading_ai::agent::AgentClient;
use trading_core::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env();
    let client = AgentClient::new(&cfg);
    let deadline_secs = cfg.agent_timeout_secs.clamp(10, 600);
    let started = Instant::now();

    println!("waiting for the agent at {} …", cfg.agent_api_url);
    let mut last_error = String::new();
    while started.elapsed().as_secs() < deadline_secs {
        match client.ping().await {
            Ok(()) => {
                println!(
                    "the agent answered after {}s — the gate is live",
                    started.elapsed().as_secs()
                );
                return Ok(());
            }
            Err(e) => {
                last_error = e.to_string();
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    }

    eprintln!(
        "the agent did not answer within {deadline_secs}s: {last_error}"
    );
    eprintln!(
        "the bot will still start, but its gate fails closed, so it will not trade."
    );
    std::process::exit(1);
}
