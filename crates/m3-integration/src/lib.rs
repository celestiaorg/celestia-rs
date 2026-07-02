//! M3 milestone acceptance tests.
//!
//! This crate keeps transaction-and-ante acceptance checks separate from the
//! lower-level conformance tests beside the M3 crates.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(test)]
mod tests {
    use ante::{
        Account, AccountReader, AccountWriter, AnteConfig, AnteContext, AnteHandler, Error,
        FeeKeeper, GasMeter, RunTxMode,
    };
    use cosmos_types::Coin;
    use prost::Message;
    use serde::Deserialize;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    const MAINNET_TX_CORPUS_FIXTURE: &str = "ante/mainnet-tx-corpus.json";
    const MIN_MAINNET_TX_CASES: usize = 1_000;

    type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

    #[derive(Debug, Deserialize)]
    struct MainnetTxCorpusFixture {
        scenario: String,
        source: String,
        chain_id: String,
        app_version: String,
        cases: Vec<MainnetTxCase>,
    }

    #[derive(Debug, Deserialize)]
    struct MainnetTxCase {
        label: String,
        network: String,
        height: i64,
        tx_index: u32,
        mode: String,
        tx_raw_hex: String,
        #[serde(default)]
        min_gas_prices: Vec<FixtureCoin>,
        #[serde(default)]
        accounts: Vec<FixtureAccount>,
        #[serde(default)]
        balances: Vec<FixtureBalance>,
        expected: ExpectedEnvelope,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct FixtureCoin {
        denom: String,
        amount: String,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct FixtureAccount {
        address_hex: String,
        account_number: u64,
        sequence: u64,
        #[serde(default)]
        public_key_any_hex: Option<String>,
    }

    #[derive(Debug, Clone, Deserialize)]
    struct FixtureBalance {
        address_hex: String,
        denom: String,
        amount: String,
    }

    #[derive(Debug, Deserialize)]
    struct ExpectedEnvelope {
        ok: bool,
        #[serde(default)]
        error: String,
        #[serde(default)]
        code: u32,
        #[serde(default)]
        codespace: String,
        #[serde(default)]
        log: String,
        #[serde(default)]
        gas_wanted: u64,
        #[serde(default)]
        gas_used: u64,
        #[serde(default)]
        events: Vec<FixtureEvent>,
        #[serde(default)]
        signer_sequences: Vec<FixtureSignerSequence>,
        #[serde(default)]
        balances: Vec<FixtureBalance>,
    }

    #[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
    struct FixtureEvent {
        #[serde(rename = "type", alias = "ty")]
        ty: String,
        #[serde(default)]
        attributes: Vec<FixtureEventAttribute>,
    }

    #[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
    struct FixtureEventAttribute {
        key: String,
        value: String,
        #[serde(default)]
        index: bool,
    }

    #[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
    struct FixtureSignerSequence {
        address_hex: String,
        sequence: u64,
    }

    #[derive(Debug, Deserialize)]
    struct GasFixture {
        chain_id: String,
        cases: Vec<GasCase>,
    }

    #[derive(Debug, Deserialize)]
    struct GasCase {
        label: String,
        mode: String,
        tx_raw_hex: String,
        address_hex: String,
        account_number: u64,
        account_sequence: u64,
        expected_ok: bool,
        #[serde(default)]
        expected_gas_wanted: u64,
        #[serde(default)]
        expected_gas_used: u64,
        #[serde(default)]
        expected_final_sequence: u64,
    }

    #[test]
    #[ignore = "pending Go-generated ante/mainnet-tx-corpus.json fixture"]
    fn mainnet_tx_validation_matches_go() -> TestResult<()> {
        let fixture: MainnetTxCorpusFixture = load_fixture(MAINNET_TX_CORPUS_FIXTURE)?;
        run_mainnet_tx_fixture(&fixture, MIN_MAINNET_TX_CASES)
    }

    #[test]
    fn mainnet_tx_runner_accepts_synthetic_success_case() -> TestResult<()> {
        let fixture = synthetic_success_fixture()?;
        run_mainnet_tx_fixture(&fixture, 1)
    }

    fn run_mainnet_tx_fixture(
        fixture: &MainnetTxCorpusFixture,
        min_cases: usize,
    ) -> TestResult<()> {
        assert_eq!(fixture.scenario, "ante-mainnet-tx-corpus");
        assert!(
            !fixture.source.is_empty(),
            "fixture source must be recorded"
        );
        assert!(
            !fixture.app_version.is_empty(),
            "fixture app version must be recorded"
        );

        assert!(
            fixture.cases.len() >= min_cases,
            "mainnet tx corpus must contain at least {min_cases} cases"
        );

        for case in &fixture.cases {
            let name = case_name(case);
            assert_eq!(case.network, "mainnet", "{name}");

            let mut config = AnteConfig::celestia_mainnet_app_v6(&fixture.chain_id);
            config.min_gas_prices = case
                .min_gas_prices
                .iter()
                .map(coin_from_fixture)
                .collect::<TestResult<Vec<_>>>()?;
            let handler = AnteHandler::new(config);
            let tx_bytes = decode_hex(&case.tx_raw_hex, &name, "tx raw")?;
            let mut accounts = CorpusAccounts::from_fixture(&case.accounts, &name)?;
            let mut fees = CorpusFees::from_fixture(&case.balances, &name)?;

            let result = handler.run(
                run_context(&case.mode, case.height)?,
                &tx_bytes,
                &mut accounts,
                &mut fees,
            );
            compare_result(case, &name, result, &fees)?;
        }

        Ok(())
    }

    fn synthetic_success_fixture() -> TestResult<MainnetTxCorpusFixture> {
        let gas_fixture: GasFixture = load_fixture("ante/gas.json")?;
        let case = gas_fixture
            .cases
            .iter()
            .find(|case| case.label == "deliver-charges-tx-size-and-signature")
            .ok_or_else(|| invalid_data("missing ante gas success fixture"))?;
        if !case.expected_ok {
            return Err(invalid_data(format!(
                "{}: expected source gas fixture to be successful",
                case.label
            ))
            .into());
        }

        Ok(MainnetTxCorpusFixture {
            scenario: "ante-mainnet-tx-corpus".to_owned(),
            source: "synthetic smoke case translated from ante/gas.json".to_owned(),
            chain_id: gas_fixture.chain_id,
            app_version: "synthetic".to_owned(),
            cases: vec![MainnetTxCase {
                label: case.label.clone(),
                network: "mainnet".to_owned(),
                height: 10,
                tx_index: 0,
                mode: case.mode.clone(),
                tx_raw_hex: case.tx_raw_hex.clone(),
                min_gas_prices: Vec::new(),
                accounts: vec![FixtureAccount {
                    address_hex: case.address_hex.clone(),
                    account_number: case.account_number,
                    sequence: case.account_sequence,
                    public_key_any_hex: None,
                }],
                balances: Vec::new(),
                expected: ExpectedEnvelope {
                    ok: true,
                    error: String::new(),
                    code: 0,
                    codespace: String::new(),
                    log: String::new(),
                    gas_wanted: case.expected_gas_wanted,
                    gas_used: case.expected_gas_used,
                    events: Vec::new(),
                    signer_sequences: vec![FixtureSignerSequence {
                        address_hex: case.address_hex.clone(),
                        sequence: case.expected_final_sequence,
                    }],
                    balances: Vec::new(),
                },
            }],
        })
    }

    fn load_fixture<T>(relative_path: &str) -> TestResult<T>
    where
        T: for<'de> Deserialize<'de>,
    {
        let path = fixture_path(relative_path)?;
        let body = std::fs::read_to_string(&path)?;
        serde_json::from_str(&body).map_err(|error| {
            invalid_data(format!("parse fixture {}: {error}", path.display())).into()
        })
    }

    fn fixture_path(relative_path: &str) -> TestResult<PathBuf> {
        Ok(workspace_root()?
            .join("conformance/fixtures")
            .join(relative_path))
    }

    fn workspace_root() -> TestResult<PathBuf> {
        let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        for _ in 0..10 {
            let manifest = dir.join("Cargo.toml");
            if manifest.exists() {
                let body = std::fs::read_to_string(&manifest)?;
                if body.contains("[workspace]") {
                    return Ok(dir);
                }
            }
            if !dir.pop() {
                break;
            }
        }
        Err("no workspace Cargo.toml with [workspace] found above CARGO_MANIFEST_DIR".into())
    }

    fn compare_result(
        case: &MainnetTxCase,
        name: &str,
        result: ante::Result<ante::AnteOutcome>,
        fees: &CorpusFees,
    ) -> TestResult<()> {
        match (case.expected.ok, result) {
            (true, Ok(outcome)) => {
                require_supported_go_result_fields(&case.expected, name)?;
                assert_eq!(outcome.gas_wanted, case.expected.gas_wanted, "{name}");
                assert_eq!(outcome.gas_used, case.expected.gas_used, "{name}");
                assert_eq!(
                    events_from_outcome(&outcome.events),
                    case.expected.events,
                    "{name}"
                );
                assert_eq!(
                    signer_sequences_from_outcome(&outcome.signer_sequences),
                    case.expected.signer_sequences,
                    "{name}"
                );
                assert_eq!(
                    fees.snapshot(),
                    balances_from_fixture(&case.expected.balances, name)?,
                    "{name}"
                );
            }
            (true, Err(error)) => {
                return Err(invalid_data(format!(
                    "{name}: Rust ante unexpectedly rejected tx with {error}"
                ))
                .into());
            }
            (false, Err(error)) => {
                assert_eq!(error_code(&error), case.expected.error, "{name}");
                require_supported_rejection_fields(&case.expected, name)?;
                assert_eq!(
                    fees.snapshot(),
                    balances_from_fixture(&case.expected.balances, name)?,
                    "{name}"
                );
            }
            (false, Ok(outcome)) => {
                return Err(invalid_data(format!(
                    "{name}: Rust ante unexpectedly accepted tx with outcome {outcome:?}"
                ))
                .into());
            }
        }
        Ok(())
    }

    fn require_supported_go_result_fields(
        expected: &ExpectedEnvelope,
        name: &str,
    ) -> TestResult<()> {
        if expected.code == 0 && expected.codespace.is_empty() && expected.log.is_empty() {
            return Ok(());
        }
        Err(invalid_data(format!(
            "{name}: fixture includes Go ABCI code/codespace/log fields; extend the Rust ante outcome comparison before enabling this corpus"
        ))
        .into())
    }

    fn require_supported_rejection_fields(
        expected: &ExpectedEnvelope,
        name: &str,
    ) -> TestResult<()> {
        require_supported_go_result_fields(expected, name)?;
        if expected.gas_wanted == 0
            && expected.gas_used == 0
            && expected.events.is_empty()
            && expected.signer_sequences.is_empty()
        {
            return Ok(());
        }
        Err(invalid_data(format!(
            "{name}: fixture includes rejected-tx gas, events, or signer state fields; extend the Rust ante error comparison before enabling this corpus"
        ))
        .into())
    }

    fn run_context(mode: &str, block_height: i64) -> TestResult<AnteContext> {
        let mode = match mode {
            "check" => RunTxMode::Check,
            "recheck" => RunTxMode::Recheck,
            "deliver" => RunTxMode::Deliver,
            "simulate" => RunTxMode::Simulate,
            other => {
                return Err(invalid_data(format!("unknown run tx mode `{other}`")).into());
            }
        };
        Ok(AnteContext {
            mode,
            block_height,
            gas_meter: GasMeter::infinite(),
            events: Vec::new(),
        })
    }

    fn coin_from_fixture(coin: &FixtureCoin) -> TestResult<Coin> {
        Coin::new(&coin.denom, &coin.amount).map_err(|error| {
            invalid_data(format!(
                "invalid fixture coin {}{}: {error}",
                coin.amount, coin.denom
            ))
            .into()
        })
    }

    fn balances_from_fixture(
        balances: &[FixtureBalance],
        label: &str,
    ) -> TestResult<BTreeMap<(cosmos_types::AccountAddress, String), Coin>> {
        let mut out = BTreeMap::new();
        for balance in balances {
            let address = decode_address(&balance.address_hex, label)?;
            let coin = Coin::new(&balance.denom, &balance.amount).map_err(|error| {
                invalid_data(format!(
                    "{label}: invalid fixture balance {}{}: {error}",
                    balance.amount, balance.denom
                ))
            })?;
            if out.insert((address, balance.denom.clone()), coin).is_some() {
                return Err(invalid_data(format!(
                    "{label}: duplicate fixture balance for {}{}",
                    balance.address_hex, balance.denom
                ))
                .into());
            }
        }
        Ok(out)
    }

    fn events_from_outcome(events: &[abci_types::Event]) -> Vec<FixtureEvent> {
        events
            .iter()
            .map(|event| FixtureEvent {
                ty: event.ty.clone(),
                attributes: event
                    .attributes
                    .iter()
                    .map(|attribute| FixtureEventAttribute {
                        key: attribute.key.clone(),
                        value: attribute.value.clone(),
                        index: attribute.index,
                    })
                    .collect(),
            })
            .collect()
    }

    fn signer_sequences_from_outcome(
        sequences: &[(cosmos_types::AccountAddress, u64)],
    ) -> Vec<FixtureSignerSequence> {
        sequences
            .iter()
            .map(|(address, sequence)| FixtureSignerSequence {
                address_hex: hex::encode(address),
                sequence: *sequence,
            })
            .collect()
    }

    fn decode_hex(input: &str, label: &str, field: &str) -> TestResult<Vec<u8>> {
        hex::decode(input).map_err(|error| {
            invalid_data(format!("decode {field} hex for {label}: {error}")).into()
        })
    }

    fn decode_address(input: &str, label: &str) -> TestResult<cosmos_types::AccountAddress> {
        let bytes = decode_hex(input, label, "address")?;
        bytes
            .as_slice()
            .try_into()
            .map_err(|_| invalid_data(format!("{label}: address length must be 20 bytes")).into())
    }

    fn decode_optional_any(
        input: Option<&String>,
        label: &str,
        field: &str,
    ) -> TestResult<Option<cosmos_types::Any>> {
        let Some(input) = input else {
            return Ok(None);
        };
        if input.is_empty() {
            return Ok(None);
        }
        let bytes = decode_hex(input, label, field)?;
        let any = cosmos_types::proto::google::Any::decode(bytes.as_slice()).map_err(|error| {
            invalid_data(format!("decode {field} protobuf for {label}: {error}"))
        })?;
        cosmos_types::Any::from_proto(any)
            .map(Some)
            .map_err(|error| invalid_data(format!("convert {field} for {label}: {error}")).into())
    }

    fn error_code(error: &Error) -> &'static str {
        match error {
            Error::MalformedTx => "malformed_tx",
            Error::UnsupportedSignMode => "unsupported_sign_mode",
            Error::MissingSigner => "missing_signer",
            Error::SignatureVerification => "signature_verification",
            Error::AccountNotFound => "account_not_found",
            Error::AccountNumberMismatch => "account_number_mismatch",
            Error::SequenceMismatch => "sequence_mismatch",
            Error::InsufficientFee => "insufficient_fee",
            Error::InsufficientFunds => "insufficient_funds",
            Error::OutOfGas => "out_of_gas",
            Error::GasOverflow => "gas_overflow",
            Error::InvalidMemo => "invalid_memo",
            Error::InvalidPublicKey => "invalid_public_key",
            Error::State(_) => "state",
        }
    }

    fn case_name(case: &MainnetTxCase) -> String {
        format!(
            "{} at height {} tx {} mode {}",
            case.label, case.height, case.tx_index, case.mode
        )
    }

    fn invalid_data(message: impl Into<String>) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::InvalidData, message.into())
    }

    #[derive(Debug, Default)]
    struct CorpusAccounts {
        accounts: BTreeMap<cosmos_types::AccountAddress, Account>,
    }

    impl CorpusAccounts {
        fn from_fixture(accounts: &[FixtureAccount], label: &str) -> TestResult<Self> {
            let mut out = BTreeMap::new();
            for account in accounts {
                let address = decode_address(&account.address_hex, label)?;
                let account = Account {
                    address,
                    account_number: account.account_number,
                    sequence: account.sequence,
                    public_key: decode_optional_any(
                        account.public_key_any_hex.as_ref(),
                        label,
                        "account public key",
                    )?,
                };
                if out.insert(address, account).is_some() {
                    return Err(invalid_data(format!(
                        "{label}: duplicate fixture account {}",
                        hex::encode(address)
                    ))
                    .into());
                }
            }
            Ok(Self { accounts: out })
        }
    }

    impl AccountReader for CorpusAccounts {
        fn account(&self, address: &cosmos_types::AccountAddress) -> ante::Result<Option<Account>> {
            Ok(self.accounts.get(address).cloned())
        }
    }

    impl AccountWriter for CorpusAccounts {
        fn set_sequence(
            &mut self,
            address: &cosmos_types::AccountAddress,
            sequence: u64,
        ) -> ante::Result<()> {
            let account = self
                .accounts
                .get_mut(address)
                .ok_or(Error::AccountNotFound)?;
            account.sequence = sequence;
            Ok(())
        }

        fn set_public_key(
            &mut self,
            address: &cosmos_types::AccountAddress,
            public_key: cosmos_types::Any,
        ) -> ante::Result<()> {
            let account = self
                .accounts
                .get_mut(address)
                .ok_or(Error::AccountNotFound)?;
            account.public_key = Some(public_key);
            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct CorpusFees {
        balances: BTreeMap<(cosmos_types::AccountAddress, String), Coin>,
    }

    impl CorpusFees {
        fn from_fixture(balances: &[FixtureBalance], label: &str) -> TestResult<Self> {
            Ok(Self {
                balances: balances_from_fixture(balances, label)?,
            })
        }

        fn snapshot(&self) -> BTreeMap<(cosmos_types::AccountAddress, String), Coin> {
            self.balances.clone()
        }
    }

    impl FeeKeeper for CorpusFees {
        fn spendable_balance(
            &self,
            address: &cosmos_types::AccountAddress,
            denom: &str,
        ) -> ante::Result<Option<Coin>> {
            Ok(self.balances.get(&(*address, denom.to_owned())).cloned())
        }

        fn deduct_fees(
            &mut self,
            payer: &cosmos_types::AccountAddress,
            fees: &[Coin],
        ) -> ante::Result<()> {
            for fee in fees {
                let balance = self
                    .balances
                    .get_mut(&(*payer, fee.denom.clone()))
                    .ok_or(Error::InsufficientFunds)?;
                let remaining = parse_amount(&balance.amount)?
                    .checked_sub(parse_amount(&fee.amount)?)
                    .ok_or(Error::InsufficientFunds)?;
                balance.amount = remaining.to_string();
            }
            Ok(())
        }
    }

    fn parse_amount(input: &str) -> ante::Result<u128> {
        input.parse().map_err(|_| Error::InsufficientFunds)
    }
}
