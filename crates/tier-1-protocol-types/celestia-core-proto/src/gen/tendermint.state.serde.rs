impl serde::Serialize for AbciResponsesInfo {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.state.ABCIResponsesInfo", len)?;
        if let Some(v) = self.legacy_abci_responses.as_ref() {
            struct_ser.serialize_field("legacy_abci_responses", v)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("height", ToString::to_string(&self.height).as_str())?;
        }
        if let Some(v) = self.response_finalize_block.as_ref() {
            struct_ser.serialize_field("response_finalize_block", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for AbciResponsesInfo {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "legacy_abci_responses",
            "legacyAbciResponses",
            "height",
            "response_finalize_block",
            "responseFinalizeBlock",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            LegacyAbciResponses,
            Height,
            ResponseFinalizeBlock,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "legacyAbciResponses" | "legacy_abci_responses" => Ok(GeneratedField::LegacyAbciResponses),
                            "height" => Ok(GeneratedField::Height),
                            "responseFinalizeBlock" | "response_finalize_block" => Ok(GeneratedField::ResponseFinalizeBlock),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = AbciResponsesInfo;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.state.ABCIResponsesInfo")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<AbciResponsesInfo, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut legacy_abci_responses__ = None;
                let mut height__ = None;
                let mut response_finalize_block__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::LegacyAbciResponses => {
                            if legacy_abci_responses__.is_some() {
                                return Err(serde::de::Error::duplicate_field("legacyAbciResponses"));
                            }
                            legacy_abci_responses__ = map_.next_value()?;
                        }
                        GeneratedField::Height => {
                            if height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("height"));
                            }
                            height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ResponseFinalizeBlock => {
                            if response_finalize_block__.is_some() {
                                return Err(serde::de::Error::duplicate_field("responseFinalizeBlock"));
                            }
                            response_finalize_block__ = map_.next_value()?;
                        }
                    }
                }
                Ok(AbciResponsesInfo {
                    legacy_abci_responses: legacy_abci_responses__,
                    height: height__.unwrap_or_default(),
                    response_finalize_block: response_finalize_block__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.state.ABCIResponsesInfo", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ConsensusParamsInfo {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.state.ConsensusParamsInfo", len)?;
        if let Some(v) = self.consensus_params.as_ref() {
            struct_ser.serialize_field("consensus_params", v)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("last_height_changed", ToString::to_string(&self.last_height_changed).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ConsensusParamsInfo {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "consensus_params",
            "consensusParams",
            "last_height_changed",
            "lastHeightChanged",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ConsensusParams,
            LastHeightChanged,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "consensusParams" | "consensus_params" => Ok(GeneratedField::ConsensusParams),
                            "lastHeightChanged" | "last_height_changed" => Ok(GeneratedField::LastHeightChanged),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ConsensusParamsInfo;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.state.ConsensusParamsInfo")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ConsensusParamsInfo, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut consensus_params__ = None;
                let mut last_height_changed__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ConsensusParams => {
                            if consensus_params__.is_some() {
                                return Err(serde::de::Error::duplicate_field("consensusParams"));
                            }
                            consensus_params__ = map_.next_value()?;
                        }
                        GeneratedField::LastHeightChanged => {
                            if last_height_changed__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastHeightChanged"));
                            }
                            last_height_changed__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(ConsensusParamsInfo {
                    consensus_params: consensus_params__,
                    last_height_changed: last_height_changed__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.state.ConsensusParamsInfo", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for LegacyAbciResponses {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.state.LegacyABCIResponses", len)?;
        if true {
            struct_ser.serialize_field("deliver_txs", &self.deliver_txs)?;
        }
        if let Some(v) = self.end_block.as_ref() {
            struct_ser.serialize_field("end_block", v)?;
        }
        if let Some(v) = self.begin_block.as_ref() {
            struct_ser.serialize_field("begin_block", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for LegacyAbciResponses {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "deliver_txs",
            "deliverTxs",
            "end_block",
            "endBlock",
            "begin_block",
            "beginBlock",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            DeliverTxs,
            EndBlock,
            BeginBlock,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "deliverTxs" | "deliver_txs" => Ok(GeneratedField::DeliverTxs),
                            "endBlock" | "end_block" => Ok(GeneratedField::EndBlock),
                            "beginBlock" | "begin_block" => Ok(GeneratedField::BeginBlock),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = LegacyAbciResponses;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.state.LegacyABCIResponses")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<LegacyAbciResponses, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut deliver_txs__ = None;
                let mut end_block__ = None;
                let mut begin_block__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::DeliverTxs => {
                            if deliver_txs__.is_some() {
                                return Err(serde::de::Error::duplicate_field("deliverTxs"));
                            }
                            deliver_txs__ = Some(map_.next_value()?);
                        }
                        GeneratedField::EndBlock => {
                            if end_block__.is_some() {
                                return Err(serde::de::Error::duplicate_field("endBlock"));
                            }
                            end_block__ = map_.next_value()?;
                        }
                        GeneratedField::BeginBlock => {
                            if begin_block__.is_some() {
                                return Err(serde::de::Error::duplicate_field("beginBlock"));
                            }
                            begin_block__ = map_.next_value()?;
                        }
                    }
                }
                Ok(LegacyAbciResponses {
                    deliver_txs: deliver_txs__.unwrap_or_default(),
                    end_block: end_block__,
                    begin_block: begin_block__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.state.LegacyABCIResponses", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ResponseBeginBlock {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.state.ResponseBeginBlock", len)?;
        if true {
            struct_ser.serialize_field("events", &self.events)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ResponseBeginBlock {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "events",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Events,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "events" => Ok(GeneratedField::Events),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ResponseBeginBlock;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.state.ResponseBeginBlock")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ResponseBeginBlock, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut events__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Events => {
                            if events__.is_some() {
                                return Err(serde::de::Error::duplicate_field("events"));
                            }
                            events__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ResponseBeginBlock {
                    events: events__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.state.ResponseBeginBlock", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ResponseEndBlock {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.state.ResponseEndBlock", len)?;
        if true {
            struct_ser.serialize_field("validator_updates", &self.validator_updates)?;
        }
        if let Some(v) = self.consensus_param_updates.as_ref() {
            struct_ser.serialize_field("consensus_param_updates", v)?;
        }
        if true {
            struct_ser.serialize_field("events", &self.events)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ResponseEndBlock {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "validator_updates",
            "validatorUpdates",
            "consensus_param_updates",
            "consensusParamUpdates",
            "events",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ValidatorUpdates,
            ConsensusParamUpdates,
            Events,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "validatorUpdates" | "validator_updates" => Ok(GeneratedField::ValidatorUpdates),
                            "consensusParamUpdates" | "consensus_param_updates" => Ok(GeneratedField::ConsensusParamUpdates),
                            "events" => Ok(GeneratedField::Events),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ResponseEndBlock;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.state.ResponseEndBlock")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ResponseEndBlock, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut validator_updates__ = None;
                let mut consensus_param_updates__ = None;
                let mut events__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ValidatorUpdates => {
                            if validator_updates__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validatorUpdates"));
                            }
                            validator_updates__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ConsensusParamUpdates => {
                            if consensus_param_updates__.is_some() {
                                return Err(serde::de::Error::duplicate_field("consensusParamUpdates"));
                            }
                            consensus_param_updates__ = map_.next_value()?;
                        }
                        GeneratedField::Events => {
                            if events__.is_some() {
                                return Err(serde::de::Error::duplicate_field("events"));
                            }
                            events__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(ResponseEndBlock {
                    validator_updates: validator_updates__.unwrap_or_default(),
                    consensus_param_updates: consensus_param_updates__,
                    events: events__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.state.ResponseEndBlock", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for State {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.state.State", len)?;
        if let Some(v) = self.version.as_ref() {
            struct_ser.serialize_field("version", v)?;
        }
        if true {
            struct_ser.serialize_field("chain_id", &self.chain_id)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("initial_height", ToString::to_string(&self.initial_height).as_str())?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("last_block_height", ToString::to_string(&self.last_block_height).as_str())?;
        }
        if let Some(v) = self.last_block_id.as_ref() {
            struct_ser.serialize_field("last_block_id", v)?;
        }
        if let Some(v) = self.last_block_time.as_ref() {
            struct_ser.serialize_field("last_block_time", v)?;
        }
        if let Some(v) = self.next_validators.as_ref() {
            struct_ser.serialize_field("next_validators", v)?;
        }
        if let Some(v) = self.validators.as_ref() {
            struct_ser.serialize_field("validators", v)?;
        }
        if let Some(v) = self.last_validators.as_ref() {
            struct_ser.serialize_field("last_validators", v)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("last_height_validators_changed", ToString::to_string(&self.last_height_validators_changed).as_str())?;
        }
        if let Some(v) = self.consensus_params.as_ref() {
            struct_ser.serialize_field("consensus_params", v)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("last_height_consensus_params_changed", ToString::to_string(&self.last_height_consensus_params_changed).as_str())?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("last_results_hash", pbjson::private::base64::encode(&self.last_results_hash).as_str())?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("app_hash", pbjson::private::base64::encode(&self.app_hash).as_str())?;
        }
        if let Some(v) = self.timeout_info.as_ref() {
            struct_ser.serialize_field("timeout_info", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for State {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "version",
            "chain_id",
            "chainId",
            "initial_height",
            "initialHeight",
            "last_block_height",
            "lastBlockHeight",
            "last_block_id",
            "lastBlockId",
            "last_block_time",
            "lastBlockTime",
            "next_validators",
            "nextValidators",
            "validators",
            "last_validators",
            "lastValidators",
            "last_height_validators_changed",
            "lastHeightValidatorsChanged",
            "consensus_params",
            "consensusParams",
            "last_height_consensus_params_changed",
            "lastHeightConsensusParamsChanged",
            "last_results_hash",
            "lastResultsHash",
            "app_hash",
            "appHash",
            "timeout_info",
            "timeoutInfo",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Version,
            ChainId,
            InitialHeight,
            LastBlockHeight,
            LastBlockId,
            LastBlockTime,
            NextValidators,
            Validators,
            LastValidators,
            LastHeightValidatorsChanged,
            ConsensusParams,
            LastHeightConsensusParamsChanged,
            LastResultsHash,
            AppHash,
            TimeoutInfo,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "version" => Ok(GeneratedField::Version),
                            "chainId" | "chain_id" => Ok(GeneratedField::ChainId),
                            "initialHeight" | "initial_height" => Ok(GeneratedField::InitialHeight),
                            "lastBlockHeight" | "last_block_height" => Ok(GeneratedField::LastBlockHeight),
                            "lastBlockId" | "last_block_id" => Ok(GeneratedField::LastBlockId),
                            "lastBlockTime" | "last_block_time" => Ok(GeneratedField::LastBlockTime),
                            "nextValidators" | "next_validators" => Ok(GeneratedField::NextValidators),
                            "validators" => Ok(GeneratedField::Validators),
                            "lastValidators" | "last_validators" => Ok(GeneratedField::LastValidators),
                            "lastHeightValidatorsChanged" | "last_height_validators_changed" => Ok(GeneratedField::LastHeightValidatorsChanged),
                            "consensusParams" | "consensus_params" => Ok(GeneratedField::ConsensusParams),
                            "lastHeightConsensusParamsChanged" | "last_height_consensus_params_changed" => Ok(GeneratedField::LastHeightConsensusParamsChanged),
                            "lastResultsHash" | "last_results_hash" => Ok(GeneratedField::LastResultsHash),
                            "appHash" | "app_hash" => Ok(GeneratedField::AppHash),
                            "timeoutInfo" | "timeout_info" => Ok(GeneratedField::TimeoutInfo),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = State;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.state.State")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<State, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut version__ = None;
                let mut chain_id__ = None;
                let mut initial_height__ = None;
                let mut last_block_height__ = None;
                let mut last_block_id__ = None;
                let mut last_block_time__ = None;
                let mut next_validators__ = None;
                let mut validators__ = None;
                let mut last_validators__ = None;
                let mut last_height_validators_changed__ = None;
                let mut consensus_params__ = None;
                let mut last_height_consensus_params_changed__ = None;
                let mut last_results_hash__ = None;
                let mut app_hash__ = None;
                let mut timeout_info__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Version => {
                            if version__.is_some() {
                                return Err(serde::de::Error::duplicate_field("version"));
                            }
                            version__ = map_.next_value()?;
                        }
                        GeneratedField::ChainId => {
                            if chain_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("chainId"));
                            }
                            chain_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::InitialHeight => {
                            if initial_height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("initialHeight"));
                            }
                            initial_height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LastBlockHeight => {
                            if last_block_height__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastBlockHeight"));
                            }
                            last_block_height__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LastBlockId => {
                            if last_block_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastBlockId"));
                            }
                            last_block_id__ = map_.next_value()?;
                        }
                        GeneratedField::LastBlockTime => {
                            if last_block_time__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastBlockTime"));
                            }
                            last_block_time__ = map_.next_value()?;
                        }
                        GeneratedField::NextValidators => {
                            if next_validators__.is_some() {
                                return Err(serde::de::Error::duplicate_field("nextValidators"));
                            }
                            next_validators__ = map_.next_value()?;
                        }
                        GeneratedField::Validators => {
                            if validators__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validators"));
                            }
                            validators__ = map_.next_value()?;
                        }
                        GeneratedField::LastValidators => {
                            if last_validators__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastValidators"));
                            }
                            last_validators__ = map_.next_value()?;
                        }
                        GeneratedField::LastHeightValidatorsChanged => {
                            if last_height_validators_changed__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastHeightValidatorsChanged"));
                            }
                            last_height_validators_changed__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ConsensusParams => {
                            if consensus_params__.is_some() {
                                return Err(serde::de::Error::duplicate_field("consensusParams"));
                            }
                            consensus_params__ = map_.next_value()?;
                        }
                        GeneratedField::LastHeightConsensusParamsChanged => {
                            if last_height_consensus_params_changed__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastHeightConsensusParamsChanged"));
                            }
                            last_height_consensus_params_changed__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::LastResultsHash => {
                            if last_results_hash__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastResultsHash"));
                            }
                            last_results_hash__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::AppHash => {
                            if app_hash__.is_some() {
                                return Err(serde::de::Error::duplicate_field("appHash"));
                            }
                            app_hash__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::TimeoutInfo => {
                            if timeout_info__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutInfo"));
                            }
                            timeout_info__ = map_.next_value()?;
                        }
                    }
                }
                Ok(State {
                    version: version__,
                    chain_id: chain_id__.unwrap_or_default(),
                    initial_height: initial_height__.unwrap_or_default(),
                    last_block_height: last_block_height__.unwrap_or_default(),
                    last_block_id: last_block_id__,
                    last_block_time: last_block_time__,
                    next_validators: next_validators__,
                    validators: validators__,
                    last_validators: last_validators__,
                    last_height_validators_changed: last_height_validators_changed__.unwrap_or_default(),
                    consensus_params: consensus_params__,
                    last_height_consensus_params_changed: last_height_consensus_params_changed__.unwrap_or_default(),
                    last_results_hash: last_results_hash__.unwrap_or_default(),
                    app_hash: app_hash__.unwrap_or_default(),
                    timeout_info: timeout_info__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.state.State", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for TimeoutInfo {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.state.TimeoutInfo", len)?;
        if let Some(v) = self.timeout_propose.as_ref() {
            struct_ser.serialize_field("timeout_propose", v)?;
        }
        if let Some(v) = self.timeout_commit.as_ref() {
            struct_ser.serialize_field("timeout_commit", v)?;
        }
        if let Some(v) = self.timeout_propose_delta.as_ref() {
            struct_ser.serialize_field("timeout_propose_delta", v)?;
        }
        if let Some(v) = self.timeout_prevote.as_ref() {
            struct_ser.serialize_field("timeout_prevote", v)?;
        }
        if let Some(v) = self.timeout_prevote_delta.as_ref() {
            struct_ser.serialize_field("timeout_prevote_delta", v)?;
        }
        if let Some(v) = self.timeout_precommit.as_ref() {
            struct_ser.serialize_field("timeout_precommit", v)?;
        }
        if let Some(v) = self.timeout_precommit_delta.as_ref() {
            struct_ser.serialize_field("timeout_precommit_delta", v)?;
        }
        if let Some(v) = self.delayed_precommit_timeout.as_ref() {
            struct_ser.serialize_field("delayed_precommit_timeout", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for TimeoutInfo {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "timeout_propose",
            "timeoutPropose",
            "timeout_commit",
            "timeoutCommit",
            "timeout_propose_delta",
            "timeoutProposeDelta",
            "timeout_prevote",
            "timeoutPrevote",
            "timeout_prevote_delta",
            "timeoutPrevoteDelta",
            "timeout_precommit",
            "timeoutPrecommit",
            "timeout_precommit_delta",
            "timeoutPrecommitDelta",
            "delayed_precommit_timeout",
            "delayedPrecommitTimeout",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TimeoutPropose,
            TimeoutCommit,
            TimeoutProposeDelta,
            TimeoutPrevote,
            TimeoutPrevoteDelta,
            TimeoutPrecommit,
            TimeoutPrecommitDelta,
            DelayedPrecommitTimeout,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "timeoutPropose" | "timeout_propose" => Ok(GeneratedField::TimeoutPropose),
                            "timeoutCommit" | "timeout_commit" => Ok(GeneratedField::TimeoutCommit),
                            "timeoutProposeDelta" | "timeout_propose_delta" => Ok(GeneratedField::TimeoutProposeDelta),
                            "timeoutPrevote" | "timeout_prevote" => Ok(GeneratedField::TimeoutPrevote),
                            "timeoutPrevoteDelta" | "timeout_prevote_delta" => Ok(GeneratedField::TimeoutPrevoteDelta),
                            "timeoutPrecommit" | "timeout_precommit" => Ok(GeneratedField::TimeoutPrecommit),
                            "timeoutPrecommitDelta" | "timeout_precommit_delta" => Ok(GeneratedField::TimeoutPrecommitDelta),
                            "delayedPrecommitTimeout" | "delayed_precommit_timeout" => Ok(GeneratedField::DelayedPrecommitTimeout),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = TimeoutInfo;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.state.TimeoutInfo")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<TimeoutInfo, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut timeout_propose__ = None;
                let mut timeout_commit__ = None;
                let mut timeout_propose_delta__ = None;
                let mut timeout_prevote__ = None;
                let mut timeout_prevote_delta__ = None;
                let mut timeout_precommit__ = None;
                let mut timeout_precommit_delta__ = None;
                let mut delayed_precommit_timeout__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TimeoutPropose => {
                            if timeout_propose__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutPropose"));
                            }
                            timeout_propose__ = map_.next_value()?;
                        }
                        GeneratedField::TimeoutCommit => {
                            if timeout_commit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutCommit"));
                            }
                            timeout_commit__ = map_.next_value()?;
                        }
                        GeneratedField::TimeoutProposeDelta => {
                            if timeout_propose_delta__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutProposeDelta"));
                            }
                            timeout_propose_delta__ = map_.next_value()?;
                        }
                        GeneratedField::TimeoutPrevote => {
                            if timeout_prevote__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutPrevote"));
                            }
                            timeout_prevote__ = map_.next_value()?;
                        }
                        GeneratedField::TimeoutPrevoteDelta => {
                            if timeout_prevote_delta__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutPrevoteDelta"));
                            }
                            timeout_prevote_delta__ = map_.next_value()?;
                        }
                        GeneratedField::TimeoutPrecommit => {
                            if timeout_precommit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutPrecommit"));
                            }
                            timeout_precommit__ = map_.next_value()?;
                        }
                        GeneratedField::TimeoutPrecommitDelta => {
                            if timeout_precommit_delta__.is_some() {
                                return Err(serde::de::Error::duplicate_field("timeoutPrecommitDelta"));
                            }
                            timeout_precommit_delta__ = map_.next_value()?;
                        }
                        GeneratedField::DelayedPrecommitTimeout => {
                            if delayed_precommit_timeout__.is_some() {
                                return Err(serde::de::Error::duplicate_field("delayedPrecommitTimeout"));
                            }
                            delayed_precommit_timeout__ = map_.next_value()?;
                        }
                    }
                }
                Ok(TimeoutInfo {
                    timeout_propose: timeout_propose__,
                    timeout_commit: timeout_commit__,
                    timeout_propose_delta: timeout_propose_delta__,
                    timeout_prevote: timeout_prevote__,
                    timeout_prevote_delta: timeout_prevote_delta__,
                    timeout_precommit: timeout_precommit__,
                    timeout_precommit_delta: timeout_precommit_delta__,
                    delayed_precommit_timeout: delayed_precommit_timeout__,
                })
            }
        }
        deserializer.deserialize_struct("tendermint.state.TimeoutInfo", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for ValidatorsInfo {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.state.ValidatorsInfo", len)?;
        if let Some(v) = self.validator_set.as_ref() {
            struct_ser.serialize_field("validator_set", v)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("last_height_changed", ToString::to_string(&self.last_height_changed).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for ValidatorsInfo {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "validator_set",
            "validatorSet",
            "last_height_changed",
            "lastHeightChanged",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ValidatorSet,
            LastHeightChanged,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "validatorSet" | "validator_set" => Ok(GeneratedField::ValidatorSet),
                            "lastHeightChanged" | "last_height_changed" => Ok(GeneratedField::LastHeightChanged),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = ValidatorsInfo;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.state.ValidatorsInfo")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<ValidatorsInfo, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut validator_set__ = None;
                let mut last_height_changed__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ValidatorSet => {
                            if validator_set__.is_some() {
                                return Err(serde::de::Error::duplicate_field("validatorSet"));
                            }
                            validator_set__ = map_.next_value()?;
                        }
                        GeneratedField::LastHeightChanged => {
                            if last_height_changed__.is_some() {
                                return Err(serde::de::Error::duplicate_field("lastHeightChanged"));
                            }
                            last_height_changed__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(ValidatorsInfo {
                    validator_set: validator_set__,
                    last_height_changed: last_height_changed__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.state.ValidatorsInfo", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Version {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut len = 0;
        if true {
            len += 1;
        }
        if true {
            len += 1;
        }
        let mut struct_ser = serializer.serialize_struct("tendermint.state.Version", len)?;
        if let Some(v) = self.consensus.as_ref() {
            struct_ser.serialize_field("consensus", v)?;
        }
        if true {
            struct_ser.serialize_field("software", &self.software)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Version {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "consensus",
            "software",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Consensus,
            Software,
        }
        impl<'de> serde::Deserialize<'de> for GeneratedField {
            fn deserialize<D>(deserializer: D) -> std::result::Result<GeneratedField, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct GeneratedVisitor;

                impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
                    type Value = GeneratedField;

                    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                        write!(formatter, "expected one of: {:?}", &FIELDS)
                    }

                    #[allow(unused_variables)]
                    fn visit_str<E>(self, value: &str) -> std::result::Result<GeneratedField, E>
                    where
                        E: serde::de::Error,
                    {
                        match value {
                            "consensus" => Ok(GeneratedField::Consensus),
                            "software" => Ok(GeneratedField::Software),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Version;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct tendermint.state.Version")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Version, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut consensus__ = None;
                let mut software__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Consensus => {
                            if consensus__.is_some() {
                                return Err(serde::de::Error::duplicate_field("consensus"));
                            }
                            consensus__ = map_.next_value()?;
                        }
                        GeneratedField::Software => {
                            if software__.is_some() {
                                return Err(serde::de::Error::duplicate_field("software"));
                            }
                            software__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(Version {
                    consensus: consensus__,
                    software: software__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("tendermint.state.Version", FIELDS, GeneratedVisitor)
    }
}
