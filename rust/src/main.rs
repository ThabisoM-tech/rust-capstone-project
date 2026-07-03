use bitcoincore_rpc::bitcoin::Amount;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use std::fs::File;
use std::io::Write;

// Node access params
const RPC_URL: &str = "http://127.0.0.1:18443"; // Default regtest RPC port
const RPC_USER: &str = "alice";
const RPC_PASS: &str = "password";

fn main() -> bitcoincore_rpc::Result<()> {
    // Connect to Bitcoin Core RPC
    let rpc = Client::new(
        RPC_URL,
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Get blockchain info
    let blockchain_info = rpc.get_blockchain_info()?;
    println!("Blockchain Info: {:?}", blockchain_info);

    // Create/Load the wallets, named 'Miner' and 'Trader'.
    // list_wallets() tells us which wallets are already loaded on the node,
    // so we only create/load a wallet if it isn't already active.
    let loaded_wallets = rpc.list_wallets()?;

    if loaded_wallets.contains(&"Miner".to_string()) {
        println!("Miner wallet already loaded.");
    } else {
        let miner_wallet_result = rpc.create_wallet("Miner", None, None, None, None);
        if miner_wallet_result.is_ok() {
            println!("Created wallet: Miner");
        } else {
            println!("Miner wallet exists but not loaded, loading it now");
            rpc.load_wallet("Miner")?;
        }
    }

    if loaded_wallets.contains(&"Trader".to_string()) {
        println!("Trader wallet already loaded.");
    } else {
        let trader_wallet_result = rpc.create_wallet("Trader", None, None, None, None);
        if trader_wallet_result.is_ok() {
            println!("Created wallet: Trader");
        } else {
            println!("Trader wallet exists but not loaded, loading it now");
            rpc.load_wallet("Trader")?;
        }
    }

    // Wallet-specific RPC calls (new address, balance, send) must go through
    // a client pointed at that wallet's own endpoint, not the base client.
    let miner_rpc = Client::new(
        &format!("{}/wallet/Miner", RPC_URL),
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    let trader_rpc = Client::new(
        &format!("{}/wallet/Trader", RPC_URL),
        Auth::UserPass(RPC_USER.to_owned(), RPC_PASS.to_owned()),
    )?;

    // Generate one address from the Miner wallet with label "Mining Reward"
    let miner_address = miner_rpc
        .get_new_address(Some("Mining Reward"), None)?
        .assume_checked();
    println!("Miner address: {:?}", miner_address);

    // Mine blocks to the Miner address until wallet balance is positive.
    // A block reward (coinbase transaction) requires 100 confirmations before
    // it becomes spendable ("coinbase maturity"), which protects the network
    // against chain reorganizations invalidating already-spent coinbase outputs.
    // So mining block #1 creates the reward, but it only matures once block #101
    // is mined on top of it - hence mining 101 blocks total to get a positive balance.
    let block_hashes = miner_rpc.generate_to_address(101, &miner_address)?;
    println!("Mined {} blocks.", block_hashes.len());

    let miner_balance = miner_rpc.get_balance(None, None)?;
    println!("Miner balance: {}", miner_balance);

    // Create a receiving address from Trader wallet, labeled "Received"
    let trader_address = trader_rpc
        .get_new_address(Some("Received"), None)?
        .assume_checked();
    println!("Trader address: {:?}", trader_address);

    // Send 20 BTC from Miner wallet to Trader's address
    let txid = miner_rpc.send_to_address(
        &trader_address,
        Amount::from_btc(20.0).unwrap(),
        None, None, None, None, None, None,
    )?;
    println!("Sent 20 BTC. TxID: {}", txid);

    // Fetch the unconfirmed transaction from the mempool
    let mempool_entry = miner_rpc.get_mempool_entry(&txid)?;
    println!("Mempool entry: {:?}", mempool_entry);

    // Confirm the transaction by mining 1 block
    let confirm_block_hashes = miner_rpc.generate_to_address(1, &miner_address)?;
    println!("Confirmed with block: {:?}", confirm_block_hashes);

    let confirm_block_hash = confirm_block_hashes[0];
    let block_header = miner_rpc.get_block_header_info(&confirm_block_hash)?;
    let block_height = block_header.height;
    println!(
        "Confirmed at height: {}, block hash: {}",
        block_height, confirm_block_hash
    );

    // Get detailed transaction info (inputs and outputs)
    let tx_info = miner_rpc.get_raw_transaction_info(&txid, None)?;

    // Identify Trader's output (payment) and Miner's output (change) by
    // comparing each output's address against the Trader address we generated.
    let mut trader_output_amount = Amount::from_sat(0);
    let mut trader_output_address = String::new();
    let mut miner_change_amount = Amount::from_sat(0);
    let mut miner_change_address = String::new();

    for vout in &tx_info.vout {
        let addr = vout
            .script_pub_key
            .address
            .clone()
            .unwrap()
            .assume_checked();
        if addr == trader_address {
            trader_output_amount = vout.value;
            trader_output_address = addr.to_string();
        } else {
            miner_change_amount = vout.value;
            miner_change_address = addr.to_string();
        }
    }

    println!(
        "Trader output: {} -> {}",
        trader_output_address, trader_output_amount
    );
    println!(
        "Miner change: {} -> {}",
        miner_change_address, miner_change_amount
    );

    // Get Miner's input address and amount by looking up the previous
    // transaction referenced by the first input (vin only stores which
    // previous output was spent, not the address/amount directly).
    let first_vin = &tx_info.vin[0];
    let prev_txid = first_vin.txid.unwrap();
    let prev_vout_index = first_vin.vout.unwrap() as usize;

    let prev_tx_info = miner_rpc.get_raw_transaction_info(&prev_txid, None)?;
    let prev_vout = &prev_tx_info.vout[prev_vout_index];

    let miner_input_amount = prev_vout.value;
    let miner_input_address = prev_vout
        .script_pub_key
        .address
        .clone()
        .unwrap()
        .assume_checked()
        .to_string();

    println!("Miner input: {} -> {}", miner_input_address, miner_input_amount);

    // Fee is shown as a negative number, representing money leaving the sender's balance
    let fee = mempool_entry.fees.base.to_btc();
    let fee_negative = -fee;

    // Write the transaction data to ../out.txt in the specified format
    let mut file = File::create("../out.txt")?;
    writeln!(file, "{}", txid)?;
    writeln!(file, "{}", miner_input_address)?;
    writeln!(file, "{}", miner_input_amount.to_btc())?;
    writeln!(file, "{}", trader_output_address)?;
    writeln!(file, "{}", trader_output_amount.to_btc())?;
    writeln!(file, "{}", miner_change_address)?;
    writeln!(file, "{}", miner_change_amount.to_btc())?;
    writeln!(file, "{}", fee_negative)?;
    writeln!(file, "{}", block_height)?;
    writeln!(file, "{}", confirm_block_hash)?;

    println!("Wrote transaction details to out.txt");

    Ok(())
}