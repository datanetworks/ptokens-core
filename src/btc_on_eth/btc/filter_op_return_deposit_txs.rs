use crate::{
    chains::btc::{
        btc_database_utils::{get_btc_address_from_db, get_btc_public_key_slice_from_db},
        btc_state::BtcState,
        btc_types::{BtcPubKeySlice, BtcTransactions},
        btc_utils::get_pay_to_pub_key_hash_script,
    },
    traits::DatabaseInterface,
    types::Result,
};
use bitcoin::{
    blockdata::{script::Script as BtcScript, transaction::Transaction as BtcTransaction},
    consensus::encode::serialize as btc_serialize,
};

fn sig_script_contains_pub_key(script_sig: &BtcScript, btc_pub_key_slice: &BtcPubKeySlice) -> bool {
    hex::encode(btc_serialize(script_sig)).contains(&hex::encode(btc_pub_key_slice.to_vec()))
}

fn tx_has_input_locked_to_pub_key(tx: &BtcTransaction, btc_pub_key_slice: &BtcPubKeySlice) -> bool {
    tx.input
        .iter()
        .any(|input| sig_script_contains_pub_key(&input.script_sig, &btc_pub_key_slice))
}

fn tx_has_output_with_target_script(tx: &BtcTransaction, target_script: &BtcScript) -> bool {
    tx.output.iter().any(|output| &output.script_pubkey == target_script)
}

pub fn filter_txs_for_op_return_deposits(
    btc_address: &str,
    btc_pub_key_slice: &BtcPubKeySlice,
    transactions: &[BtcTransaction],
) -> Result<BtcTransactions> {
    info!("✔ Filtering `p2pkh` deposits that are NOT enclave's own change outputs...");
    let target_script = get_pay_to_pub_key_hash_script(&btc_address)?;
    Ok(transactions
        .iter()
        .filter(|tx| !tx_has_input_locked_to_pub_key(tx, &btc_pub_key_slice))
        .filter(|tx| tx_has_output_with_target_script(tx, &target_script))
        .cloned()
        .collect::<BtcTransactions>())
}

pub fn filter_op_return_deposit_txs_and_add_to_state<D>(state: BtcState<D>) -> Result<BtcState<D>>
where
    D: DatabaseInterface,
{
    info!("✔ Filtering `p2pkh || OP_RETURN` deposits & adding to state...");
    filter_txs_for_op_return_deposits(
        &get_btc_address_from_db(&state.db)?,
        &get_btc_public_key_slice_from_db(&state.db)?,
        &state.get_btc_block_and_id()?.block.txdata,
    )
    .and_then(|txs| {
        info!("✔ Found {} `p2pkh || OP_RETURN` deposits", txs.len());
        state.add_op_return_deposit_txs(txs)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chains::btc::{
        btc_block::BtcBlockAndId,
        btc_test_utils::{
            get_sample_btc_block_n,
            get_sample_btc_p2pkh_address,
            get_sample_btc_private_key,
            get_sample_btc_pub_key_slice,
            get_sample_testnet_block_and_txs,
            SAMPLE_TARGET_BTC_ADDRESS,
        },
        btc_utils::get_script_sig,
    };
    use bitcoin::hashes::{sha256d, Hash};
    use std::str::FromStr;

    fn get_block_with_external_p2pkh_deposit_tx() -> BtcBlockAndId {
        get_sample_testnet_block_and_txs().unwrap()
    }

    fn get_block_with_internal_p2pkh_deposit() -> BtcBlockAndId {
        get_sample_btc_block_n(7).unwrap()
    }

    fn get_tx_with_internal_p2pkh_deposit() -> BtcTransaction {
        get_block_with_internal_p2pkh_deposit().block.txdata[12].clone()
    }

    fn get_tx_with_external_p2pkh_deposit() -> BtcTransaction {
        get_block_with_external_p2pkh_deposit_tx().block.txdata[1].clone()
    }

    #[test]
    fn script_sig_should_contain_pub_key() {
        let hash_type = 1;
        let hash = sha256d::Hash::hash(b"a message");
        let btc_pk = get_sample_btc_private_key();
        let signature = btc_pk
            .sign_hash_and_append_btc_hash_type(hash.to_vec(), hash_type)
            .unwrap();
        let btc_pub_key_slice = get_sample_btc_pub_key_slice();
        let sig_script = get_script_sig(&signature, &btc_pub_key_slice);
        let result = sig_script_contains_pub_key(&sig_script, &btc_pub_key_slice);
        assert!(result);
    }

    #[test]
    fn should_not_filter_out_external_p2pkh_deposits() {
        let expected_prev_id =
            sha256d::Hash::from_str("65c5ea468d8a51e6f9120076ff0f5717b8fd1547e6311d5f89f85b21291da96f").unwrap();
        let expected_num_txs = 1;
        let block_and_id = get_block_with_external_p2pkh_deposit_tx();
        let sample_pub_key_hash = get_sample_btc_pub_key_slice();
        let sample_address = get_sample_btc_p2pkh_address();
        let filtered_txs =
            filter_txs_for_op_return_deposits(&sample_address, &sample_pub_key_hash, &block_and_id.block.txdata)
                .unwrap();
        let prev_id = filtered_txs[0].input[0].previous_output.txid;
        assert_eq!(prev_id, expected_prev_id);
        assert_eq!(filtered_txs.len(), expected_num_txs);
    }

    #[test]
    fn should_filter_out_internal_p2pkh_deposits() {
        let expected_num_txs = 0;
        let block_and_id = get_block_with_internal_p2pkh_deposit();
        let sample_pub_key_hash = get_sample_btc_pub_key_slice();
        let sample_address = get_sample_btc_p2pkh_address();
        let filtered_txs =
            filter_txs_for_op_return_deposits(&sample_address, &sample_pub_key_hash, &block_and_id.block.txdata)
                .unwrap();
        assert_eq!(filtered_txs.len(), expected_num_txs);
    }

    #[test]
    fn external_p2pkh_tx_should_have_output_with_target_script() {
        let tx = get_tx_with_external_p2pkh_deposit();
        let target_script = get_pay_to_pub_key_hash_script(&SAMPLE_TARGET_BTC_ADDRESS).unwrap();
        let result = tx_has_output_with_target_script(&tx, &target_script);
        assert!(result);
    }

    #[test]
    fn internal_p2pkh_tx_should_have_output_with_target_script() {
        let tx = get_tx_with_internal_p2pkh_deposit();
        let target_script = get_pay_to_pub_key_hash_script(&SAMPLE_TARGET_BTC_ADDRESS).unwrap();
        let result = tx_has_output_with_target_script(&tx, &target_script);
        assert!(result);
    }

    #[test]
    fn external_p2pkh_tx_should_not_have_input_locked_to_pub_key() {
        let tx = get_tx_with_external_p2pkh_deposit();
        let btc_pub_key_slice = get_sample_btc_pub_key_slice();
        let result = tx_has_input_locked_to_pub_key(&tx, &btc_pub_key_slice);
        assert!(!result);
    }

    #[test]
    fn internal_p2pkh_tx_should_have_input_locked_to_pub_key() {
        let tx = get_tx_with_internal_p2pkh_deposit();
        let btc_pub_key_slice = get_sample_btc_pub_key_slice();
        let result = tx_has_input_locked_to_pub_key(&tx, &btc_pub_key_slice);
        assert!(result);
    }
}
