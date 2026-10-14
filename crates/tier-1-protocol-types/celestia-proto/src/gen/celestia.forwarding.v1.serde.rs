impl serde::Serialize for EventTokenForwarded {
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
        let mut struct_ser = serializer.serialize_struct("celestia.forwarding.v1.EventTokenForwarded", len)?;
        if true {
            struct_ser.serialize_field("forward_addr", &self.forward_addr)?;
        }
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("denom", &self.denom)?;
        }
        if true {
            struct_ser.serialize_field("amount", &self.amount)?;
        }
        if true {
            struct_ser.serialize_field("message_id", &self.message_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventTokenForwarded {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "forward_addr",
            "forwardAddr",
            "token_id",
            "tokenId",
            "denom",
            "amount",
            "message_id",
            "messageId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ForwardAddr,
            TokenId,
            Denom,
            Amount,
            MessageId,
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
                            "forwardAddr" | "forward_addr" => Ok(GeneratedField::ForwardAddr),
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "denom" => Ok(GeneratedField::Denom),
                            "amount" => Ok(GeneratedField::Amount),
                            "messageId" | "message_id" => Ok(GeneratedField::MessageId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventTokenForwarded;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.forwarding.v1.EventTokenForwarded")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventTokenForwarded, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut forward_addr__ = None;
                let mut token_id__ = None;
                let mut denom__ = None;
                let mut amount__ = None;
                let mut message_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ForwardAddr => {
                            if forward_addr__.is_some() {
                                return Err(serde::de::Error::duplicate_field("forwardAddr"));
                            }
                            forward_addr__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Denom => {
                            if denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("denom"));
                            }
                            denom__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Amount => {
                            if amount__.is_some() {
                                return Err(serde::de::Error::duplicate_field("amount"));
                            }
                            amount__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MessageId => {
                            if message_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageId"));
                            }
                            message_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventTokenForwarded {
                    forward_addr: forward_addr__.unwrap_or_default(),
                    token_id: token_id__.unwrap_or_default(),
                    denom: denom__.unwrap_or_default(),
                    amount: amount__.unwrap_or_default(),
                    message_id: message_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.forwarding.v1.EventTokenForwarded", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for GenesisState {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("celestia.forwarding.v1.GenesisState", len)?;
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
            type Value = GenesisState;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.forwarding.v1.GenesisState")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisState, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(GenesisState {
                })
            }
        }
        deserializer.deserialize_struct("celestia.forwarding.v1.GenesisState", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgForward {
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
        let mut struct_ser = serializer.serialize_struct("celestia.forwarding.v1.MsgForward", len)?;
        if true {
            struct_ser.serialize_field("signer", &self.signer)?;
        }
        if true {
            struct_ser.serialize_field("forward_addr", &self.forward_addr)?;
        }
        if true {
            struct_ser.serialize_field("dest_domain", &self.dest_domain)?;
        }
        if true {
            struct_ser.serialize_field("dest_recipient", &self.dest_recipient)?;
        }
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if let Some(v) = self.max_igp_fee.as_ref() {
            struct_ser.serialize_field("max_igp_fee", v)?;
        }
        if true {
            struct_ser.serialize_field("custom_hook_id", &self.custom_hook_id)?;
        }
        if true {
            struct_ser.serialize_field("custom_hook_metadata", &self.custom_hook_metadata)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgForward {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "signer",
            "forward_addr",
            "forwardAddr",
            "dest_domain",
            "destDomain",
            "dest_recipient",
            "destRecipient",
            "token_id",
            "tokenId",
            "max_igp_fee",
            "maxIgpFee",
            "custom_hook_id",
            "customHookId",
            "custom_hook_metadata",
            "customHookMetadata",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Signer,
            ForwardAddr,
            DestDomain,
            DestRecipient,
            TokenId,
            MaxIgpFee,
            CustomHookId,
            CustomHookMetadata,
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
                            "signer" => Ok(GeneratedField::Signer),
                            "forwardAddr" | "forward_addr" => Ok(GeneratedField::ForwardAddr),
                            "destDomain" | "dest_domain" => Ok(GeneratedField::DestDomain),
                            "destRecipient" | "dest_recipient" => Ok(GeneratedField::DestRecipient),
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "maxIgpFee" | "max_igp_fee" => Ok(GeneratedField::MaxIgpFee),
                            "customHookId" | "custom_hook_id" => Ok(GeneratedField::CustomHookId),
                            "customHookMetadata" | "custom_hook_metadata" => Ok(GeneratedField::CustomHookMetadata),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgForward;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.forwarding.v1.MsgForward")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgForward, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut signer__ = None;
                let mut forward_addr__ = None;
                let mut dest_domain__ = None;
                let mut dest_recipient__ = None;
                let mut token_id__ = None;
                let mut max_igp_fee__ = None;
                let mut custom_hook_id__ = None;
                let mut custom_hook_metadata__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Signer => {
                            if signer__.is_some() {
                                return Err(serde::de::Error::duplicate_field("signer"));
                            }
                            signer__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ForwardAddr => {
                            if forward_addr__.is_some() {
                                return Err(serde::de::Error::duplicate_field("forwardAddr"));
                            }
                            forward_addr__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DestDomain => {
                            if dest_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destDomain"));
                            }
                            dest_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::DestRecipient => {
                            if dest_recipient__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destRecipient"));
                            }
                            dest_recipient__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MaxIgpFee => {
                            if max_igp_fee__.is_some() {
                                return Err(serde::de::Error::duplicate_field("maxIgpFee"));
                            }
                            max_igp_fee__ = map_.next_value()?;
                        }
                        GeneratedField::CustomHookId => {
                            if custom_hook_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("customHookId"));
                            }
                            custom_hook_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::CustomHookMetadata => {
                            if custom_hook_metadata__.is_some() {
                                return Err(serde::de::Error::duplicate_field("customHookMetadata"));
                            }
                            custom_hook_metadata__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgForward {
                    signer: signer__.unwrap_or_default(),
                    forward_addr: forward_addr__.unwrap_or_default(),
                    dest_domain: dest_domain__.unwrap_or_default(),
                    dest_recipient: dest_recipient__.unwrap_or_default(),
                    token_id: token_id__.unwrap_or_default(),
                    max_igp_fee: max_igp_fee__,
                    custom_hook_id: custom_hook_id__.unwrap_or_default(),
                    custom_hook_metadata: custom_hook_metadata__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.forwarding.v1.MsgForward", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgForwardResponse {
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
        let mut struct_ser = serializer.serialize_struct("celestia.forwarding.v1.MsgForwardResponse", len)?;
        if true {
            struct_ser.serialize_field("denom", &self.denom)?;
        }
        if true {
            struct_ser.serialize_field("amount", &self.amount)?;
        }
        if true {
            struct_ser.serialize_field("message_id", &self.message_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgForwardResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "denom",
            "amount",
            "message_id",
            "messageId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Denom,
            Amount,
            MessageId,
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
                            "denom" => Ok(GeneratedField::Denom),
                            "amount" => Ok(GeneratedField::Amount),
                            "messageId" | "message_id" => Ok(GeneratedField::MessageId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgForwardResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.forwarding.v1.MsgForwardResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgForwardResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut denom__ = None;
                let mut amount__ = None;
                let mut message_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Denom => {
                            if denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("denom"));
                            }
                            denom__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Amount => {
                            if amount__.is_some() {
                                return Err(serde::de::Error::duplicate_field("amount"));
                            }
                            amount__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MessageId => {
                            if message_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageId"));
                            }
                            message_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgForwardResponse {
                    denom: denom__.unwrap_or_default(),
                    amount: amount__.unwrap_or_default(),
                    message_id: message_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.forwarding.v1.MsgForwardResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryDeriveForwardingAddressRequest {
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
        let mut struct_ser = serializer.serialize_struct("celestia.forwarding.v1.QueryDeriveForwardingAddressRequest", len)?;
        if true {
            struct_ser.serialize_field("dest_domain", &self.dest_domain)?;
        }
        if true {
            struct_ser.serialize_field("dest_recipient", &self.dest_recipient)?;
        }
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("custom_hook_id", &self.custom_hook_id)?;
        }
        if true {
            struct_ser.serialize_field("custom_hook_metadata", &self.custom_hook_metadata)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryDeriveForwardingAddressRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "dest_domain",
            "destDomain",
            "dest_recipient",
            "destRecipient",
            "token_id",
            "tokenId",
            "custom_hook_id",
            "customHookId",
            "custom_hook_metadata",
            "customHookMetadata",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            DestDomain,
            DestRecipient,
            TokenId,
            CustomHookId,
            CustomHookMetadata,
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
                            "destDomain" | "dest_domain" => Ok(GeneratedField::DestDomain),
                            "destRecipient" | "dest_recipient" => Ok(GeneratedField::DestRecipient),
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "customHookId" | "custom_hook_id" => Ok(GeneratedField::CustomHookId),
                            "customHookMetadata" | "custom_hook_metadata" => Ok(GeneratedField::CustomHookMetadata),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryDeriveForwardingAddressRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.forwarding.v1.QueryDeriveForwardingAddressRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryDeriveForwardingAddressRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut dest_domain__ = None;
                let mut dest_recipient__ = None;
                let mut token_id__ = None;
                let mut custom_hook_id__ = None;
                let mut custom_hook_metadata__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::DestDomain => {
                            if dest_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destDomain"));
                            }
                            dest_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::DestRecipient => {
                            if dest_recipient__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destRecipient"));
                            }
                            dest_recipient__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::CustomHookId => {
                            if custom_hook_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("customHookId"));
                            }
                            custom_hook_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::CustomHookMetadata => {
                            if custom_hook_metadata__.is_some() {
                                return Err(serde::de::Error::duplicate_field("customHookMetadata"));
                            }
                            custom_hook_metadata__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryDeriveForwardingAddressRequest {
                    dest_domain: dest_domain__.unwrap_or_default(),
                    dest_recipient: dest_recipient__.unwrap_or_default(),
                    token_id: token_id__.unwrap_or_default(),
                    custom_hook_id: custom_hook_id__.unwrap_or_default(),
                    custom_hook_metadata: custom_hook_metadata__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.forwarding.v1.QueryDeriveForwardingAddressRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryDeriveForwardingAddressResponse {
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
        let mut struct_ser = serializer.serialize_struct("celestia.forwarding.v1.QueryDeriveForwardingAddressResponse", len)?;
        if true {
            struct_ser.serialize_field("address", &self.address)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryDeriveForwardingAddressResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "address",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Address,
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
                            "address" => Ok(GeneratedField::Address),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryDeriveForwardingAddressResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.forwarding.v1.QueryDeriveForwardingAddressResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryDeriveForwardingAddressResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut address__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Address => {
                            if address__.is_some() {
                                return Err(serde::de::Error::duplicate_field("address"));
                            }
                            address__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryDeriveForwardingAddressResponse {
                    address: address__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.forwarding.v1.QueryDeriveForwardingAddressResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryQuoteForwardingFeeRequest {
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
        let mut struct_ser = serializer.serialize_struct("celestia.forwarding.v1.QueryQuoteForwardingFeeRequest", len)?;
        if true {
            struct_ser.serialize_field("dest_domain", &self.dest_domain)?;
        }
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("custom_hook_id", &self.custom_hook_id)?;
        }
        if true {
            struct_ser.serialize_field("custom_hook_metadata", &self.custom_hook_metadata)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryQuoteForwardingFeeRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "dest_domain",
            "destDomain",
            "token_id",
            "tokenId",
            "custom_hook_id",
            "customHookId",
            "custom_hook_metadata",
            "customHookMetadata",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            DestDomain,
            TokenId,
            CustomHookId,
            CustomHookMetadata,
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
                            "destDomain" | "dest_domain" => Ok(GeneratedField::DestDomain),
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "customHookId" | "custom_hook_id" => Ok(GeneratedField::CustomHookId),
                            "customHookMetadata" | "custom_hook_metadata" => Ok(GeneratedField::CustomHookMetadata),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryQuoteForwardingFeeRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.forwarding.v1.QueryQuoteForwardingFeeRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryQuoteForwardingFeeRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut dest_domain__ = None;
                let mut token_id__ = None;
                let mut custom_hook_id__ = None;
                let mut custom_hook_metadata__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::DestDomain => {
                            if dest_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destDomain"));
                            }
                            dest_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::CustomHookId => {
                            if custom_hook_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("customHookId"));
                            }
                            custom_hook_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::CustomHookMetadata => {
                            if custom_hook_metadata__.is_some() {
                                return Err(serde::de::Error::duplicate_field("customHookMetadata"));
                            }
                            custom_hook_metadata__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(QueryQuoteForwardingFeeRequest {
                    dest_domain: dest_domain__.unwrap_or_default(),
                    token_id: token_id__.unwrap_or_default(),
                    custom_hook_id: custom_hook_id__.unwrap_or_default(),
                    custom_hook_metadata: custom_hook_metadata__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("celestia.forwarding.v1.QueryQuoteForwardingFeeRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryQuoteForwardingFeeResponse {
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
        let mut struct_ser = serializer.serialize_struct("celestia.forwarding.v1.QueryQuoteForwardingFeeResponse", len)?;
        if let Some(v) = self.fee.as_ref() {
            struct_ser.serialize_field("fee", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryQuoteForwardingFeeResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "fee",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Fee,
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
                            "fee" => Ok(GeneratedField::Fee),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryQuoteForwardingFeeResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct celestia.forwarding.v1.QueryQuoteForwardingFeeResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryQuoteForwardingFeeResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut fee__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Fee => {
                            if fee__.is_some() {
                                return Err(serde::de::Error::duplicate_field("fee"));
                            }
                            fee__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryQuoteForwardingFeeResponse {
                    fee: fee__,
                })
            }
        }
        deserializer.deserialize_struct("celestia.forwarding.v1.QueryQuoteForwardingFeeResponse", FIELDS, GeneratedVisitor)
    }
}
