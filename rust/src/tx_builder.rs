use super::*;
use super::fees;
use super::utils;
use std::collections::BTreeSet;

// comes from witsVKeyNeeded in the Ledger spec
fn witness_keys_for_cert(cert_enum: &Certificate, keys: &mut BTreeSet<Ed25519KeyHash>) {
    match &cert_enum.0 {
        // stake key registrations do not require a witness
        CertificateEnum::StakeRegistration(_cert) => {},
        CertificateEnum::StakeDeregistration(cert) => {
            keys.insert(cert.stake_credential().to_keyhash().unwrap());
        },
        CertificateEnum::StakeDelegation(cert) => {
            keys.insert(cert.stake_credential().to_keyhash().unwrap());
        },
        CertificateEnum::PoolRegistration(cert) => {
            for owner in &cert.pool_params().pool_owners().0 {
                keys.insert(owner.clone());
            }
            keys.insert(
                Ed25519KeyHash::from_bytes(cert.pool_params().operator().to_bytes()).unwrap()
            );
        },
        CertificateEnum::PoolRetirement(cert) => {
            keys.insert(
                Ed25519KeyHash::from_bytes(cert.pool_keyhash().to_bytes()).unwrap()
            );
        },
        CertificateEnum::GenesisKeyDelegation(cert) => {
            keys.insert(
                Ed25519KeyHash::from_bytes(cert.genesis_delegate_hash().to_bytes()).unwrap()
            );
        },
        // not witness as there is no single core node or genesis key that posts the certificate
        CertificateEnum::MoveInstantaneousRewardsCert(_cert) => {},
    }
}

fn min_fee(tx_builder: &TransactionBuilder) -> Result<Coin, JsError> {
    let body = tx_builder.build()?;

    let fake_key_root = Bip32PrivateKey::from_bip39_entropy(
        // art forum devote street sure rather head chuckle guard poverty release quote oak craft enemy
        &[0x0c, 0xcb, 0x74, 0xf3, 0x6b, 0x7d, 0xa1, 0x64, 0x9a, 0x81, 0x44, 0x67, 0x55, 0x22, 0xd4, 0xd8, 0x09, 0x7c, 0x64, 0x12],
        &[]
    );

    // recall: this includes keys for input, certs and withdrawals
    let vkeys = match tx_builder.input_types.vkeys.len() {
        0 => None,
        x => {
            let mut result = Vkeywitnesses::new();
            let raw_key = fake_key_root.to_raw_key();
            for _i in 0..x {
                result.add(&Vkeywitness::new(
                    &Vkey::new(&raw_key.to_public()),
                    &raw_key.sign([1u8; 100].as_ref())
                ));
            }
            Some(result)
        },
    };
    let script_keys = match tx_builder.input_types.scripts.len() {
        0 => None,
        _x => {
            // TODO: figure out how to populate fake witnesses for these
            return Err(JsError::from_str("Script inputs not supported yet"))
        },
    };
    let bootstrap_keys = match tx_builder.input_types.bootstraps.len() {
        0 => None,
        _x => {
            let mut result = BootstrapWitnesses::new();
            for addr in &tx_builder.input_types.bootstraps {
                // picking icarus over daedalus for fake witness generation shouldn't matter
                result.add(&make_icarus_bootstrap_witness(
                    &hash_transaction(&body),
                    &ByronAddress::from_bytes(addr.clone()).unwrap(),
                    &fake_key_root
                ));
            }
            Some(result)
        },
    };
    let witness_set = TransactionWitnessSet {
        vkeys: vkeys,
        scripts: script_keys,
        bootstraps: bootstrap_keys,
    };
    let full_tx = Transaction {
        body,
        witness_set,
        metadata: tx_builder.metadata.clone(),
    };
    fees::min_fee(&full_tx, &tx_builder.fee_algo)
}


// We need to know how many of each type of witness will be in the transaction so we can calculate the tx fee
#[derive(Clone, Debug)]
struct MockWitnessSet {
    vkeys: BTreeSet<Ed25519KeyHash>,
    scripts: BTreeSet<ScriptHash>,
    bootstraps: BTreeSet<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct TxBuilderInput {
    input: TransactionInput,
    amount: Value, // we need to keep track of the amount in the inputs for input selection
}

#[wasm_bindgen]
#[derive(Clone, Debug)]
pub struct TransactionBuilder {
    pool_deposit: BigNum,
    key_deposit: BigNum,
    fee_algo: fees::LinearFee,
    inputs: Vec<TxBuilderInput>,
    outputs: TransactionOutputs,
    fee: Option<Coin>,
    ttl: Option<Slot>, // absolute slot number
    certs: Option<Certificates>,
    withdrawals: Option<Withdrawals>,
    metadata: Option<TransactionMetadata>,
    validity_start_interval: Option<Slot>,
    input_types: MockWitnessSet,
    mint: Option<Mint>,
    ada_per_unit_size: BigNum,
}

#[wasm_bindgen]
impl TransactionBuilder {
    // We have to know what kind of inputs these are to know what kind of mock witnesses to create since
    // 1) mock witnesses have different lengths depending on the type which changes the expecting fee
    // 2) Witnesses are a set so we need to get rid of duplicates to avoid over-estimating the fee
    pub fn add_key_input(&mut self, hash: &Ed25519KeyHash, input: &TransactionInput, amount: &Value) {
        self.inputs.push(TxBuilderInput {
            input: input.clone(),
            amount: amount.clone(),
        });
        self.input_types.vkeys.insert(hash.clone());
    }
    pub fn add_script_input(&mut self, hash: &ScriptHash, input: &TransactionInput, amount: &Value) {
        self.inputs.push(TxBuilderInput {
            input: input.clone(),
            amount: amount.clone(),
        });
        self.input_types.scripts.insert(hash.clone());
    }
    pub fn add_bootstrap_input(&mut self, hash: &ByronAddress, input: &TransactionInput, amount: &Value) {
        self.inputs.push(TxBuilderInput {
            input: input.clone(),
            amount: amount.clone(),
        });
        self.input_types.bootstraps.insert(hash.to_bytes());
    }
    
    pub fn add_input(&mut self, address: &Address, input: &TransactionInput, amount: &Value) {
        match &BaseAddress::from_address(address) {
            Some(addr) => {
                match &addr.payment_cred().to_keyhash() {
                    Some(hash) => return self.add_key_input(hash, input, amount),
                    None => (),
                }
                match &addr.payment_cred().to_scripthash() {
                    Some(hash) => return self.add_script_input(hash, input, amount),
                    None => (),
                }
            },
            None => (),
        }
        match &EnterpriseAddress::from_address(address) {
            Some(addr) => {
                match &addr.payment_cred().to_keyhash() {
                    Some(hash) => return self.add_key_input(hash, input, amount),
                    None => (),
                }
                match &addr.payment_cred().to_scripthash() {
                    Some(hash) => return self.add_script_input(hash, input, amount),
                    None => (),
                }
            },
            None => (),
        }
        match &PointerAddress::from_address(address) {
            Some(addr) => {
                match &addr.payment_cred().to_keyhash() {
                    Some(hash) => return self.add_key_input(hash, input, amount),
                    None => (),
                }
                match &addr.payment_cred().to_scripthash() {
                    Some(hash) => return self.add_script_input(hash, input, amount),
                    None => (),
                }
            },
            None => (),
        }
        match &ByronAddress::from_address(address) {
            Some(addr) => {
                return self.add_bootstrap_input(addr, input, amount);
            },
            None => (),
        }
    }

    /// calculates how much the fee would increase if you added a given output
    pub fn fee_for_input(&mut self, address: &Address, input: &TransactionInput, amount: &Value) -> Result<Coin, JsError> {
        let mut self_copy = self.clone();

        // we need some value for these for it to be a a valid transaction
        // but since we're only calculating the different between the fee of two transactions
        // it doesn't matter what these are set as, since it cancels out
        self_copy.set_fee(&to_bignum(0));

        let fee_before = min_fee(&self_copy)?;

        self_copy.add_input(&address, &input, &amount);
        let fee_after = min_fee(&self_copy)?;
        fee_after.checked_sub(&fee_before)
    }

    pub fn add_output(&mut self, output: &TransactionOutput) -> Result<(), JsError> {
        let minimum_utxo_val = self.minimum_utxo_val()?;
        if output.amount().coin() < minimum_utxo_val {
            Err(JsError::from_str(&format!(
                "Value {} less than the minimum UTXO value {}",
                from_bignum(&output.amount().coin()),
                from_bignum(&minimum_utxo_val)
            )))
        } else {
            self.outputs.add(output);
            Ok(())
        }
    }

    /// calculates how much the fee would increase if you added a given output
    pub fn fee_for_output(&mut self, output: &TransactionOutput) -> Result<Coin, JsError> {
        let mut self_copy = self.clone();

        // we need some value for these for it to be a a valid transaction
        // but since we're only calculating the different between the fee of two transactions
        // it doesn't matter what these are set as, since it cancels out
        self_copy.set_fee(&to_bignum(0));

        let fee_before = min_fee(&self_copy)?;

        self_copy.add_output(&output)?;
        let fee_after = min_fee(&self_copy)?;
        fee_after.checked_sub(&fee_before)
    }

    pub fn set_fee(&mut self, fee: &Coin) {
        self.fee = Some(fee.clone())
    }

    pub fn set_ttl(&mut self, ttl: Slot) {
        self.ttl = Some(ttl)
    }

    pub fn set_validity_start_interval(&mut self, validity_start_interval: Slot) {
        self.validity_start_interval = Some(validity_start_interval)
    }

    pub fn set_certs(&mut self, certs: &Certificates) {
        self.certs = Some(certs.clone());
        for cert in &certs.0 {
            witness_keys_for_cert(cert, &mut self.input_types.vkeys);
        };
    }

    pub fn set_withdrawals(&mut self, withdrawals: &Withdrawals) {
        self.withdrawals = Some(withdrawals.clone());
        for (withdrawal, _coin) in &withdrawals.0 {
            self.input_types.vkeys.insert(withdrawal.payment_cred().to_keyhash().unwrap().clone());
        };
    }

    pub fn set_metadata(&mut self, metadata: &TransactionMetadata) {
        self.metadata = Some(metadata.clone())
    }

    pub fn new(
        linear_fee: &fees::LinearFee,
        ada_per_unit_size: &BigNum,
        pool_deposit: &BigNum, // protocol parameter
        key_deposit: &BigNum,  // protocol parameter
    ) -> Self {
        Self {
            ada_per_unit_size: ada_per_unit_size.clone(),
            key_deposit: key_deposit.clone(),
            pool_deposit: pool_deposit.clone(),
            fee_algo: linear_fee.clone(),
            inputs: Vec::new(),
            outputs: TransactionOutputs::new(),
            fee: None,
            ttl: None,
            certs: None,
            withdrawals: None,
            metadata: None,
            input_types: MockWitnessSet {
                vkeys: BTreeSet::new(),
                scripts: BTreeSet::new(),
                bootstraps: BTreeSet::new(),
            },
            validity_start_interval: None,
            mint: None
        }
    }

    /// does not include refunds or withdrawals
    pub fn get_explicit_input(&self) -> Result<Value, JsError> {
        self.inputs
            .iter()
            .try_fold(Value::new(to_bignum(0)), |acc, ref tx_builder_input| {
                acc.checked_add(&tx_builder_input.amount)
            })
    }
    /// withdrawals and refunds
    pub fn get_implicit_input(&self) -> Result<Value, JsError> {
        internal_get_implicit_input(
            &self.withdrawals,
            &self.certs,
            &self.pool_deposit,
            &self.key_deposit,
        )
    }

    /// does not include fee
    pub fn get_explicit_output(&self) -> Result<Value, JsError> {
        self.outputs
            .0
            .iter()
            .try_fold(Value::new(to_bignum(0)), |acc, ref output| {
                acc.checked_add(&output.amount())
            })
    }

    pub fn get_deposit(&self) -> Result<Coin, JsError> {
        internal_get_deposit(
            &self.certs,
            &self.pool_deposit,
            &self.key_deposit,
        )
    }

    pub fn get_fee_if_set(&self) -> Option<Coin> {
        self.fee.clone()
    }
    /// Warning: this function will mutate the /fee/ field
    pub fn add_change_if_needed(&mut self, address: &Address) -> Result<bool, JsError> {
        let fee = match &self.fee {
            None => self.min_fee(),
            // generating the change output involves changing the fee
            Some(_x) => return Err(JsError::from_str("Cannot calculate change if fee was explicitly specified")),
        }?;
        let input_total = self.get_explicit_input()?.checked_add(&self.get_implicit_input()?)?;
        let output_total = self.get_explicit_output()?.checked_add(&self.get_deposit()?)?;

        // the change for multiasset inputs doesn't affect the fee (because it's
        // paid in ada), so we can just compute it right away unlike the ada
        // change
        let ma_change = self.multiasset_change()?;

        match &input_total >= &output_total.checked_add(&fee)? || ma_change.is_some() {
            false => return Err(JsError::from_str("Insufficient input in transaction")),
            true => {
                // check how much the fee would increase if we added a change output

                // maximum possible output to maximize fee from adding this output
                // this may over-estimate the fee by a few bytes but that's okay
                let mut estimation_output = Value::new(to_bignum(0x1_00_00_00_00));

                if let Some(ma) = ma_change.as_ref() {
                    estimation_output.set_multiasset(ma);
                }

                let fee_for_change = self.fee_for_output(&TransactionOutput {
                    address: address.clone(),
                    amount: estimation_output,
                })?;
                let new_fee = fee.checked_add(&fee_for_change)?;
                // needs to have at least minimum_utxo_val leftover for the change to be a valid UTXO entry
                match input_total
                    >= output_total
                        .checked_add(&new_fee)?
                        .checked_add(&self.minimum_utxo_val()?)?
                {
                    false => {
                        // recall: we originally assumed the fee was the maximum possible so we definitely have enough input to cover whatever fee it ends up being
                        self.set_fee(&input_total.checked_sub(&output_total)?);
                        return Ok(false) // not enough input to covert the extra fee from adding an output so we just burn whatever is left
                    },
                    true => {
                        // recall: we originally assumed the fee was the maximum possible so we definitely have enough input to cover whatever fee it ends up being
                        self.set_fee(&new_fee);

                        let mut amount = Value::new(
                            input_total
                                .checked_sub(&output_total)?
                                .checked_sub(&new_fee)?,
                        );

                        if let Some(ma) = ma_change {
                            amount.set_multiasset(&ma);
                        }

                        self.add_output(&TransactionOutput {
                            address: address.clone(),
                            amount,
                        })?;
                    },
                };
            },
        };
        Ok(true)
    }

    pub fn build(&self) -> Result<TransactionBody, JsError> {
        let fee = self.fee.ok_or_else(|| JsError::from_str("Fee not specified"))?;
        Ok(TransactionBody {
            inputs: TransactionInputs(self.inputs.iter().map(|ref tx_builder_input| tx_builder_input.input.clone()).collect()),
            outputs: self.outputs.clone(),
            fee: fee,
            ttl: self.ttl,
            certs: self.certs.clone(),
            withdrawals: self.withdrawals.clone(),
            update: None,
            metadata_hash: match &self.metadata {
                None => None,
                Some(x) => Some(utils::hash_metadata(x)),
            },
            validity_start_interval: self.validity_start_interval,
            mint: self.mint.clone(),
        })
    }

    /// warning: sum of all parts of a transaction must equal 0. You cannot just set the fee to the min value and forget about it
    /// warning: min_fee may be slightly larger than the actual minimum fee (ex: a few lovelaces)
    /// this is done to simplify the library code, but can be fixed later
    pub fn min_fee(&self) -> Result<Coin, JsError> {
        let mut self_copy = self.clone();
        self_copy.set_fee(&to_bignum(0x1_00_00_00_00));
        min_fee(&self_copy)
    }

    fn minimum_utxo_val(&self) -> Result<Coin, JsError> {
        if self
            .outputs
            .0
            .iter()
            .any(|output| output.amount().multiasset().is_some())
        {
            Err(JsError::from_str(
                "Transaction with non-ada outputs is not implemented",
            ))
        } else {
            Ok(self.ada_per_unit_size)
        }
    }

    fn multiasset_change(&self) -> Result<Option<MultiAsset>, JsError> {
        fn merge(items: impl Iterator<Item = MultiAsset>) -> Result<MultiAsset, JsError> {
            use std::collections::btree_map::Entry;
            let mut multiasset = MultiAsset::new();

            for item in items {
                for (policy, assets) in item.0 {
                    for (asset_name, amount) in assets.0 {
                        match multiasset.0.entry(policy.clone()) {
                            Entry::Occupied(mut assets) => {
                                match assets.get_mut().0.entry(asset_name) {
                                    Entry::Occupied(mut assets) => {
                                        let current = assets.get_mut();
                                        *current = current.checked_add(&amount)?;
                                    }
                                    Entry::Vacant(vacant_entry) => {
                                        vacant_entry.insert(amount);
                                    }
                                }
                            }
                            Entry::Vacant(empty_assets) => {
                                let mut a = Assets::new();
                                a.0.insert(asset_name, amount);
                                empty_assets.insert(a);
                            }
                        }
                    }
                }
            }

            Ok(multiasset)
        }

        fn substract(mut first: MultiAsset, second: &MultiAsset) -> Result<MultiAsset, JsError> {
            for (policy, assets) in &second.0 {
                for (asset_name, amount) in &assets.0 {
                    match first.0.get_mut(policy) {
                        Some(assets) => match assets.0.get_mut(asset_name) {
                            Some(current) => match current.checked_sub(&amount) {
                                Ok(new) => *current = new,
                                Err(_) => {
                                    assets.0.remove(asset_name);
                                }
                            },
                            None => {
                                return Err(JsError::from_str("missing input for asset"));
                            }
                        },
                        None => {
                            return Err(JsError::from_str("missing input for policy"));
                        }
                    }
                }
            }

            Ok(first)
        }

        let merged_input = merge(
            self.inputs
                .iter()
                .filter_map(|input| input.amount.multiasset()),
        )?;

        let merged_output = merge(
            self.outputs
                .0
                .iter()
                .filter_map(|input| input.amount.multiasset()),
        )?;

        let output = substract(merged_input, &merged_output)?;

        Ok(Some(output).filter(|output| !output.0.is_empty()))
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use fees::*;

    fn genesis_id() -> TransactionHash {
        TransactionHash::from([0u8; TransactionHash::BYTE_COUNT])
    }

    fn root_key_15() -> Bip32PrivateKey {
        // art forum devote street sure rather head chuckle guard poverty release quote oak craft enemy
        let entropy = [0x0c, 0xcb, 0x74, 0xf3, 0x6b, 0x7d, 0xa1, 0x64, 0x9a, 0x81, 0x44, 0x67, 0x55, 0x22, 0xd4, 0xd8, 0x09, 0x7c, 0x64, 0x12];
        Bip32PrivateKey::from_bip39_entropy(&entropy, &[])
    }

    fn harden(index: u32) -> u32 {
        index | 0x80_00_00_00
    }

    #[test]
    fn build_tx_with_change() {
        let linear_fee = LinearFee::new(&to_bignum(500), &to_bignum(2));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(1), &to_bignum(1));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(NetworkInfo::testnet().network_id(), &spend_cred, &stake_cred).to_address();
        tx_builder.add_key_input(
            &spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(to_bignum(1_000_000))
        );
        tx_builder.add_output(&TransactionOutput::new(
            &addr_net_0,
            &Value::new(to_bignum(10))
        )).unwrap();
        tx_builder.set_ttl(1000);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr
        );
        assert!(added_change.unwrap());
        assert_eq!(tx_builder.outputs.len(), 2);
        assert_eq!(
            tx_builder.get_explicit_input().unwrap().checked_add(&tx_builder.get_implicit_input().unwrap()).unwrap(),
            tx_builder.get_explicit_output().unwrap().checked_add(&Value::new(tx_builder.get_fee_if_set().unwrap())).unwrap()
        );
        let _final_tx = tx_builder.build(); // just test that it doesn't throw
    }

    #[test]
    fn build_tx_without_change() {
        let linear_fee = LinearFee::new(&to_bignum(500), &to_bignum(2));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(1), &to_bignum(1));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(NetworkInfo::testnet().network_id(), &spend_cred, &stake_cred).to_address();
        tx_builder.add_key_input(
            &spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(to_bignum(1_000_000))
        );
        tx_builder.add_output(&TransactionOutput::new(
            &addr_net_0,
            &Value::new(to_bignum(880_000))
        )).unwrap();
        tx_builder.set_ttl(1000);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr
        );
        assert!(!added_change.unwrap());
        assert_eq!(tx_builder.outputs.len(), 1);
        assert_eq!(
            tx_builder.get_explicit_input().unwrap().checked_add(&tx_builder.get_implicit_input().unwrap()).unwrap(),
            tx_builder.get_explicit_output().unwrap().checked_add(&Value::new(tx_builder.get_fee_if_set().unwrap())).unwrap()
        );
        let _final_tx = tx_builder.build(); // just test that it doesn't throw
    }

    #[test]
    fn build_tx_with_certs() {
        let linear_fee = LinearFee::new(&to_bignum(500), &to_bignum(2));
        let mut tx_builder = TransactionBuilder::new(
            &linear_fee,
            &to_bignum(1),
            &to_bignum(1),
            &to_bignum(1_000_000),
        );
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        tx_builder.add_key_input(
            &spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(to_bignum(5_000_000))
        );
        tx_builder.set_ttl(1000);

        let mut certs = Certificates::new();
        certs.add(&Certificate::new_stake_registration(&StakeRegistration::new(&stake_cred)));
        certs.add(&Certificate::new_stake_delegation(&StakeDelegation::new(
            &stake_cred,
            &stake.to_raw_key().hash(), // in reality, this should be the pool owner's key, not ours
        )));
        tx_builder.set_certs(&certs);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        tx_builder.add_change_if_needed(
            &change_addr
        ).unwrap();
        assert_eq!(tx_builder.min_fee().unwrap().to_str(), "213502");
        assert_eq!(tx_builder.get_fee_if_set().unwrap().to_str(), "213502");
        assert_eq!(tx_builder.get_deposit().unwrap().to_str(), "1000000");
        assert_eq!(tx_builder.outputs.len(), 1);
        assert_eq!(
            tx_builder.get_explicit_input().unwrap().checked_add(&tx_builder.get_implicit_input().unwrap()).unwrap(),
            tx_builder
                .get_explicit_output().unwrap()
                .checked_add(&Value::new(tx_builder.get_fee_if_set().unwrap())).unwrap()
                .checked_add(&Value::new(tx_builder.get_deposit().unwrap())).unwrap()
        );
        let _final_tx = tx_builder.build(); // just test that it doesn't throw
    }

    #[test]
    fn build_tx_exact_amount() {
        // transactions where sum(input) == sum(output) exact should pass
        let linear_fee = LinearFee::new(&to_bignum(0), &to_bignum(0));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(0), &to_bignum(0));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();
        tx_builder.add_key_input(
            &&spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(to_bignum(5))
        );
        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(NetworkInfo::testnet().network_id(), &spend_cred, &stake_cred).to_address();
        tx_builder.add_output(&TransactionOutput::new(
            &addr_net_0,
            &Value::new(to_bignum(5))
        )).unwrap();
        tx_builder.set_ttl(0);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr
        ).unwrap();
        assert_eq!(added_change, false);
        let final_tx = tx_builder.build().unwrap();
        assert_eq!(final_tx.outputs().len(), 1);
    }

    #[test]
    fn build_tx_exact_change() {
        // transactions where we have exactly enough ADA to add change should pass
        let linear_fee = LinearFee::new(&to_bignum(0), &to_bignum(0));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(0), &to_bignum(0));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();
        tx_builder.add_key_input(
            &&spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(to_bignum(6))
        );
        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &spend_cred,
            &stake_cred,
        )
        .to_address();
        tx_builder
            .add_output(&TransactionOutput::new(
                &addr_net_0,
                &Value::new(to_bignum(5)),
            ))
            .unwrap();
        tx_builder.set_ttl(0);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(NetworkInfo::testnet().network_id(), &change_cred, &stake_cred).to_address();
        let added_change = tx_builder.add_change_if_needed(
            &change_addr
        ).unwrap();
        assert_eq!(added_change, true);
        let final_tx = tx_builder.build().unwrap();
        assert_eq!(final_tx.outputs().len(), 2);
        assert_eq!(final_tx.outputs().get(1).amount().coin().to_str(), "1");
    }

    #[test]
    #[should_panic]
    fn build_tx_insufficient_deposit() {
        // transactions should fail with insufficient fees if a deposit is required
        let linear_fee = LinearFee::new(&to_bignum(0), &to_bignum(0));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(0), &to_bignum(5));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();
        tx_builder.add_key_input(
            &&spend.to_raw_key().hash(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(to_bignum(5)),
        );
        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let addr_net_0 = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &spend_cred,
            &stake_cred,
        )
        .to_address();
        tx_builder
            .add_output(&TransactionOutput::new(
                &addr_net_0,
                &Value::new(to_bignum(5)),
            ))
            .unwrap();
        tx_builder.set_ttl(0);

        // add a cert which requires a deposit
        let mut certs = Certificates::new();
        certs.add(&Certificate::new_stake_registration(
            &StakeRegistration::new(&stake_cred),
        ));
        tx_builder.set_certs(&certs);

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &change_cred,
            &stake_cred,
        )
        .to_address();

        tx_builder.add_change_if_needed(&change_addr).unwrap();
    }

    #[test]
    fn build_tx_with_inputs() {
        let linear_fee = LinearFee::new(&to_bignum(500), &to_bignum(2));
        let mut tx_builder =
            TransactionBuilder::new(&linear_fee, &to_bignum(1), &to_bignum(1), &to_bignum(1));
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());
        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());

        {
            assert_eq!(tx_builder.fee_for_input(
                &EnterpriseAddress::new(
                    NetworkInfo::testnet().network_id(),
                    &spend_cred
                ).to_address(),
                &TransactionInput::new(&genesis_id(), 0),
                &Value::new(to_bignum(1_000_000))
            ).unwrap().to_str(), "69500");
            tx_builder.add_input(
                &EnterpriseAddress::new(
                    NetworkInfo::testnet().network_id(),
                    &spend_cred
                ).to_address(),
                &TransactionInput::new(&genesis_id(), 0),
                &Value::new(to_bignum(1_000_000))
            );
        }
        tx_builder.add_input(
            &BaseAddress::new(
                NetworkInfo::testnet().network_id(),
                &spend_cred,
                &stake_cred
            ).to_address(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(to_bignum(1_000_000))
        );
        tx_builder.add_input(
            &PointerAddress::new(
                NetworkInfo::testnet().network_id(),
                &spend_cred,
                &Pointer::new(
                    0,
                    0,
                    0
                )
            ).to_address(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(to_bignum(1_000_000))
        );
        tx_builder.add_input(
            &ByronAddress::icarus_from_key(
                &spend, NetworkInfo::testnet().protocol_magic()
            ).to_address(),
            &TransactionInput::new(&genesis_id(), 0),
            &Value::new(to_bignum(1_000_000))
        );

        assert_eq!(tx_builder.inputs.len(), 4);
    }

    #[test]
    fn build_tx_with_native_assets_change() {
        let linear_fee = LinearFee::new(&to_bignum(0), &to_bignum(1));
        let minimum_utxo_value = to_bignum(1);
        let mut tx_builder = TransactionBuilder::new(
            &linear_fee,
            &minimum_utxo_value,
            &to_bignum(0),
            &to_bignum(0),
        );
        let spend = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(0)
            .derive(0)
            .to_public();
        let change_key = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(1)
            .derive(0)
            .to_public();
        let stake = root_key_15()
            .derive(harden(1852))
            .derive(harden(1815))
            .derive(harden(0))
            .derive(2)
            .derive(0)
            .to_public();

        let policy_id = &PolicyID::from([0u8; 28]);
        let name = AssetName::new(vec![0u8, 1, 2, 3]).unwrap();

        let ma_input1 = 100;
        let ma_input2 = 200;
        let ma_output1 = 60;

        let multiassets = [ma_input1, ma_input2, ma_output1]
            .iter()
            .map(|input| {
                let mut multiasset = MultiAsset::new();
                multiasset.insert(policy_id, &{
                    let mut assets = Assets::new();
                    assets.insert(&name, to_bignum(*input));
                    assets
                });
                multiasset
            })
            .collect::<Vec<MultiAsset>>();

        for (multiasset, ada) in multiassets
            .iter()
            .zip([10u64, 10].iter().cloned().map(to_bignum))
        {
            let mut input_amount = Value::new(ada);
            input_amount.set_multiasset(multiasset);

            tx_builder.add_key_input(
                &&spend.to_raw_key().hash(),
                &TransactionInput::new(&genesis_id(), 0),
                &input_amount,
            );
        }

        let stake_cred = StakeCredential::from_keyhash(&stake.to_raw_key().hash());
        let spend_cred = StakeCredential::from_keyhash(&spend.to_raw_key().hash());

        let addr_net_0 = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &spend_cred,
            &stake_cred,
        )
        .to_address();

        let mut output_amount = Value::new(to_bignum(1));
        output_amount.set_multiasset(&multiassets[2]);

        tx_builder
            .add_output(&TransactionOutput::new(&addr_net_0, &output_amount))
            .unwrap();

        let change_cred = StakeCredential::from_keyhash(&change_key.to_raw_key().hash());
        let change_addr = BaseAddress::new(
            NetworkInfo::testnet().network_id(),
            &change_cred,
            &stake_cred,
        )
        .to_address();

        let added_change = tx_builder.add_change_if_needed(&change_addr).unwrap();
        assert_eq!(added_change, true);
        let final_tx = tx_builder.build().unwrap();
        assert_eq!(final_tx.outputs().len(), 2);
        assert_eq!(
            final_tx.outputs().get(0).amount().coin(),
            minimum_utxo_value
        );
        assert_eq!(
            final_tx
                .outputs()
                .get(1)
                .amount()
                .multiasset()
                .unwrap()
                .get(policy_id)
                .unwrap()
                .get(&name)
                .unwrap(),
            to_bignum(ma_input1 + ma_input2 - ma_output1)
        );
    }
}
