use anyhow::{Context, Result};
use clap::Parser;
use colored::*;
use reqwest::Client;
use rand::Rng;
use serde::Deserialize;
use serde_json::json;
use solana_client::rpc_client::RpcClient;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::sleep;
use bs58;
use bincode;
use solana_program::vote::state::Vote;
use solana_program::vote::instruction::VoteInstruction;
use solana_client::rpc_response::RpcLeaderSchedule;
use indicatif::{ProgressBar, ProgressStyle};

/// CLI args: all required, no defaults.
#[derive(Parser, Debug)]
#[command(name = "Vote Checker", about = "Check vote transactions by slot/account")]
struct Args {
    #[arg(long)]
    url: String,
    #[arg(long)]
    account: String,
    #[arg(long)]
    slot: u64,
    #[arg(long)]
    distance: u64,
}

#[derive(Debug, Deserialize)]
struct BlockResponse {
    result: Option<BlockResult>,
}

#[derive(Debug, Deserialize)]
struct BlockResult {
    transactions: Vec<Transaction>,
}

#[derive(Debug, Deserialize)]
struct Transaction {
    transaction: TransactionData,
    meta: Option<TransactionMeta>,
}

#[derive(Debug, Deserialize)]
struct TransactionData {
    signatures: Vec<String>,
    message: TransactionMessage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransactionMessage {
    account_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransactionMeta {
    log_messages: Option<Vec<String>>,
}

/// Retry get_block with exponential backoff on errors or None results.
async fn get_block_with_retry(
    client: &Client,
    api_url: &str,
    slot: u64,
    max_attempts: usize,
) -> Result<Option<BlockResult>> {
    let mut attempts = 0;
    let mut delay = Duration::from_secs(2);

    loop {
        let res = get_block(client, api_url, slot).await;
        match &res {
            Ok(Some(_)) => return res,
            Ok(None) => {
                if attempts < max_attempts {
                    attempts += 1;
                    eprintln!(
                        "Block {} not found (attempt {}). Retrying in {:?}...",
                        slot, attempts, delay
                    );
                    sleep(delay).await;
                    delay *= 2;
                    continue;
                } else {
                    return res;
                }
            }
            Err(e) => {
                if attempts < max_attempts {
                    attempts += 1;
                    eprintln!(
                        "Error fetching block {}: {}. Retrying in {:?}...",
                        slot, e, delay
                    );
                    sleep(delay).await;
                    delay *= 2;
                    continue;
                } else {
                    return res;
                }
            }
        }
    }
}

/// Retry leader schedule fetch with exponential backoff and rate limit handling.
async fn get_leader_map_with_retry(
    rpc_url: &str,
    slot: u64,
    max_attempts: usize,
) -> anyhow::Result<HashMap<u64, String>> {
    let mut attempts = 0;
    let mut delay = Duration::from_secs(2);

    loop {
        let pb = ProgressBar::new_spinner();
        pb.set_style(ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .unwrap());
        pb.set_message("Fetching leader schedule...");
        pb.enable_steady_tick(Duration::from_millis(80));

        let rpc_client = RpcClient::new_with_timeout(rpc_url.to_string(), Duration::from_secs(20));
        let result = map_leader_slots(&rpc_client, slot);

        pb.finish_and_clear();

        match result {
            Ok(map) => {
                // No extra message, just continue
                return Ok(map);
            }
            Err(e) => {
                let is_rate_limited = e.to_string().contains("429") || e.to_string().contains("rate limit") || e.to_string().contains("timed out");
                if attempts < max_attempts && is_rate_limited {
                    attempts += 1;
                    eprintln!(
                        "Leader schedule fetch rate limited or timed out. Retrying in {:?}... (attempt {}/{})",
                        delay, attempts, max_attempts
                    );
                    sleep(delay).await;
                    delay *= 2;
                    continue;
                } else {
                    return Err(e).context("Failed to fetch leader schedule after retries");
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    println!(
        "\n{}\n{} {}  {} {}\n{}\n",
        "==============================".bright_black(),
        "Slot:".bold(),
        args.slot.to_string().yellow(),
        "Distance:".bold(),
        args.distance.to_string().yellow(),
        "==============================".bright_black()
    );

    // Maybe better retry logic for leader schedule
    let leader_map = get_leader_map_with_retry(&args.url, args.slot, 5).await
        .context("Could not fetch leader schedule (rate limited or RPC error). Exiting.")?;

    let http_client = Client::new();

    let mut rng = rand::rng();

    for offset in 0..=args.distance {
        let current_slot = args.slot.saturating_sub(offset);

        let block_result = get_block_with_retry(&http_client, &args.url, current_slot, 5).await;
        let jitter = rng.random_range(1000..=3000);
        sleep(Duration::from_millis(jitter)).await;

        match block_result {
            Ok(Some(block)) => {
                let vote_txs = extract_vote_transactions(&block);
                let vote_count = vote_txs.len();

                let mut found_vote = false;

                for (i, tx) in vote_txs.iter().enumerate() {
                    if let Some(account) = tx.transaction.message.account_keys.get(0) {
                        if account == &args.account {
                            println!(
                                "\n{} {}  {} {}",
                                "Slot:".bold(),
                                current_slot.to_string().green(),
                                "Votes:".bold(),
                                vote_count.to_string().cyan()
                            );

                            let sig = &tx.transaction.signatures[0];

                            println!(
                                "{} {}",
                                "Signature:".bold(),
                                sig.bright_white()
                            );

                            let voted_slot = extract_voted_slot(&args.url, sig).await;
                            let jitter = rng.random_range(1000..=3000);
                            sleep(Duration::from_millis(jitter)).await;

                            match voted_slot {
                                Ok(Some(vote_slot)) => println!(
                                    "{} [{}]",
                                    "Voted slot:".bold(),
                                    vote_slot.to_string().bright_yellow()
                                ),
                                Ok(None) => println!(
                                    "{}",
                                    "[unknown vote slot]".dimmed()
                                ),
                                Err(e) => println!(
                                    "{} {} {}",
                                    "[error extracting vote slot]".red(),
                                    sig.bright_white(),
                                    format!("({})", e).dimmed()
                                ),
                            }

                            println!(
                                "{} {}\n",
                                "Position:".bold(),
                                i.to_string().bright_blue()
                            );

                            found_vote = true;
                            break;
                        }
                    }
                }

                if !found_vote {
                    let leader_info = leader_map
                        .get(&current_slot)
                        .map(|l| format!("Slot leader: {}", l))
                        .unwrap_or_else(|| "Slot leader: unknown".to_string());

                    println!(
                        "\n{} {}  {} {} {}\n{}\n",
                        "Slot:".bold(),
                        current_slot.to_string().green(),
                        "Votes:".bold(),
                        vote_count.to_string().cyan(),
                        "[X]".red(),
                        leader_info.bright_black()
                    );
                }
            }
            Ok(None) => {
                println!(
                    "{} {}\n",
                    "Error:".bold().red(),
                    format!("No block found for {}", current_slot)
                );
            }
            Err(e) => {
                println!(
                    "{} {}\n",
                    "Error:".bold().red(),
                    format!("Failed to fetch block {}: {}", current_slot, e)
                );
            }
        }
    }

    println!("{}", "\nAll done!".bright_green());
    println!();
    Ok(())
}

/// Extract only vote transactions by looking for Vote program log in transaction meta.
fn extract_vote_transactions(block: &BlockResult) -> Vec<&Transaction> {
    block
        .transactions
        .iter()
        .filter(|tx| {
            tx.meta
                .as_ref()
                .and_then(|meta| meta.log_messages.as_ref())
                .map_or(false, |logs| {
                    logs.iter().any(|log| {
                        log.starts_with("Program Vote111111111111111111111111111111111111111 invoke")
                    })
                })
        })
        .collect()
}

/// Raw getBlock RPC call, returns block data or None if not found.
async fn get_block(client: &Client, api_url: &str, slot: u64) -> Result<Option<BlockResult>> {
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getBlock",
        "params": [
            slot,
            {
                "encoding": "json",
                "transactionDetails": "full",
                "rewards": false,
                "maxSupportedTransactionVersion": 0
            }
        ]
    });

    let resp = client
        .post(api_url)
        .json(&body)
        .send()
        .await
        .context("Failed to send getBlock request")?;

    let block_resp = resp
        .json::<BlockResponse>()
        .await
        .context("Failed to parse getBlock response")?;

    Ok(block_resp.result)
}

/// Extract the voted slot from a vote transaction signature.
/// Handles both Vote and TowerSync instructions.
/// Retries on RPC rate limit (429) with exponential backoff.
async fn extract_voted_slot(
    rpc_url: &str,
    signature: &str,
) -> Result<Option<u64>> {
    let client = reqwest::Client::new();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTransaction",
        "params": [
            signature,
            {
                "encoding": "json",
                "commitment": "confirmed"
            }
        ]
    });

    let mut attempts = 0;
    let max_attempts = 5;
    let mut delay = Duration::from_secs(2);
    let mut rate_limited = false;

    loop {
        let resp = client.post(rpc_url)
            .json(&body)
            .send()
            .await
            .context("Failed to send getTransaction request")?
            .json::<serde_json::Value>()
            .await
            .context("Failed to parse getTransaction response")?;

        if let Some(error) = resp.get("error") {
            if error.get("code") == Some(&serde_json::json!(429)) {
                rate_limited = true;
                if attempts < max_attempts {
                    attempts += 1;
                    eprintln!("Rate limited (429). Retrying in {:?}... (attempt {}/{})", delay, attempts, max_attempts);
                    sleep(delay).await;
                    delay *= 2;
                    continue;
                } else {
                    eprintln!("Max retries reached for signature {}. Skipping this transaction.", signature);
                    println!();
                    return Ok(None);
                }
            }
        }

        if rate_limited {
            println!();
        }

        // Defensive debug print for unexpected transaction structure.
        let print_debug = |reason: &str| {
            eprintln!(
                "DEBUG: Could not extract voted slot for signature {} ({}). Full transaction JSON:\n{}",
                signature,
                reason,
                serde_json::to_string_pretty(&resp).unwrap_or_else(|_| "<failed to serialize>".to_string())
            );
        };

        let tx = match resp.get("result").and_then(|r| r.get("transaction")).and_then(|t| t.get("message")) {
            Some(tx) => tx,
            None => {
                print_debug("missing transaction/message");
                return Ok(None);
            }
        };
        let instructions = match tx.get("instructions").and_then(|i| i.as_array()) {
            Some(i) => i,
            None => {
                print_debug("missing instructions");
                return Ok(None);
            }
        };
        let account_keys = match tx.get("accountKeys").and_then(|a| a.as_array()) {
            Some(a) => a,
            None => {
                print_debug("missing accountKeys");
                return Ok(None);
            }
        };

        for instr in instructions {
            let program_index = match instr.get("programIdIndex").and_then(|i| i.as_u64()) {
                Some(idx) => idx as usize,
                None => {
                    print_debug("missing programIdIndex");
                    continue;
                }
            };
            let program_id = match account_keys.get(program_index).and_then(|k| k.as_str()) {
                Some(pid) => pid,
                None => {
                    print_debug("missing program_id in account_keys");
                    continue;
                }
            };

            if program_id != "Vote111111111111111111111111111111111111111" {
                continue;
            }

            let encoded_data = match instr.get("data").and_then(|d| d.as_str()) {
                Some(data) => data,
                None => {
                    print_debug("missing data in instruction");
                    continue;
                }
            };
            let decoded_data = match bs58::decode(encoded_data).into_vec() {
                Ok(d) => d,
                Err(err) => {
                    eprintln!("Failed to decode base58: {}", err);
                    print_debug("base58 decode failed");
                    continue;
                }
            };

            match bincode::deserialize::<VoteInstruction>(&decoded_data) {
                Ok(VoteInstruction::Vote(vote_tx)) => {
                    let vote: &Vote = &vote_tx;
                    if let Some((slot, _)) = vote
                        .slots
                        .iter()
                        .zip((1..=vote.slots.len()).rev())
                        .find(|(_, confirmation_count)| *confirmation_count == 1)
                    {
                        return Ok(Some(*slot));
                    }
                }
                Ok(VoteInstruction::TowerSync(sync)) => {
                    if let Some(lockout) = sync
                        .lockouts
                        .iter()
                        .find(|l| l.confirmation_count() == 1)
                    {
                        return Ok(Some(lockout.slot()));
                    }
                }
                Ok(other) => {
                    eprintln!("Decoded but not Vote or TowerSync: {:?}", other);
                    print_debug("decoded but not Vote or TowerSync");
                }
                Err(err) => {
                    eprintln!("Failed to deserialize vote instruction: {}", err);
                    print_debug("bincode deserialize failed");
                }
            }
        }

        print_debug("no matching instruction found");
        return Ok(None);
    }
}

/// Map absolute slot -> leader identity for the epoch containing the given slot.
fn map_leader_slots(
    client: &RpcClient,
    slot: u64,
) -> anyhow::Result<HashMap<u64, String>> {
    let epoch_start = get_epoch_start_slot(client, slot)?;
    let schedule: RpcLeaderSchedule = client
        .get_leader_schedule(Some(epoch_start))?
        .ok_or_else(|| anyhow::anyhow!("No leader schedule found"))?;

    let mut slot_to_leader = HashMap::new();
    for (validator, rel_slots) in schedule {
        for rel_slot in rel_slots {
            let abs_slot = epoch_start + rel_slot as u64;
            slot_to_leader.insert(abs_slot, validator.clone());
        }
    }
    Ok(slot_to_leader)
}

/// Get the first slot of the epoch for a given slot.
fn get_epoch_start_slot(
    client: &RpcClient,
    slot: u64,
) -> anyhow::Result<u64> {
    let schedule = client.get_epoch_schedule()?;
    let epoch = schedule.get_epoch(slot);
    Ok(schedule.get_first_slot_in_epoch(epoch))
}
