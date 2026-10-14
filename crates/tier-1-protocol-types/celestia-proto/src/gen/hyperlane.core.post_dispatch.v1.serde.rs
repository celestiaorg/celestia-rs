impl serde::Serialize for DestinationGasConfig {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.DestinationGasConfig", len)?;
        if true {
            struct_ser.serialize_field("remote_domain", &self.remote_domain)?;
        }
        if let Some(v) = self.gas_oracle.as_ref() {
            struct_ser.serialize_field("gas_oracle", v)?;
        }
        if true {
            struct_ser.serialize_field("gas_overhead", &self.gas_overhead)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for DestinationGasConfig {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "remote_domain",
            "remoteDomain",
            "gas_oracle",
            "gasOracle",
            "gas_overhead",
            "gasOverhead",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RemoteDomain,
            GasOracle,
            GasOverhead,
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
                            "remoteDomain" | "remote_domain" => Ok(GeneratedField::RemoteDomain),
                            "gasOracle" | "gas_oracle" => Ok(GeneratedField::GasOracle),
                            "gasOverhead" | "gas_overhead" => Ok(GeneratedField::GasOverhead),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = DestinationGasConfig;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.DestinationGasConfig")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<DestinationGasConfig, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut remote_domain__ = None;
                let mut gas_oracle__ = None;
                let mut gas_overhead__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::RemoteDomain => {
                            if remote_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("remoteDomain"));
                            }
                            remote_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::GasOracle => {
                            if gas_oracle__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasOracle"));
                            }
                            gas_oracle__ = map_.next_value()?;
                        }
                        GeneratedField::GasOverhead => {
                            if gas_overhead__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasOverhead"));
                            }
                            gas_overhead__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(DestinationGasConfig {
                    remote_domain: remote_domain__.unwrap_or_default(),
                    gas_oracle: gas_oracle__,
                    gas_overhead: gas_overhead__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.DestinationGasConfig", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventClaimIgp {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.EventClaimIgp", len)?;
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("amount", &self.amount)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventClaimIgp {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "igp_id",
            "igpId",
            "owner",
            "amount",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IgpId,
            Owner,
            Amount,
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
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            "owner" => Ok(GeneratedField::Owner),
                            "amount" => Ok(GeneratedField::Amount),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventClaimIgp;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.EventClaimIgp")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventClaimIgp, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut igp_id__ = None;
                let mut owner__ = None;
                let mut amount__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Amount => {
                            if amount__.is_some() {
                                return Err(serde::de::Error::duplicate_field("amount"));
                            }
                            amount__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventClaimIgp {
                    igp_id: igp_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    amount: amount__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.EventClaimIgp", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventCreateIgp {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.EventCreateIgp", len)?;
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("denom", &self.denom)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateIgp {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "igp_id",
            "igpId",
            "owner",
            "denom",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IgpId,
            Owner,
            Denom,
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
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            "owner" => Ok(GeneratedField::Owner),
                            "denom" => Ok(GeneratedField::Denom),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventCreateIgp;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.EventCreateIgp")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateIgp, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut igp_id__ = None;
                let mut owner__ = None;
                let mut denom__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Denom => {
                            if denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("denom"));
                            }
                            denom__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventCreateIgp {
                    igp_id: igp_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    denom: denom__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.EventCreateIgp", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventCreateMerkleTreeHook {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.EventCreateMerkleTreeHook", len)?;
        if true {
            struct_ser.serialize_field("merkle_tree_hook_id", &self.merkle_tree_hook_id)?;
        }
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateMerkleTreeHook {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "merkle_tree_hook_id",
            "merkleTreeHookId",
            "mailbox_id",
            "mailboxId",
            "owner",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MerkleTreeHookId,
            MailboxId,
            Owner,
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
                            "merkleTreeHookId" | "merkle_tree_hook_id" => Ok(GeneratedField::MerkleTreeHookId),
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "owner" => Ok(GeneratedField::Owner),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventCreateMerkleTreeHook;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.EventCreateMerkleTreeHook")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateMerkleTreeHook, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut merkle_tree_hook_id__ = None;
                let mut mailbox_id__ = None;
                let mut owner__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MerkleTreeHookId => {
                            if merkle_tree_hook_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("merkleTreeHookId"));
                            }
                            merkle_tree_hook_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventCreateMerkleTreeHook {
                    merkle_tree_hook_id: merkle_tree_hook_id__.unwrap_or_default(),
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.EventCreateMerkleTreeHook", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventCreateNoopHook {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.EventCreateNoopHook", len)?;
        if true {
            struct_ser.serialize_field("noop_hook_id", &self.noop_hook_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateNoopHook {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "noop_hook_id",
            "noopHookId",
            "owner",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            NoopHookId,
            Owner,
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
                            "noopHookId" | "noop_hook_id" => Ok(GeneratedField::NoopHookId),
                            "owner" => Ok(GeneratedField::Owner),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventCreateNoopHook;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.EventCreateNoopHook")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateNoopHook, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut noop_hook_id__ = None;
                let mut owner__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::NoopHookId => {
                            if noop_hook_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("noopHookId"));
                            }
                            noop_hook_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventCreateNoopHook {
                    noop_hook_id: noop_hook_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.EventCreateNoopHook", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventGasPayment {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.EventGasPayment", len)?;
        if true {
            struct_ser.serialize_field("message_id", &self.message_id)?;
        }
        if true {
            struct_ser.serialize_field("destination", &self.destination)?;
        }
        if true {
            struct_ser.serialize_field("gas_amount", &self.gas_amount)?;
        }
        if true {
            struct_ser.serialize_field("payment", &self.payment)?;
        }
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventGasPayment {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "message_id",
            "messageId",
            "destination",
            "gas_amount",
            "gasAmount",
            "payment",
            "igp_id",
            "igpId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MessageId,
            Destination,
            GasAmount,
            Payment,
            IgpId,
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
                            "messageId" | "message_id" => Ok(GeneratedField::MessageId),
                            "destination" => Ok(GeneratedField::Destination),
                            "gasAmount" | "gas_amount" => Ok(GeneratedField::GasAmount),
                            "payment" => Ok(GeneratedField::Payment),
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventGasPayment;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.EventGasPayment")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventGasPayment, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut message_id__ = None;
                let mut destination__ = None;
                let mut gas_amount__ = None;
                let mut payment__ = None;
                let mut igp_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MessageId => {
                            if message_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageId"));
                            }
                            message_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Destination => {
                            if destination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destination"));
                            }
                            destination__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::GasAmount => {
                            if gas_amount__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasAmount"));
                            }
                            gas_amount__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Payment => {
                            if payment__.is_some() {
                                return Err(serde::de::Error::duplicate_field("payment"));
                            }
                            payment__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventGasPayment {
                    message_id: message_id__.unwrap_or_default(),
                    destination: destination__.unwrap_or_default(),
                    gas_amount: gas_amount__.unwrap_or_default(),
                    payment: payment__.unwrap_or_default(),
                    igp_id: igp_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.EventGasPayment", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventInsertedIntoTree {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.EventInsertedIntoTree", len)?;
        if true {
            struct_ser.serialize_field("message_id", &self.message_id)?;
        }
        if true {
            struct_ser.serialize_field("index", &self.index)?;
        }
        if true {
            struct_ser.serialize_field("merkle_tree_hook_id", &self.merkle_tree_hook_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventInsertedIntoTree {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "message_id",
            "messageId",
            "index",
            "merkle_tree_hook_id",
            "merkleTreeHookId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MessageId,
            Index,
            MerkleTreeHookId,
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
                            "messageId" | "message_id" => Ok(GeneratedField::MessageId),
                            "index" => Ok(GeneratedField::Index),
                            "merkleTreeHookId" | "merkle_tree_hook_id" => Ok(GeneratedField::MerkleTreeHookId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventInsertedIntoTree;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.EventInsertedIntoTree")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventInsertedIntoTree, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut message_id__ = None;
                let mut index__ = None;
                let mut merkle_tree_hook_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MessageId => {
                            if message_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageId"));
                            }
                            message_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Index => {
                            if index__.is_some() {
                                return Err(serde::de::Error::duplicate_field("index"));
                            }
                            index__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::MerkleTreeHookId => {
                            if merkle_tree_hook_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("merkleTreeHookId"));
                            }
                            merkle_tree_hook_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventInsertedIntoTree {
                    message_id: message_id__.unwrap_or_default(),
                    index: index__.unwrap_or_default(),
                    merkle_tree_hook_id: merkle_tree_hook_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.EventInsertedIntoTree", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventSetDestinationGasConfig {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.EventSetDestinationGasConfig", len)?;
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("remote_domain", &self.remote_domain)?;
        }
        if true {
            struct_ser.serialize_field("gas_overhead", &self.gas_overhead)?;
        }
        if true {
            struct_ser.serialize_field("gas_price", &self.gas_price)?;
        }
        if true {
            struct_ser.serialize_field("token_exchange_rate", &self.token_exchange_rate)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventSetDestinationGasConfig {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "igp_id",
            "igpId",
            "owner",
            "remote_domain",
            "remoteDomain",
            "gas_overhead",
            "gasOverhead",
            "gas_price",
            "gasPrice",
            "token_exchange_rate",
            "tokenExchangeRate",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IgpId,
            Owner,
            RemoteDomain,
            GasOverhead,
            GasPrice,
            TokenExchangeRate,
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
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            "owner" => Ok(GeneratedField::Owner),
                            "remoteDomain" | "remote_domain" => Ok(GeneratedField::RemoteDomain),
                            "gasOverhead" | "gas_overhead" => Ok(GeneratedField::GasOverhead),
                            "gasPrice" | "gas_price" => Ok(GeneratedField::GasPrice),
                            "tokenExchangeRate" | "token_exchange_rate" => Ok(GeneratedField::TokenExchangeRate),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventSetDestinationGasConfig;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.EventSetDestinationGasConfig")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventSetDestinationGasConfig, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut igp_id__ = None;
                let mut owner__ = None;
                let mut remote_domain__ = None;
                let mut gas_overhead__ = None;
                let mut gas_price__ = None;
                let mut token_exchange_rate__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RemoteDomain => {
                            if remote_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("remoteDomain"));
                            }
                            remote_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::GasOverhead => {
                            if gas_overhead__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasOverhead"));
                            }
                            gas_overhead__ = Some(map_.next_value()?);
                        }
                        GeneratedField::GasPrice => {
                            if gas_price__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasPrice"));
                            }
                            gas_price__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TokenExchangeRate => {
                            if token_exchange_rate__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenExchangeRate"));
                            }
                            token_exchange_rate__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventSetDestinationGasConfig {
                    igp_id: igp_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    remote_domain: remote_domain__.unwrap_or_default(),
                    gas_overhead: gas_overhead__.unwrap_or_default(),
                    gas_price: gas_price__.unwrap_or_default(),
                    token_exchange_rate: token_exchange_rate__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.EventSetDestinationGasConfig", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventSetIgp {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.EventSetIgp", len)?;
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("new_owner", &self.new_owner)?;
        }
        if true {
            struct_ser.serialize_field("renounce_ownership", &self.renounce_ownership)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventSetIgp {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "igp_id",
            "igpId",
            "owner",
            "new_owner",
            "newOwner",
            "renounce_ownership",
            "renounceOwnership",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IgpId,
            Owner,
            NewOwner,
            RenounceOwnership,
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
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            "owner" => Ok(GeneratedField::Owner),
                            "newOwner" | "new_owner" => Ok(GeneratedField::NewOwner),
                            "renounceOwnership" | "renounce_ownership" => Ok(GeneratedField::RenounceOwnership),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventSetIgp;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.EventSetIgp")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventSetIgp, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut igp_id__ = None;
                let mut owner__ = None;
                let mut new_owner__ = None;
                let mut renounce_ownership__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::NewOwner => {
                            if new_owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("newOwner"));
                            }
                            new_owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RenounceOwnership => {
                            if renounce_ownership__.is_some() {
                                return Err(serde::de::Error::duplicate_field("renounceOwnership"));
                            }
                            renounce_ownership__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventSetIgp {
                    igp_id: igp_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    new_owner: new_owner__.unwrap_or_default(),
                    renounce_ownership: renounce_ownership__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.EventSetIgp", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for GasOracle {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.GasOracle", len)?;
        if true {
            struct_ser.serialize_field("token_exchange_rate", &self.token_exchange_rate)?;
        }
        if true {
            struct_ser.serialize_field("gas_price", &self.gas_price)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for GasOracle {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token_exchange_rate",
            "tokenExchangeRate",
            "gas_price",
            "gasPrice",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TokenExchangeRate,
            GasPrice,
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
                            "tokenExchangeRate" | "token_exchange_rate" => Ok(GeneratedField::TokenExchangeRate),
                            "gasPrice" | "gas_price" => Ok(GeneratedField::GasPrice),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = GasOracle;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.GasOracle")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GasOracle, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token_exchange_rate__ = None;
                let mut gas_price__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TokenExchangeRate => {
                            if token_exchange_rate__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenExchangeRate"));
                            }
                            token_exchange_rate__ = Some(map_.next_value()?);
                        }
                        GeneratedField::GasPrice => {
                            if gas_price__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasPrice"));
                            }
                            gas_price__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(GasOracle {
                    token_exchange_rate: token_exchange_rate__.unwrap_or_default(),
                    gas_price: gas_price__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.GasOracle", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for GenesisDestinationGasConfigWrapper {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.GenesisDestinationGasConfigWrapper", len)?;
        if true {
            struct_ser.serialize_field("remote_domain", &self.remote_domain)?;
        }
        if let Some(v) = self.gas_oracle.as_ref() {
            struct_ser.serialize_field("gas_oracle", v)?;
        }
        if true {
            struct_ser.serialize_field("gas_overhead", &self.gas_overhead)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("igp_id", ToString::to_string(&self.igp_id).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for GenesisDestinationGasConfigWrapper {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "remote_domain",
            "remoteDomain",
            "gas_oracle",
            "gasOracle",
            "gas_overhead",
            "gasOverhead",
            "igp_id",
            "igpId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RemoteDomain,
            GasOracle,
            GasOverhead,
            IgpId,
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
                            "remoteDomain" | "remote_domain" => Ok(GeneratedField::RemoteDomain),
                            "gasOracle" | "gas_oracle" => Ok(GeneratedField::GasOracle),
                            "gasOverhead" | "gas_overhead" => Ok(GeneratedField::GasOverhead),
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = GenesisDestinationGasConfigWrapper;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.GenesisDestinationGasConfigWrapper")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisDestinationGasConfigWrapper, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut remote_domain__ = None;
                let mut gas_oracle__ = None;
                let mut gas_overhead__ = None;
                let mut igp_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::RemoteDomain => {
                            if remote_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("remoteDomain"));
                            }
                            remote_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::GasOracle => {
                            if gas_oracle__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasOracle"));
                            }
                            gas_oracle__ = map_.next_value()?;
                        }
                        GeneratedField::GasOverhead => {
                            if gas_overhead__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasOverhead"));
                            }
                            gas_overhead__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(GenesisDestinationGasConfigWrapper {
                    remote_domain: remote_domain__.unwrap_or_default(),
                    gas_oracle: gas_oracle__,
                    gas_overhead: gas_overhead__.unwrap_or_default(),
                    igp_id: igp_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.GenesisDestinationGasConfigWrapper", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for GenesisState {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.GenesisState", len)?;
        if true {
            struct_ser.serialize_field("igps", &self.igps)?;
        }
        if true {
            struct_ser.serialize_field("igp_gas_configs", &self.igp_gas_configs)?;
        }
        if true {
            struct_ser.serialize_field("merkle_tree_hooks", &self.merkle_tree_hooks)?;
        }
        if true {
            struct_ser.serialize_field("noop_hooks", &self.noop_hooks)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for GenesisState {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "igps",
            "igp_gas_configs",
            "igpGasConfigs",
            "merkle_tree_hooks",
            "merkleTreeHooks",
            "noop_hooks",
            "noopHooks",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Igps,
            IgpGasConfigs,
            MerkleTreeHooks,
            NoopHooks,
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
                            "igps" => Ok(GeneratedField::Igps),
                            "igpGasConfigs" | "igp_gas_configs" => Ok(GeneratedField::IgpGasConfigs),
                            "merkleTreeHooks" | "merkle_tree_hooks" => Ok(GeneratedField::MerkleTreeHooks),
                            "noopHooks" | "noop_hooks" => Ok(GeneratedField::NoopHooks),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = GenesisState;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.GenesisState")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisState, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut igps__ = None;
                let mut igp_gas_configs__ = None;
                let mut merkle_tree_hooks__ = None;
                let mut noop_hooks__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Igps => {
                            if igps__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igps"));
                            }
                            igps__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IgpGasConfigs => {
                            if igp_gas_configs__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpGasConfigs"));
                            }
                            igp_gas_configs__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MerkleTreeHooks => {
                            if merkle_tree_hooks__.is_some() {
                                return Err(serde::de::Error::duplicate_field("merkleTreeHooks"));
                            }
                            merkle_tree_hooks__ = Some(map_.next_value()?);
                        }
                        GeneratedField::NoopHooks => {
                            if noop_hooks__.is_some() {
                                return Err(serde::de::Error::duplicate_field("noopHooks"));
                            }
                            noop_hooks__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(GenesisState {
                    igps: igps__.unwrap_or_default(),
                    igp_gas_configs: igp_gas_configs__.unwrap_or_default(),
                    merkle_tree_hooks: merkle_tree_hooks__.unwrap_or_default(),
                    noop_hooks: noop_hooks__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.GenesisState", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for InterchainGasPaymaster {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.InterchainGasPaymaster", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("denom", &self.denom)?;
        }
        if true {
            struct_ser.serialize_field("claimable_fees", &self.claimable_fees)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for InterchainGasPaymaster {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "owner",
            "denom",
            "claimable_fees",
            "claimableFees",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Owner,
            Denom,
            ClaimableFees,
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
                            "id" => Ok(GeneratedField::Id),
                            "owner" => Ok(GeneratedField::Owner),
                            "denom" => Ok(GeneratedField::Denom),
                            "claimableFees" | "claimable_fees" => Ok(GeneratedField::ClaimableFees),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = InterchainGasPaymaster;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.InterchainGasPaymaster")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<InterchainGasPaymaster, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut owner__ = None;
                let mut denom__ = None;
                let mut claimable_fees__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Denom => {
                            if denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("denom"));
                            }
                            denom__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ClaimableFees => {
                            if claimable_fees__.is_some() {
                                return Err(serde::de::Error::duplicate_field("claimableFees"));
                            }
                            claimable_fees__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(InterchainGasPaymaster {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    denom: denom__.unwrap_or_default(),
                    claimable_fees: claimable_fees__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.InterchainGasPaymaster", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MerkleTreeHook {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MerkleTreeHook", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if let Some(v) = self.tree.as_ref() {
            struct_ser.serialize_field("tree", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MerkleTreeHook {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "mailbox_id",
            "mailboxId",
            "owner",
            "tree",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            MailboxId,
            Owner,
            Tree,
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
                            "id" => Ok(GeneratedField::Id),
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "owner" => Ok(GeneratedField::Owner),
                            "tree" => Ok(GeneratedField::Tree),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MerkleTreeHook;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MerkleTreeHook")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MerkleTreeHook, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut mailbox_id__ = None;
                let mut owner__ = None;
                let mut tree__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Tree => {
                            if tree__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tree"));
                            }
                            tree__ = map_.next_value()?;
                        }
                    }
                }
                Ok(MerkleTreeHook {
                    id: id__.unwrap_or_default(),
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    tree: tree__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MerkleTreeHook", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgClaim {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgClaim", len)?;
        if true {
            struct_ser.serialize_field("sender", &self.sender)?;
        }
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgClaim {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "sender",
            "igp_id",
            "igpId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Sender,
            IgpId,
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
                            "sender" => Ok(GeneratedField::Sender),
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgClaim;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgClaim")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgClaim, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut sender__ = None;
                let mut igp_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Sender => {
                            if sender__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sender"));
                            }
                            sender__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgClaim {
                    sender: sender__.unwrap_or_default(),
                    igp_id: igp_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgClaim", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgClaimResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgClaimResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgClaimResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
                            Err(serde::de::Error::unknown_field(value, FIELDS))
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgClaimResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgClaimResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgClaimResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgClaimResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgClaimResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateIgp {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateIgp", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("denom", &self.denom)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateIgp {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "denom",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            Denom,
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
                            "owner" => Ok(GeneratedField::Owner),
                            "denom" => Ok(GeneratedField::Denom),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateIgp;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgCreateIgp")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateIgp, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut denom__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Denom => {
                            if denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("denom"));
                            }
                            denom__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateIgp {
                    owner: owner__.unwrap_or_default(),
                    denom: denom__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateIgp", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateIgpResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateIgpResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateIgpResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
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
                            "id" => Ok(GeneratedField::Id),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateIgpResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgCreateIgpResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateIgpResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateIgpResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateIgpResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateMerkleTreeHook {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHook", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateMerkleTreeHook {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "mailbox_id",
            "mailboxId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            MailboxId,
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
                            "owner" => Ok(GeneratedField::Owner),
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateMerkleTreeHook;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHook")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateMerkleTreeHook, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut mailbox_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateMerkleTreeHook {
                    owner: owner__.unwrap_or_default(),
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHook", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateMerkleTreeHookResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHookResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateMerkleTreeHookResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
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
                            "id" => Ok(GeneratedField::Id),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateMerkleTreeHookResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHookResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateMerkleTreeHookResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateMerkleTreeHookResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateMerkleTreeHookResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateNoopHook {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateNoopHook", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateNoopHook {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
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
                            "owner" => Ok(GeneratedField::Owner),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateNoopHook;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgCreateNoopHook")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateNoopHook, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateNoopHook {
                    owner: owner__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateNoopHook", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateNoopHookResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateNoopHookResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateNoopHookResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
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
                            "id" => Ok(GeneratedField::Id),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateNoopHookResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgCreateNoopHookResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateNoopHookResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateNoopHookResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgCreateNoopHookResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgPayForGas {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgPayForGas", len)?;
        if true {
            struct_ser.serialize_field("sender", &self.sender)?;
        }
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        if true {
            struct_ser.serialize_field("message_id", &self.message_id)?;
        }
        if true {
            struct_ser.serialize_field("destination_domain", &self.destination_domain)?;
        }
        if true {
            struct_ser.serialize_field("gas_limit", &self.gas_limit)?;
        }
        if let Some(v) = self.amount.as_ref() {
            struct_ser.serialize_field("amount", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgPayForGas {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "sender",
            "igp_id",
            "igpId",
            "message_id",
            "messageId",
            "destination_domain",
            "destinationDomain",
            "gas_limit",
            "gasLimit",
            "amount",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Sender,
            IgpId,
            MessageId,
            DestinationDomain,
            GasLimit,
            Amount,
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
                            "sender" => Ok(GeneratedField::Sender),
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            "messageId" | "message_id" => Ok(GeneratedField::MessageId),
                            "destinationDomain" | "destination_domain" => Ok(GeneratedField::DestinationDomain),
                            "gasLimit" | "gas_limit" => Ok(GeneratedField::GasLimit),
                            "amount" => Ok(GeneratedField::Amount),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgPayForGas;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgPayForGas")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgPayForGas, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut sender__ = None;
                let mut igp_id__ = None;
                let mut message_id__ = None;
                let mut destination_domain__ = None;
                let mut gas_limit__ = None;
                let mut amount__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Sender => {
                            if sender__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sender"));
                            }
                            sender__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MessageId => {
                            if message_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageId"));
                            }
                            message_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DestinationDomain => {
                            if destination_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destinationDomain"));
                            }
                            destination_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::GasLimit => {
                            if gas_limit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasLimit"));
                            }
                            gas_limit__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Amount => {
                            if amount__.is_some() {
                                return Err(serde::de::Error::duplicate_field("amount"));
                            }
                            amount__ = map_.next_value()?;
                        }
                    }
                }
                Ok(MsgPayForGas {
                    sender: sender__.unwrap_or_default(),
                    igp_id: igp_id__.unwrap_or_default(),
                    message_id: message_id__.unwrap_or_default(),
                    destination_domain: destination_domain__.unwrap_or_default(),
                    gas_limit: gas_limit__.unwrap_or_default(),
                    amount: amount__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgPayForGas", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgPayForGasResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgPayForGasResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgPayForGasResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
                            Err(serde::de::Error::unknown_field(value, FIELDS))
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgPayForGasResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgPayForGasResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgPayForGasResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgPayForGasResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgPayForGasResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetDestinationGasConfig {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        if let Some(v) = self.destination_gas_config.as_ref() {
            struct_ser.serialize_field("destination_gas_config", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgSetDestinationGasConfig {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "igp_id",
            "igpId",
            "destination_gas_config",
            "destinationGasConfig",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            IgpId,
            DestinationGasConfig,
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
                            "owner" => Ok(GeneratedField::Owner),
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            "destinationGasConfig" | "destination_gas_config" => Ok(GeneratedField::DestinationGasConfig),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgSetDestinationGasConfig;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetDestinationGasConfig, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut igp_id__ = None;
                let mut destination_gas_config__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DestinationGasConfig => {
                            if destination_gas_config__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destinationGasConfig"));
                            }
                            destination_gas_config__ = map_.next_value()?;
                        }
                    }
                }
                Ok(MsgSetDestinationGasConfig {
                    owner: owner__.unwrap_or_default(),
                    igp_id: igp_id__.unwrap_or_default(),
                    destination_gas_config: destination_gas_config__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfig", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetDestinationGasConfigResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfigResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgSetDestinationGasConfigResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
                            Err(serde::de::Error::unknown_field(value, FIELDS))
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgSetDestinationGasConfigResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfigResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetDestinationGasConfigResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgSetDestinationGasConfigResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgSetDestinationGasConfigResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetIgpOwner {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgSetIgpOwner", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        if true {
            struct_ser.serialize_field("new_owner", &self.new_owner)?;
        }
        if true {
            struct_ser.serialize_field("renounce_ownership", &self.renounce_ownership)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgSetIgpOwner {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "igp_id",
            "igpId",
            "new_owner",
            "newOwner",
            "renounce_ownership",
            "renounceOwnership",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            IgpId,
            NewOwner,
            RenounceOwnership,
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
                            "owner" => Ok(GeneratedField::Owner),
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            "newOwner" | "new_owner" => Ok(GeneratedField::NewOwner),
                            "renounceOwnership" | "renounce_ownership" => Ok(GeneratedField::RenounceOwnership),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgSetIgpOwner;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgSetIgpOwner")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetIgpOwner, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut igp_id__ = None;
                let mut new_owner__ = None;
                let mut renounce_ownership__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::NewOwner => {
                            if new_owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("newOwner"));
                            }
                            new_owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RenounceOwnership => {
                            if renounce_ownership__.is_some() {
                                return Err(serde::de::Error::duplicate_field("renounceOwnership"));
                            }
                            renounce_ownership__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgSetIgpOwner {
                    owner: owner__.unwrap_or_default(),
                    igp_id: igp_id__.unwrap_or_default(),
                    new_owner: new_owner__.unwrap_or_default(),
                    renounce_ownership: renounce_ownership__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgSetIgpOwner", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetIgpOwnerResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.MsgSetIgpOwnerResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgSetIgpOwnerResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
                            Err(serde::de::Error::unknown_field(value, FIELDS))
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgSetIgpOwnerResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.MsgSetIgpOwnerResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetIgpOwnerResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgSetIgpOwnerResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.MsgSetIgpOwnerResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for NoopHook {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.NoopHook", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for NoopHook {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "owner",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Owner,
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
                            "id" => Ok(GeneratedField::Id),
                            "owner" => Ok(GeneratedField::Owner),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = NoopHook;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.NoopHook")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<NoopHook, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut owner__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(NoopHook {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.NoopHook", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryDestinationGasConfigsRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryDestinationGasConfigsRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryDestinationGasConfigsRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Pagination,
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
                            "id" => Ok(GeneratedField::Id),
                            "pagination" => Ok(GeneratedField::Pagination),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryDestinationGasConfigsRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryDestinationGasConfigsRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryDestinationGasConfigsRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryDestinationGasConfigsRequest {
                    id: id__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryDestinationGasConfigsRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryDestinationGasConfigsResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryDestinationGasConfigsResponse", len)?;
        if true {
            struct_ser.serialize_field("destination_gas_configs", &self.destination_gas_configs)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryDestinationGasConfigsResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "destination_gas_configs",
            "destinationGasConfigs",
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            DestinationGasConfigs,
            Pagination,
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
                            "destinationGasConfigs" | "destination_gas_configs" => Ok(GeneratedField::DestinationGasConfigs),
                            "pagination" => Ok(GeneratedField::Pagination),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryDestinationGasConfigsResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryDestinationGasConfigsResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryDestinationGasConfigsResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut destination_gas_configs__ = None;
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::DestinationGasConfigs => {
                            if destination_gas_configs__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destinationGasConfigs"));
                            }
                            destination_gas_configs__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryDestinationGasConfigsResponse {
                    destination_gas_configs: destination_gas_configs__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryDestinationGasConfigsResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryIgpRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryIgpRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryIgpRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
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
                            "id" => Ok(GeneratedField::Id),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryIgpRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryIgpRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryIgpRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryIgpRequest {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryIgpRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryIgpResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryIgpResponse", len)?;
        if let Some(v) = self.igp.as_ref() {
            struct_ser.serialize_field("igp", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryIgpResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "igp",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Igp,
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
                            "igp" => Ok(GeneratedField::Igp),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryIgpResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryIgpResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryIgpResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut igp__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Igp => {
                            if igp__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igp"));
                            }
                            igp__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryIgpResponse {
                    igp: igp__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryIgpResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryIgpsRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryIgpsRequest", len)?;
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryIgpsRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Pagination,
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
                            "pagination" => Ok(GeneratedField::Pagination),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryIgpsRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryIgpsRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryIgpsRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryIgpsRequest {
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryIgpsRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryIgpsResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryIgpsResponse", len)?;
        if true {
            struct_ser.serialize_field("igps", &self.igps)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryIgpsResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "igps",
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Igps,
            Pagination,
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
                            "igps" => Ok(GeneratedField::Igps),
                            "pagination" => Ok(GeneratedField::Pagination),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryIgpsResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryIgpsResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryIgpsResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut igps__ = None;
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Igps => {
                            if igps__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igps"));
                            }
                            igps__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryIgpsResponse {
                    igps: igps__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryIgpsResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryMerkleTreeHookRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryMerkleTreeHookRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryMerkleTreeHookRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
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
                            "id" => Ok(GeneratedField::Id),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryMerkleTreeHookRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryMerkleTreeHookRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryMerkleTreeHookRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryMerkleTreeHookRequest {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryMerkleTreeHookRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryMerkleTreeHookResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryMerkleTreeHookResponse", len)?;
        if let Some(v) = self.merkle_tree_hook.as_ref() {
            struct_ser.serialize_field("merkle_tree_hook", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryMerkleTreeHookResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "merkle_tree_hook",
            "merkleTreeHook",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MerkleTreeHook,
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
                            "merkleTreeHook" | "merkle_tree_hook" => Ok(GeneratedField::MerkleTreeHook),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryMerkleTreeHookResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryMerkleTreeHookResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryMerkleTreeHookResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut merkle_tree_hook__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MerkleTreeHook => {
                            if merkle_tree_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("merkleTreeHook"));
                            }
                            merkle_tree_hook__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryMerkleTreeHookResponse {
                    merkle_tree_hook: merkle_tree_hook__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryMerkleTreeHookResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryMerkleTreeHooksRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryMerkleTreeHooksRequest", len)?;
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryMerkleTreeHooksRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Pagination,
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
                            "pagination" => Ok(GeneratedField::Pagination),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryMerkleTreeHooksRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryMerkleTreeHooksRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryMerkleTreeHooksRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryMerkleTreeHooksRequest {
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryMerkleTreeHooksRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryMerkleTreeHooksResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryMerkleTreeHooksResponse", len)?;
        if true {
            struct_ser.serialize_field("merkle_tree_hooks", &self.merkle_tree_hooks)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryMerkleTreeHooksResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "merkle_tree_hooks",
            "merkleTreeHooks",
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            MerkleTreeHooks,
            Pagination,
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
                            "merkleTreeHooks" | "merkle_tree_hooks" => Ok(GeneratedField::MerkleTreeHooks),
                            "pagination" => Ok(GeneratedField::Pagination),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryMerkleTreeHooksResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryMerkleTreeHooksResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryMerkleTreeHooksResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut merkle_tree_hooks__ = None;
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MerkleTreeHooks => {
                            if merkle_tree_hooks__.is_some() {
                                return Err(serde::de::Error::duplicate_field("merkleTreeHooks"));
                            }
                            merkle_tree_hooks__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryMerkleTreeHooksResponse {
                    merkle_tree_hooks: merkle_tree_hooks__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryMerkleTreeHooksResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryNoopHookRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryNoopHookRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryNoopHookRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
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
                            "id" => Ok(GeneratedField::Id),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryNoopHookRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryNoopHookRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryNoopHookRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryNoopHookRequest {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryNoopHookRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryNoopHookResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryNoopHookResponse", len)?;
        if let Some(v) = self.noop_hook.as_ref() {
            struct_ser.serialize_field("noop_hook", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryNoopHookResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "noop_hook",
            "noopHook",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            NoopHook,
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
                            "noopHook" | "noop_hook" => Ok(GeneratedField::NoopHook),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryNoopHookResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryNoopHookResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryNoopHookResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut noop_hook__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::NoopHook => {
                            if noop_hook__.is_some() {
                                return Err(serde::de::Error::duplicate_field("noopHook"));
                            }
                            noop_hook__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryNoopHookResponse {
                    noop_hook: noop_hook__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryNoopHookResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryNoopHooksRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryNoopHooksRequest", len)?;
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryNoopHooksRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Pagination,
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
                            "pagination" => Ok(GeneratedField::Pagination),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryNoopHooksRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryNoopHooksRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryNoopHooksRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryNoopHooksRequest {
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryNoopHooksRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryNoopHooksResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryNoopHooksResponse", len)?;
        if true {
            struct_ser.serialize_field("noop_hooks", &self.noop_hooks)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryNoopHooksResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "noop_hooks",
            "noopHooks",
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            NoopHooks,
            Pagination,
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
                            "noopHooks" | "noop_hooks" => Ok(GeneratedField::NoopHooks),
                            "pagination" => Ok(GeneratedField::Pagination),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryNoopHooksResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryNoopHooksResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryNoopHooksResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut noop_hooks__ = None;
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::NoopHooks => {
                            if noop_hooks__.is_some() {
                                return Err(serde::de::Error::duplicate_field("noopHooks"));
                            }
                            noop_hooks__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryNoopHooksResponse {
                    noop_hooks: noop_hooks__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryNoopHooksResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryQuoteGasPaymentRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryQuoteGasPaymentRequest", len)?;
        if true {
            struct_ser.serialize_field("igp_id", &self.igp_id)?;
        }
        if true {
            struct_ser.serialize_field("destination_domain", &self.destination_domain)?;
        }
        if true {
            struct_ser.serialize_field("gas_limit", &self.gas_limit)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryQuoteGasPaymentRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "igp_id",
            "igpId",
            "destination_domain",
            "destinationDomain",
            "gas_limit",
            "gasLimit",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            IgpId,
            DestinationDomain,
            GasLimit,
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
                            "igpId" | "igp_id" => Ok(GeneratedField::IgpId),
                            "destinationDomain" | "destination_domain" => Ok(GeneratedField::DestinationDomain),
                            "gasLimit" | "gas_limit" => Ok(GeneratedField::GasLimit),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryQuoteGasPaymentRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryQuoteGasPaymentRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryQuoteGasPaymentRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut igp_id__ = None;
                let mut destination_domain__ = None;
                let mut gas_limit__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::IgpId => {
                            if igp_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("igpId"));
                            }
                            igp_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DestinationDomain => {
                            if destination_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destinationDomain"));
                            }
                            destination_domain__ = Some(map_.next_value()?);
                        }
                        GeneratedField::GasLimit => {
                            if gas_limit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasLimit"));
                            }
                            gas_limit__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryQuoteGasPaymentRequest {
                    igp_id: igp_id__.unwrap_or_default(),
                    destination_domain: destination_domain__.unwrap_or_default(),
                    gas_limit: gas_limit__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryQuoteGasPaymentRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryQuoteGasPaymentResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.QueryQuoteGasPaymentResponse", len)?;
        if true {
            struct_ser.serialize_field("gas_payment", &self.gas_payment)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryQuoteGasPaymentResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "gas_payment",
            "gasPayment",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            GasPayment,
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
                            "gasPayment" | "gas_payment" => Ok(GeneratedField::GasPayment),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryQuoteGasPaymentResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.QueryQuoteGasPaymentResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryQuoteGasPaymentResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut gas_payment__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::GasPayment => {
                            if gas_payment__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasPayment"));
                            }
                            gas_payment__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryQuoteGasPaymentResponse {
                    gas_payment: gas_payment__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.QueryQuoteGasPaymentResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Tree {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.Tree", len)?;
        if true {
            struct_ser.serialize_field("branch", &self.branch.iter().map(pbjson::private::base64::encode).collect::<Vec<_>>())?;
        }
        if true {
            struct_ser.serialize_field("count", &self.count)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Tree {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "branch",
            "count",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Branch,
            Count,
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
                            "branch" => Ok(GeneratedField::Branch),
                            "count" => Ok(GeneratedField::Count),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = Tree;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.Tree")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Tree, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut branch__ = None;
                let mut count__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Branch => {
                            if branch__.is_some() {
                                return Err(serde::de::Error::duplicate_field("branch"));
                            }
                            branch__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::BytesDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                        GeneratedField::Count => {
                            if count__.is_some() {
                                return Err(serde::de::Error::duplicate_field("count"));
                            }
                            count__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(Tree {
                    branch: branch__.unwrap_or_default(),
                    count: count__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.Tree", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for TreeResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.TreeResponse", len)?;
        if true {
            struct_ser.serialize_field("leafs", &self.leafs.iter().map(pbjson::private::base64::encode).collect::<Vec<_>>())?;
        }
        if true {
            struct_ser.serialize_field("count", &self.count)?;
        }
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("root", pbjson::private::base64::encode(&self.root).as_str())?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for TreeResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "leafs",
            "count",
            "root",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Leafs,
            Count,
            Root,
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
                            "leafs" => Ok(GeneratedField::Leafs),
                            "count" => Ok(GeneratedField::Count),
                            "root" => Ok(GeneratedField::Root),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = TreeResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.TreeResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<TreeResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut leafs__ = None;
                let mut count__ = None;
                let mut root__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Leafs => {
                            if leafs__.is_some() {
                                return Err(serde::de::Error::duplicate_field("leafs"));
                            }
                            leafs__ = 
                                Some(map_.next_value::<Vec<::pbjson::private::BytesDeserialize<_>>>()?
                                    .into_iter().map(|x| x.0).collect())
                            ;
                        }
                        GeneratedField::Count => {
                            if count__.is_some() {
                                return Err(serde::de::Error::duplicate_field("count"));
                            }
                            count__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Root => {
                            if root__.is_some() {
                                return Err(serde::de::Error::duplicate_field("root"));
                            }
                            root__ = 
                                Some(map_.next_value::<::pbjson::private::BytesDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(TreeResponse {
                    leafs: leafs__.unwrap_or_default(),
                    count: count__.unwrap_or_default(),
                    root: root__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.TreeResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for WrappedMerkleTreeHookResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.core.post_dispatch.v1.WrappedMerkleTreeHookResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("mailbox_id", &self.mailbox_id)?;
        }
        if let Some(v) = self.merkle_tree.as_ref() {
            struct_ser.serialize_field("merkle_tree", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for WrappedMerkleTreeHookResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "owner",
            "mailbox_id",
            "mailboxId",
            "merkle_tree",
            "merkleTree",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Owner,
            MailboxId,
            MerkleTree,
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
                            "id" => Ok(GeneratedField::Id),
                            "owner" => Ok(GeneratedField::Owner),
                            "mailboxId" | "mailbox_id" => Ok(GeneratedField::MailboxId),
                            "merkleTree" | "merkle_tree" => Ok(GeneratedField::MerkleTree),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = WrappedMerkleTreeHookResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.core.post_dispatch.v1.WrappedMerkleTreeHookResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<WrappedMerkleTreeHookResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut owner__ = None;
                let mut mailbox_id__ = None;
                let mut merkle_tree__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MailboxId => {
                            if mailbox_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("mailboxId"));
                            }
                            mailbox_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MerkleTree => {
                            if merkle_tree__.is_some() {
                                return Err(serde::de::Error::duplicate_field("merkleTree"));
                            }
                            merkle_tree__ = map_.next_value()?;
                        }
                    }
                }
                Ok(WrappedMerkleTreeHookResponse {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    mailbox_id: mailbox_id__.unwrap_or_default(),
                    merkle_tree: merkle_tree__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.core.post_dispatch.v1.WrappedMerkleTreeHookResponse", FIELDS, GeneratedVisitor)
    }
}
