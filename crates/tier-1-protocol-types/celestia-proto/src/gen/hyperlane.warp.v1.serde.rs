impl serde::Serialize for EventCreateCollateralToken {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.EventCreateCollateralToken", len)?;
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("origin_mailbox", &self.origin_mailbox)?;
        }
        if true {
            struct_ser.serialize_field("origin_denom", &self.origin_denom)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateCollateralToken {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token_id",
            "tokenId",
            "owner",
            "origin_mailbox",
            "originMailbox",
            "origin_denom",
            "originDenom",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TokenId,
            Owner,
            OriginMailbox,
            OriginDenom,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "owner" => Ok(GeneratedField::Owner),
                            "originMailbox" | "origin_mailbox" => Ok(GeneratedField::OriginMailbox),
                            "originDenom" | "origin_denom" => Ok(GeneratedField::OriginDenom),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventCreateCollateralToken;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.EventCreateCollateralToken")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateCollateralToken, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token_id__ = None;
                let mut owner__ = None;
                let mut origin_mailbox__ = None;
                let mut origin_denom__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginMailbox => {
                            if origin_mailbox__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originMailbox"));
                            }
                            origin_mailbox__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginDenom => {
                            if origin_denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originDenom"));
                            }
                            origin_denom__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventCreateCollateralToken {
                    token_id: token_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    origin_mailbox: origin_mailbox__.unwrap_or_default(),
                    origin_denom: origin_denom__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.EventCreateCollateralToken", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventCreateSyntheticToken {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.EventCreateSyntheticToken", len)?;
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("origin_mailbox", &self.origin_mailbox)?;
        }
        if true {
            struct_ser.serialize_field("origin_denom", &self.origin_denom)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventCreateSyntheticToken {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token_id",
            "tokenId",
            "owner",
            "origin_mailbox",
            "originMailbox",
            "origin_denom",
            "originDenom",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TokenId,
            Owner,
            OriginMailbox,
            OriginDenom,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "owner" => Ok(GeneratedField::Owner),
                            "originMailbox" | "origin_mailbox" => Ok(GeneratedField::OriginMailbox),
                            "originDenom" | "origin_denom" => Ok(GeneratedField::OriginDenom),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventCreateSyntheticToken;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.EventCreateSyntheticToken")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventCreateSyntheticToken, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token_id__ = None;
                let mut owner__ = None;
                let mut origin_mailbox__ = None;
                let mut origin_denom__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginMailbox => {
                            if origin_mailbox__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originMailbox"));
                            }
                            origin_mailbox__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginDenom => {
                            if origin_denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originDenom"));
                            }
                            origin_denom__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventCreateSyntheticToken {
                    token_id: token_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    origin_mailbox: origin_mailbox__.unwrap_or_default(),
                    origin_denom: origin_denom__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.EventCreateSyntheticToken", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventEnrollRemoteRouter {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.EventEnrollRemoteRouter", len)?;
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("receiver_domain", &self.receiver_domain)?;
        }
        if true {
            struct_ser.serialize_field("receiver_contract", &self.receiver_contract)?;
        }
        if true {
            struct_ser.serialize_field("gas", &self.gas)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventEnrollRemoteRouter {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token_id",
            "tokenId",
            "owner",
            "receiver_domain",
            "receiverDomain",
            "receiver_contract",
            "receiverContract",
            "gas",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TokenId,
            Owner,
            ReceiverDomain,
            ReceiverContract,
            Gas,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "owner" => Ok(GeneratedField::Owner),
                            "receiverDomain" | "receiver_domain" => Ok(GeneratedField::ReceiverDomain),
                            "receiverContract" | "receiver_contract" => Ok(GeneratedField::ReceiverContract),
                            "gas" => Ok(GeneratedField::Gas),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventEnrollRemoteRouter;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.EventEnrollRemoteRouter")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventEnrollRemoteRouter, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token_id__ = None;
                let mut owner__ = None;
                let mut receiver_domain__ = None;
                let mut receiver_contract__ = None;
                let mut gas__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ReceiverDomain => {
                            if receiver_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("receiverDomain"));
                            }
                            receiver_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ReceiverContract => {
                            if receiver_contract__.is_some() {
                                return Err(serde::de::Error::duplicate_field("receiverContract"));
                            }
                            receiver_contract__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Gas => {
                            if gas__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gas"));
                            }
                            gas__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventEnrollRemoteRouter {
                    token_id: token_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    receiver_domain: receiver_domain__.unwrap_or_default(),
                    receiver_contract: receiver_contract__.unwrap_or_default(),
                    gas: gas__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.EventEnrollRemoteRouter", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventReceiveRemoteTransfer {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.EventReceiveRemoteTransfer", len)?;
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("sender", &self.sender)?;
        }
        if true {
            struct_ser.serialize_field("origin_domain", &self.origin_domain)?;
        }
        if true {
            struct_ser.serialize_field("recipient", &self.recipient)?;
        }
        if true {
            struct_ser.serialize_field("amount", &self.amount)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventReceiveRemoteTransfer {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token_id",
            "tokenId",
            "sender",
            "origin_domain",
            "originDomain",
            "recipient",
            "amount",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TokenId,
            Sender,
            OriginDomain,
            Recipient,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "sender" => Ok(GeneratedField::Sender),
                            "originDomain" | "origin_domain" => Ok(GeneratedField::OriginDomain),
                            "recipient" => Ok(GeneratedField::Recipient),
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
            type Value = EventReceiveRemoteTransfer;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.EventReceiveRemoteTransfer")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventReceiveRemoteTransfer, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token_id__ = None;
                let mut sender__ = None;
                let mut origin_domain__ = None;
                let mut recipient__ = None;
                let mut amount__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Sender => {
                            if sender__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sender"));
                            }
                            sender__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginDomain => {
                            if origin_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originDomain"));
                            }
                            origin_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Recipient => {
                            if recipient__.is_some() {
                                return Err(serde::de::Error::duplicate_field("recipient"));
                            }
                            recipient__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Amount => {
                            if amount__.is_some() {
                                return Err(serde::de::Error::duplicate_field("amount"));
                            }
                            amount__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventReceiveRemoteTransfer {
                    token_id: token_id__.unwrap_or_default(),
                    sender: sender__.unwrap_or_default(),
                    origin_domain: origin_domain__.unwrap_or_default(),
                    recipient: recipient__.unwrap_or_default(),
                    amount: amount__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.EventReceiveRemoteTransfer", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventSendRemoteTransfer {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.EventSendRemoteTransfer", len)?;
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("sender", &self.sender)?;
        }
        if true {
            struct_ser.serialize_field("destination_domain", &self.destination_domain)?;
        }
        if true {
            struct_ser.serialize_field("recipient", &self.recipient)?;
        }
        if true {
            struct_ser.serialize_field("amount", &self.amount)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventSendRemoteTransfer {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token_id",
            "tokenId",
            "sender",
            "destination_domain",
            "destinationDomain",
            "recipient",
            "amount",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TokenId,
            Sender,
            DestinationDomain,
            Recipient,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "sender" => Ok(GeneratedField::Sender),
                            "destinationDomain" | "destination_domain" => Ok(GeneratedField::DestinationDomain),
                            "recipient" => Ok(GeneratedField::Recipient),
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
            type Value = EventSendRemoteTransfer;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.EventSendRemoteTransfer")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventSendRemoteTransfer, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token_id__ = None;
                let mut sender__ = None;
                let mut destination_domain__ = None;
                let mut recipient__ = None;
                let mut amount__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Sender => {
                            if sender__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sender"));
                            }
                            sender__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DestinationDomain => {
                            if destination_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destinationDomain"));
                            }
                            destination_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Recipient => {
                            if recipient__.is_some() {
                                return Err(serde::de::Error::duplicate_field("recipient"));
                            }
                            recipient__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Amount => {
                            if amount__.is_some() {
                                return Err(serde::de::Error::duplicate_field("amount"));
                            }
                            amount__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(EventSendRemoteTransfer {
                    token_id: token_id__.unwrap_or_default(),
                    sender: sender__.unwrap_or_default(),
                    destination_domain: destination_domain__.unwrap_or_default(),
                    recipient: recipient__.unwrap_or_default(),
                    amount: amount__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.EventSendRemoteTransfer", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventSetToken {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.EventSetToken", len)?;
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
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
impl<'de> serde::Deserialize<'de> for EventSetToken {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token_id",
            "tokenId",
            "owner",
            "ism_id",
            "ismId",
            "new_owner",
            "newOwner",
            "renounce_ownership",
            "renounceOwnership",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TokenId,
            Owner,
            IsmId,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "owner" => Ok(GeneratedField::Owner),
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
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
            type Value = EventSetToken;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.EventSetToken")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventSetToken, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token_id__ = None;
                let mut owner__ = None;
                let mut ism_id__ = None;
                let mut new_owner__ = None;
                let mut renounce_ownership__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
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
                Ok(EventSetToken {
                    token_id: token_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    ism_id: ism_id__.unwrap_or_default(),
                    new_owner: new_owner__.unwrap_or_default(),
                    renounce_ownership: renounce_ownership__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.EventSetToken", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for EventUnrollRemoteRouter {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.EventUnrollRemoteRouter", len)?;
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("receiver_domain", &self.receiver_domain)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for EventUnrollRemoteRouter {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token_id",
            "tokenId",
            "owner",
            "receiver_domain",
            "receiverDomain",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TokenId,
            Owner,
            ReceiverDomain,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "owner" => Ok(GeneratedField::Owner),
                            "receiverDomain" | "receiver_domain" => Ok(GeneratedField::ReceiverDomain),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = EventUnrollRemoteRouter;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.EventUnrollRemoteRouter")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<EventUnrollRemoteRouter, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token_id__ = None;
                let mut owner__ = None;
                let mut receiver_domain__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ReceiverDomain => {
                            if receiver_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("receiverDomain"));
                            }
                            receiver_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(EventUnrollRemoteRouter {
                    token_id: token_id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    receiver_domain: receiver_domain__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.EventUnrollRemoteRouter", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for GenesisRemoteRouterWrapper {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.GenesisRemoteRouterWrapper", len)?;
        if true {
            #[allow(clippy::needless_borrow)]
            #[allow(clippy::needless_borrows_for_generic_args)]
            struct_ser.serialize_field("token_id", ToString::to_string(&self.token_id).as_str())?;
        }
        if let Some(v) = self.remote_router.as_ref() {
            struct_ser.serialize_field("remote_router", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for GenesisRemoteRouterWrapper {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token_id",
            "tokenId",
            "remote_router",
            "remoteRouter",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            TokenId,
            RemoteRouter,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "remoteRouter" | "remote_router" => Ok(GeneratedField::RemoteRouter),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = GenesisRemoteRouterWrapper;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.GenesisRemoteRouterWrapper")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisRemoteRouterWrapper, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token_id__ = None;
                let mut remote_router__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::RemoteRouter => {
                            if remote_router__.is_some() {
                                return Err(serde::de::Error::duplicate_field("remoteRouter"));
                            }
                            remote_router__ = map_.next_value()?;
                        }
                    }
                }
                Ok(GenesisRemoteRouterWrapper {
                    token_id: token_id__.unwrap_or_default(),
                    remote_router: remote_router__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.GenesisRemoteRouterWrapper", FIELDS, GeneratedVisitor)
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.GenesisState", len)?;
        if let Some(v) = self.params.as_ref() {
            struct_ser.serialize_field("params", v)?;
        }
        if true {
            struct_ser.serialize_field("tokens", &self.tokens)?;
        }
        if true {
            struct_ser.serialize_field("remote_routers", &self.remote_routers)?;
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
            "params",
            "tokens",
            "remote_routers",
            "remoteRouters",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Params,
            Tokens,
            RemoteRouters,
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
                            "params" => Ok(GeneratedField::Params),
                            "tokens" => Ok(GeneratedField::Tokens),
                            "remoteRouters" | "remote_routers" => Ok(GeneratedField::RemoteRouters),
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
                formatter.write_str("struct hyperlane.warp.v1.GenesisState")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<GenesisState, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut params__ = None;
                let mut tokens__ = None;
                let mut remote_routers__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Params => {
                            if params__.is_some() {
                                return Err(serde::de::Error::duplicate_field("params"));
                            }
                            params__ = map_.next_value()?;
                        }
                        GeneratedField::Tokens => {
                            if tokens__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokens"));
                            }
                            tokens__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RemoteRouters => {
                            if remote_routers__.is_some() {
                                return Err(serde::de::Error::duplicate_field("remoteRouters"));
                            }
                            remote_routers__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(GenesisState {
                    params: params__,
                    tokens: tokens__.unwrap_or_default(),
                    remote_routers: remote_routers__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.GenesisState", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for HypToken {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.HypToken", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            let v = HypTokenType::try_from(self.token_type)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.token_type)))?;
            struct_ser.serialize_field("token_type", &v)?;
        }
        if true {
            struct_ser.serialize_field("origin_mailbox", &self.origin_mailbox)?;
        }
        if true {
            struct_ser.serialize_field("origin_denom", &self.origin_denom)?;
        }
        if true {
            struct_ser.serialize_field("collateral_balance", &self.collateral_balance)?;
        }
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for HypToken {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "owner",
            "token_type",
            "tokenType",
            "origin_mailbox",
            "originMailbox",
            "origin_denom",
            "originDenom",
            "collateral_balance",
            "collateralBalance",
            "ism_id",
            "ismId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Owner,
            TokenType,
            OriginMailbox,
            OriginDenom,
            CollateralBalance,
            IsmId,
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
                            "tokenType" | "token_type" => Ok(GeneratedField::TokenType),
                            "originMailbox" | "origin_mailbox" => Ok(GeneratedField::OriginMailbox),
                            "originDenom" | "origin_denom" => Ok(GeneratedField::OriginDenom),
                            "collateralBalance" | "collateral_balance" => Ok(GeneratedField::CollateralBalance),
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = HypToken;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.HypToken")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<HypToken, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut owner__ = None;
                let mut token_type__ = None;
                let mut origin_mailbox__ = None;
                let mut origin_denom__ = None;
                let mut collateral_balance__ = None;
                let mut ism_id__ = None;
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
                        GeneratedField::TokenType => {
                            if token_type__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenType"));
                            }
                            token_type__ = Some(map_.next_value::<HypTokenType>()? as i32);
                        }
                        GeneratedField::OriginMailbox => {
                            if origin_mailbox__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originMailbox"));
                            }
                            origin_mailbox__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginDenom => {
                            if origin_denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originDenom"));
                            }
                            origin_denom__ = Some(map_.next_value()?);
                        }
                        GeneratedField::CollateralBalance => {
                            if collateral_balance__.is_some() {
                                return Err(serde::de::Error::duplicate_field("collateralBalance"));
                            }
                            collateral_balance__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(HypToken {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    token_type: token_type__.unwrap_or_default(),
                    origin_mailbox: origin_mailbox__.unwrap_or_default(),
                    origin_denom: origin_denom__.unwrap_or_default(),
                    collateral_balance: collateral_balance__.unwrap_or_default(),
                    ism_id: ism_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.HypToken", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for HypTokenType {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let variant = match self {
            Self::Unspecified => "HYP_TOKEN_TYPE_UNSPECIFIED",
            Self::Collateral => "HYP_TOKEN_TYPE_COLLATERAL",
            Self::Synthetic => "HYP_TOKEN_TYPE_SYNTHETIC",
        };
        serializer.serialize_str(variant)
    }
}
impl<'de> serde::Deserialize<'de> for HypTokenType {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "HYP_TOKEN_TYPE_UNSPECIFIED",
            "HYP_TOKEN_TYPE_COLLATERAL",
            "HYP_TOKEN_TYPE_SYNTHETIC",
        ];

        struct GeneratedVisitor;

        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = HypTokenType;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(formatter, "expected one of: {:?}", &FIELDS)
            }

            fn visit_i64<E>(self, v: i64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                i32::try_from(v)
                    .ok()
                    .and_then(|x| x.try_into().ok())
                    .ok_or_else(|| {
                        serde::de::Error::invalid_value(serde::de::Unexpected::Signed(v), &self)
                    })
            }

            fn visit_u64<E>(self, v: u64) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                i32::try_from(v)
                    .ok()
                    .and_then(|x| x.try_into().ok())
                    .ok_or_else(|| {
                        serde::de::Error::invalid_value(serde::de::Unexpected::Unsigned(v), &self)
                    })
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                match value {
                    "HYP_TOKEN_TYPE_UNSPECIFIED" => Ok(HypTokenType::Unspecified),
                    "HYP_TOKEN_TYPE_COLLATERAL" => Ok(HypTokenType::Collateral),
                    "HYP_TOKEN_TYPE_SYNTHETIC" => Ok(HypTokenType::Synthetic),
                    _ => Err(serde::de::Error::unknown_variant(value, FIELDS)),
                }
            }
        }
        deserializer.deserialize_any(GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateCollateralToken {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgCreateCollateralToken", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("origin_mailbox", &self.origin_mailbox)?;
        }
        if true {
            struct_ser.serialize_field("origin_denom", &self.origin_denom)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateCollateralToken {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "origin_mailbox",
            "originMailbox",
            "origin_denom",
            "originDenom",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            OriginMailbox,
            OriginDenom,
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
                            "originMailbox" | "origin_mailbox" => Ok(GeneratedField::OriginMailbox),
                            "originDenom" | "origin_denom" => Ok(GeneratedField::OriginDenom),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateCollateralToken;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgCreateCollateralToken")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateCollateralToken, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut origin_mailbox__ = None;
                let mut origin_denom__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginMailbox => {
                            if origin_mailbox__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originMailbox"));
                            }
                            origin_mailbox__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginDenom => {
                            if origin_denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originDenom"));
                            }
                            origin_denom__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateCollateralToken {
                    owner: owner__.unwrap_or_default(),
                    origin_mailbox: origin_mailbox__.unwrap_or_default(),
                    origin_denom: origin_denom__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgCreateCollateralToken", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateCollateralTokenResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgCreateCollateralTokenResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateCollateralTokenResponse {
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
            type Value = MsgCreateCollateralTokenResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgCreateCollateralTokenResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateCollateralTokenResponse, V::Error>
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
                Ok(MsgCreateCollateralTokenResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgCreateCollateralTokenResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateNativeSyntheticToken {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgCreateNativeSyntheticToken", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("origin_mailbox", &self.origin_mailbox)?;
        }
        if true {
            struct_ser.serialize_field("origin_denom", &self.origin_denom)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateNativeSyntheticToken {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "origin_mailbox",
            "originMailbox",
            "origin_denom",
            "originDenom",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            OriginMailbox,
            OriginDenom,
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
                            "originMailbox" | "origin_mailbox" => Ok(GeneratedField::OriginMailbox),
                            "originDenom" | "origin_denom" => Ok(GeneratedField::OriginDenom),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateNativeSyntheticToken;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgCreateNativeSyntheticToken")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateNativeSyntheticToken, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut origin_mailbox__ = None;
                let mut origin_denom__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginMailbox => {
                            if origin_mailbox__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originMailbox"));
                            }
                            origin_mailbox__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginDenom => {
                            if origin_denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originDenom"));
                            }
                            origin_denom__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateNativeSyntheticToken {
                    owner: owner__.unwrap_or_default(),
                    origin_mailbox: origin_mailbox__.unwrap_or_default(),
                    origin_denom: origin_denom__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgCreateNativeSyntheticToken", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateSyntheticToken {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgCreateSyntheticToken", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("origin_mailbox", &self.origin_mailbox)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateSyntheticToken {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "origin_mailbox",
            "originMailbox",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            OriginMailbox,
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
                            "originMailbox" | "origin_mailbox" => Ok(GeneratedField::OriginMailbox),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgCreateSyntheticToken;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgCreateSyntheticToken")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateSyntheticToken, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut origin_mailbox__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginMailbox => {
                            if origin_mailbox__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originMailbox"));
                            }
                            origin_mailbox__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgCreateSyntheticToken {
                    owner: owner__.unwrap_or_default(),
                    origin_mailbox: origin_mailbox__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgCreateSyntheticToken", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgCreateSyntheticTokenResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgCreateSyntheticTokenResponse", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgCreateSyntheticTokenResponse {
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
            type Value = MsgCreateSyntheticTokenResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgCreateSyntheticTokenResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgCreateSyntheticTokenResponse, V::Error>
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
                Ok(MsgCreateSyntheticTokenResponse {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgCreateSyntheticTokenResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgEnrollRemoteRouter {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgEnrollRemoteRouter", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if let Some(v) = self.remote_router.as_ref() {
            struct_ser.serialize_field("remote_router", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgEnrollRemoteRouter {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "token_id",
            "tokenId",
            "remote_router",
            "remoteRouter",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            TokenId,
            RemoteRouter,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "remoteRouter" | "remote_router" => Ok(GeneratedField::RemoteRouter),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgEnrollRemoteRouter;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgEnrollRemoteRouter")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgEnrollRemoteRouter, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut token_id__ = None;
                let mut remote_router__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RemoteRouter => {
                            if remote_router__.is_some() {
                                return Err(serde::de::Error::duplicate_field("remoteRouter"));
                            }
                            remote_router__ = map_.next_value()?;
                        }
                    }
                }
                Ok(MsgEnrollRemoteRouter {
                    owner: owner__.unwrap_or_default(),
                    token_id: token_id__.unwrap_or_default(),
                    remote_router: remote_router__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgEnrollRemoteRouter", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgEnrollRemoteRouterResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgEnrollRemoteRouterResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgEnrollRemoteRouterResponse {
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
            type Value = MsgEnrollRemoteRouterResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgEnrollRemoteRouterResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgEnrollRemoteRouterResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgEnrollRemoteRouterResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgEnrollRemoteRouterResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgRemoteTransfer {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgRemoteTransfer", len)?;
        if true {
            struct_ser.serialize_field("sender", &self.sender)?;
        }
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("destination_domain", &self.destination_domain)?;
        }
        if true {
            struct_ser.serialize_field("recipient", &self.recipient)?;
        }
        if true {
            struct_ser.serialize_field("amount", &self.amount)?;
        }
        if true {
            struct_ser.serialize_field("custom_hook_id", &self.custom_hook_id)?;
        }
        if true {
            struct_ser.serialize_field("gas_limit", &self.gas_limit)?;
        }
        if let Some(v) = self.max_fee.as_ref() {
            struct_ser.serialize_field("max_fee", v)?;
        }
        if true {
            struct_ser.serialize_field("custom_hook_metadata", &self.custom_hook_metadata)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgRemoteTransfer {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "sender",
            "token_id",
            "tokenId",
            "destination_domain",
            "destinationDomain",
            "recipient",
            "amount",
            "custom_hook_id",
            "customHookId",
            "gas_limit",
            "gasLimit",
            "max_fee",
            "maxFee",
            "custom_hook_metadata",
            "customHookMetadata",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Sender,
            TokenId,
            DestinationDomain,
            Recipient,
            Amount,
            CustomHookId,
            GasLimit,
            MaxFee,
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
                            "sender" => Ok(GeneratedField::Sender),
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "destinationDomain" | "destination_domain" => Ok(GeneratedField::DestinationDomain),
                            "recipient" => Ok(GeneratedField::Recipient),
                            "amount" => Ok(GeneratedField::Amount),
                            "customHookId" | "custom_hook_id" => Ok(GeneratedField::CustomHookId),
                            "gasLimit" | "gas_limit" => Ok(GeneratedField::GasLimit),
                            "maxFee" | "max_fee" => Ok(GeneratedField::MaxFee),
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
            type Value = MsgRemoteTransfer;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgRemoteTransfer")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgRemoteTransfer, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut sender__ = None;
                let mut token_id__ = None;
                let mut destination_domain__ = None;
                let mut recipient__ = None;
                let mut amount__ = None;
                let mut custom_hook_id__ = None;
                let mut gas_limit__ = None;
                let mut max_fee__ = None;
                let mut custom_hook_metadata__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Sender => {
                            if sender__.is_some() {
                                return Err(serde::de::Error::duplicate_field("sender"));
                            }
                            sender__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DestinationDomain => {
                            if destination_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destinationDomain"));
                            }
                            destination_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::Recipient => {
                            if recipient__.is_some() {
                                return Err(serde::de::Error::duplicate_field("recipient"));
                            }
                            recipient__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Amount => {
                            if amount__.is_some() {
                                return Err(serde::de::Error::duplicate_field("amount"));
                            }
                            amount__ = Some(map_.next_value()?);
                        }
                        GeneratedField::CustomHookId => {
                            if custom_hook_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("customHookId"));
                            }
                            custom_hook_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::GasLimit => {
                            if gas_limit__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gasLimit"));
                            }
                            gas_limit__ = Some(map_.next_value()?);
                        }
                        GeneratedField::MaxFee => {
                            if max_fee__.is_some() {
                                return Err(serde::de::Error::duplicate_field("maxFee"));
                            }
                            max_fee__ = map_.next_value()?;
                        }
                        GeneratedField::CustomHookMetadata => {
                            if custom_hook_metadata__.is_some() {
                                return Err(serde::de::Error::duplicate_field("customHookMetadata"));
                            }
                            custom_hook_metadata__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgRemoteTransfer {
                    sender: sender__.unwrap_or_default(),
                    token_id: token_id__.unwrap_or_default(),
                    destination_domain: destination_domain__.unwrap_or_default(),
                    recipient: recipient__.unwrap_or_default(),
                    amount: amount__.unwrap_or_default(),
                    custom_hook_id: custom_hook_id__.unwrap_or_default(),
                    gas_limit: gas_limit__.unwrap_or_default(),
                    max_fee: max_fee__,
                    custom_hook_metadata: custom_hook_metadata__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgRemoteTransfer", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgRemoteTransferResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgRemoteTransferResponse", len)?;
        if true {
            struct_ser.serialize_field("message_id", &self.message_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgRemoteTransferResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "message_id",
            "messageId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
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
            type Value = MsgRemoteTransferResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgRemoteTransferResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgRemoteTransferResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut message_id__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::MessageId => {
                            if message_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("messageId"));
                            }
                            message_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgRemoteTransferResponse {
                    message_id: message_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgRemoteTransferResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetToken {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgSetToken", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("new_owner", &self.new_owner)?;
        }
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        if true {
            struct_ser.serialize_field("renounce_ownership", &self.renounce_ownership)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgSetToken {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "token_id",
            "tokenId",
            "new_owner",
            "newOwner",
            "ism_id",
            "ismId",
            "renounce_ownership",
            "renounceOwnership",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            TokenId,
            NewOwner,
            IsmId,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "newOwner" | "new_owner" => Ok(GeneratedField::NewOwner),
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
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
            type Value = MsgSetToken;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgSetToken")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetToken, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut token_id__ = None;
                let mut new_owner__ = None;
                let mut ism_id__ = None;
                let mut renounce_ownership__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::NewOwner => {
                            if new_owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("newOwner"));
                            }
                            new_owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::RenounceOwnership => {
                            if renounce_ownership__.is_some() {
                                return Err(serde::de::Error::duplicate_field("renounceOwnership"));
                            }
                            renounce_ownership__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(MsgSetToken {
                    owner: owner__.unwrap_or_default(),
                    token_id: token_id__.unwrap_or_default(),
                    new_owner: new_owner__.unwrap_or_default(),
                    ism_id: ism_id__.unwrap_or_default(),
                    renounce_ownership: renounce_ownership__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgSetToken", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgSetTokenResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgSetTokenResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgSetTokenResponse {
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
            type Value = MsgSetTokenResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgSetTokenResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgSetTokenResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgSetTokenResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgSetTokenResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgUnrollRemoteRouter {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgUnrollRemoteRouter", len)?;
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            struct_ser.serialize_field("token_id", &self.token_id)?;
        }
        if true {
            struct_ser.serialize_field("receiver_domain", &self.receiver_domain)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgUnrollRemoteRouter {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "owner",
            "token_id",
            "tokenId",
            "receiver_domain",
            "receiverDomain",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Owner,
            TokenId,
            ReceiverDomain,
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
                            "tokenId" | "token_id" => Ok(GeneratedField::TokenId),
                            "receiverDomain" | "receiver_domain" => Ok(GeneratedField::ReceiverDomain),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = MsgUnrollRemoteRouter;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgUnrollRemoteRouter")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgUnrollRemoteRouter, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut owner__ = None;
                let mut token_id__ = None;
                let mut receiver_domain__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Owner => {
                            if owner__.is_some() {
                                return Err(serde::de::Error::duplicate_field("owner"));
                            }
                            owner__ = Some(map_.next_value()?);
                        }
                        GeneratedField::TokenId => {
                            if token_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenId"));
                            }
                            token_id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::ReceiverDomain => {
                            if receiver_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("receiverDomain"));
                            }
                            receiver_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                    }
                }
                Ok(MsgUnrollRemoteRouter {
                    owner: owner__.unwrap_or_default(),
                    token_id: token_id__.unwrap_or_default(),
                    receiver_domain: receiver_domain__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgUnrollRemoteRouter", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for MsgUnrollRemoteRouterResponse {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.warp.v1.MsgUnrollRemoteRouterResponse", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for MsgUnrollRemoteRouterResponse {
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
            type Value = MsgUnrollRemoteRouterResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.MsgUnrollRemoteRouterResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<MsgUnrollRemoteRouterResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(MsgUnrollRemoteRouterResponse {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.MsgUnrollRemoteRouterResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for Params {
    #[allow(deprecated)]
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = 0;
        let struct_ser = serializer.serialize_struct("hyperlane.warp.v1.Params", len)?;
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for Params {
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
            type Value = Params;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.Params")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<Params, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                while map_.next_key::<GeneratedField>()?.is_some() {
                    let _ = map_.next_value::<serde::de::IgnoredAny>()?;
                }
                Ok(Params {
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.Params", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryBridgedSupplyRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryBridgedSupplyRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryBridgedSupplyRequest {
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
            type Value = QueryBridgedSupplyRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryBridgedSupplyRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryBridgedSupplyRequest, V::Error>
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
                Ok(QueryBridgedSupplyRequest {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryBridgedSupplyRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryBridgedSupplyResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryBridgedSupplyResponse", len)?;
        if let Some(v) = self.bridged_supply.as_ref() {
            struct_ser.serialize_field("bridged_supply", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryBridgedSupplyResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "bridged_supply",
            "bridgedSupply",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            BridgedSupply,
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
                            "bridgedSupply" | "bridged_supply" => Ok(GeneratedField::BridgedSupply),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryBridgedSupplyResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryBridgedSupplyResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryBridgedSupplyResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut bridged_supply__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::BridgedSupply => {
                            if bridged_supply__.is_some() {
                                return Err(serde::de::Error::duplicate_field("bridgedSupply"));
                            }
                            bridged_supply__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryBridgedSupplyResponse {
                    bridged_supply: bridged_supply__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryBridgedSupplyResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryQuoteRemoteTransferRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryQuoteRemoteTransferRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("destination_domain", &self.destination_domain)?;
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
impl<'de> serde::Deserialize<'de> for QueryQuoteRemoteTransferRequest {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "destination_domain",
            "destinationDomain",
            "custom_hook_id",
            "customHookId",
            "custom_hook_metadata",
            "customHookMetadata",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            DestinationDomain,
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
                            "id" => Ok(GeneratedField::Id),
                            "destinationDomain" | "destination_domain" => Ok(GeneratedField::DestinationDomain),
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
            type Value = QueryQuoteRemoteTransferRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryQuoteRemoteTransferRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryQuoteRemoteTransferRequest, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut destination_domain__ = None;
                let mut custom_hook_id__ = None;
                let mut custom_hook_metadata__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Id => {
                            if id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("id"));
                            }
                            id__ = Some(map_.next_value()?);
                        }
                        GeneratedField::DestinationDomain => {
                            if destination_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("destinationDomain"));
                            }
                            destination_domain__ = Some(map_.next_value()?);
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
                Ok(QueryQuoteRemoteTransferRequest {
                    id: id__.unwrap_or_default(),
                    destination_domain: destination_domain__.unwrap_or_default(),
                    custom_hook_id: custom_hook_id__.unwrap_or_default(),
                    custom_hook_metadata: custom_hook_metadata__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryQuoteRemoteTransferRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryQuoteRemoteTransferResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryQuoteRemoteTransferResponse", len)?;
        if true {
            struct_ser.serialize_field("gas_payment", &self.gas_payment)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryQuoteRemoteTransferResponse {
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
            type Value = QueryQuoteRemoteTransferResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryQuoteRemoteTransferResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryQuoteRemoteTransferResponse, V::Error>
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
                Ok(QueryQuoteRemoteTransferResponse {
                    gas_payment: gas_payment__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryQuoteRemoteTransferResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRemoteRoutersRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryRemoteRoutersRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRemoteRoutersRequest {
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
            type Value = QueryRemoteRoutersRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryRemoteRoutersRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRemoteRoutersRequest, V::Error>
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
                Ok(QueryRemoteRoutersRequest {
                    id: id__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryRemoteRoutersRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryRemoteRoutersResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryRemoteRoutersResponse", len)?;
        if true {
            struct_ser.serialize_field("remote_routers", &self.remote_routers)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryRemoteRoutersResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "remote_routers",
            "remoteRouters",
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            RemoteRouters,
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
                            "remoteRouters" | "remote_routers" => Ok(GeneratedField::RemoteRouters),
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
            type Value = QueryRemoteRoutersResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryRemoteRoutersResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryRemoteRoutersResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut remote_routers__ = None;
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::RemoteRouters => {
                            if remote_routers__.is_some() {
                                return Err(serde::de::Error::duplicate_field("remoteRouters"));
                            }
                            remote_routers__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryRemoteRoutersResponse {
                    remote_routers: remote_routers__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryRemoteRoutersResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryTokenRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryTokenRequest", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryTokenRequest {
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
            type Value = QueryTokenRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryTokenRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryTokenRequest, V::Error>
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
                Ok(QueryTokenRequest {
                    id: id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryTokenRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryTokenResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryTokenResponse", len)?;
        if let Some(v) = self.token.as_ref() {
            struct_ser.serialize_field("token", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryTokenResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "token",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Token,
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
                            "token" => Ok(GeneratedField::Token),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = QueryTokenResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryTokenResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryTokenResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut token__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Token => {
                            if token__.is_some() {
                                return Err(serde::de::Error::duplicate_field("token"));
                            }
                            token__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryTokenResponse {
                    token: token__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryTokenResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryTokensRequest {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryTokensRequest", len)?;
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryTokensRequest {
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
            type Value = QueryTokensRequest;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryTokensRequest")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryTokensRequest, V::Error>
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
                Ok(QueryTokensRequest {
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryTokensRequest", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for QueryTokensResponse {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.QueryTokensResponse", len)?;
        if true {
            struct_ser.serialize_field("tokens", &self.tokens)?;
        }
        if let Some(v) = self.pagination.as_ref() {
            struct_ser.serialize_field("pagination", v)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for QueryTokensResponse {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "tokens",
            "pagination",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Tokens,
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
                            "tokens" => Ok(GeneratedField::Tokens),
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
            type Value = QueryTokensResponse;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.QueryTokensResponse")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<QueryTokensResponse, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut tokens__ = None;
                let mut pagination__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::Tokens => {
                            if tokens__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokens"));
                            }
                            tokens__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Pagination => {
                            if pagination__.is_some() {
                                return Err(serde::de::Error::duplicate_field("pagination"));
                            }
                            pagination__ = map_.next_value()?;
                        }
                    }
                }
                Ok(QueryTokensResponse {
                    tokens: tokens__.unwrap_or_default(),
                    pagination: pagination__,
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.QueryTokensResponse", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for RemoteRouter {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.RemoteRouter", len)?;
        if true {
            struct_ser.serialize_field("receiver_domain", &self.receiver_domain)?;
        }
        if true {
            struct_ser.serialize_field("receiver_contract", &self.receiver_contract)?;
        }
        if true {
            struct_ser.serialize_field("gas", &self.gas)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for RemoteRouter {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "receiver_domain",
            "receiverDomain",
            "receiver_contract",
            "receiverContract",
            "gas",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            ReceiverDomain,
            ReceiverContract,
            Gas,
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
                            "receiverDomain" | "receiver_domain" => Ok(GeneratedField::ReceiverDomain),
                            "receiverContract" | "receiver_contract" => Ok(GeneratedField::ReceiverContract),
                            "gas" => Ok(GeneratedField::Gas),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = RemoteRouter;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.RemoteRouter")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<RemoteRouter, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut receiver_domain__ = None;
                let mut receiver_contract__ = None;
                let mut gas__ = None;
                while let Some(k) = map_.next_key()? {
                    match k {
                        GeneratedField::ReceiverDomain => {
                            if receiver_domain__.is_some() {
                                return Err(serde::de::Error::duplicate_field("receiverDomain"));
                            }
                            receiver_domain__ = 
                                Some(map_.next_value::<::pbjson::private::NumberDeserialize<_>>()?.0)
                            ;
                        }
                        GeneratedField::ReceiverContract => {
                            if receiver_contract__.is_some() {
                                return Err(serde::de::Error::duplicate_field("receiverContract"));
                            }
                            receiver_contract__ = Some(map_.next_value()?);
                        }
                        GeneratedField::Gas => {
                            if gas__.is_some() {
                                return Err(serde::de::Error::duplicate_field("gas"));
                            }
                            gas__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(RemoteRouter {
                    receiver_domain: receiver_domain__.unwrap_or_default(),
                    receiver_contract: receiver_contract__.unwrap_or_default(),
                    gas: gas__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.RemoteRouter", FIELDS, GeneratedVisitor)
    }
}
impl serde::Serialize for WrappedHypToken {
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
        let mut struct_ser = serializer.serialize_struct("hyperlane.warp.v1.WrappedHypToken", len)?;
        if true {
            struct_ser.serialize_field("id", &self.id)?;
        }
        if true {
            struct_ser.serialize_field("owner", &self.owner)?;
        }
        if true {
            let v = HypTokenType::try_from(self.token_type)
                .map_err(|_| serde::ser::Error::custom(format!("Invalid variant {}", self.token_type)))?;
            struct_ser.serialize_field("token_type", &v)?;
        }
        if true {
            struct_ser.serialize_field("origin_mailbox", &self.origin_mailbox)?;
        }
        if true {
            struct_ser.serialize_field("origin_denom", &self.origin_denom)?;
        }
        if true {
            struct_ser.serialize_field("ism_id", &self.ism_id)?;
        }
        struct_ser.end()
    }
}
impl<'de> serde::Deserialize<'de> for WrappedHypToken {
    #[allow(deprecated)]
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        const FIELDS: &[&str] = &[
            "id",
            "owner",
            "token_type",
            "tokenType",
            "origin_mailbox",
            "originMailbox",
            "origin_denom",
            "originDenom",
            "ism_id",
            "ismId",
        ];

        #[allow(clippy::enum_variant_names)]
        enum GeneratedField {
            Id,
            Owner,
            TokenType,
            OriginMailbox,
            OriginDenom,
            IsmId,
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
                            "tokenType" | "token_type" => Ok(GeneratedField::TokenType),
                            "originMailbox" | "origin_mailbox" => Ok(GeneratedField::OriginMailbox),
                            "originDenom" | "origin_denom" => Ok(GeneratedField::OriginDenom),
                            "ismId" | "ism_id" => Ok(GeneratedField::IsmId),
                            _ => Err(serde::de::Error::unknown_field(value, FIELDS)),
                        }
                    }
                }
                deserializer.deserialize_identifier(GeneratedVisitor)
            }
        }
        struct GeneratedVisitor;
        impl<'de> serde::de::Visitor<'de> for GeneratedVisitor {
            type Value = WrappedHypToken;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("struct hyperlane.warp.v1.WrappedHypToken")
            }

            fn visit_map<V>(self, mut map_: V) -> std::result::Result<WrappedHypToken, V::Error>
                where
                    V: serde::de::MapAccess<'de>,
            {
                let mut id__ = None;
                let mut owner__ = None;
                let mut token_type__ = None;
                let mut origin_mailbox__ = None;
                let mut origin_denom__ = None;
                let mut ism_id__ = None;
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
                        GeneratedField::TokenType => {
                            if token_type__.is_some() {
                                return Err(serde::de::Error::duplicate_field("tokenType"));
                            }
                            token_type__ = Some(map_.next_value::<HypTokenType>()? as i32);
                        }
                        GeneratedField::OriginMailbox => {
                            if origin_mailbox__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originMailbox"));
                            }
                            origin_mailbox__ = Some(map_.next_value()?);
                        }
                        GeneratedField::OriginDenom => {
                            if origin_denom__.is_some() {
                                return Err(serde::de::Error::duplicate_field("originDenom"));
                            }
                            origin_denom__ = Some(map_.next_value()?);
                        }
                        GeneratedField::IsmId => {
                            if ism_id__.is_some() {
                                return Err(serde::de::Error::duplicate_field("ismId"));
                            }
                            ism_id__ = Some(map_.next_value()?);
                        }
                    }
                }
                Ok(WrappedHypToken {
                    id: id__.unwrap_or_default(),
                    owner: owner__.unwrap_or_default(),
                    token_type: token_type__.unwrap_or_default(),
                    origin_mailbox: origin_mailbox__.unwrap_or_default(),
                    origin_denom: origin_denom__.unwrap_or_default(),
                    ism_id: ism_id__.unwrap_or_default(),
                })
            }
        }
        deserializer.deserialize_struct("hyperlane.warp.v1.WrappedHypToken", FIELDS, GeneratedVisitor)
    }
}
